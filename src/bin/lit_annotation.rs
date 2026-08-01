//! CLI for the Lit annotation DSL.
//!
//! Reads a Lit document (or bare annotation text) from stdin or file arguments
//! and emits the parsed annotation AST as a JSON array on stdout.

use lit_annotation_core::block::{is_block_form, parse_block};
use lit_annotation_core::compact::parse_compact;
use lit_annotation_core::marks::{builtin_config, builtin_mark_codes, sorted_mark_codes, MarkConfig};
use lit_annotation_core::parser::parse_annotations;
use lit_annotation_core::scanner::utf16_len;
use lit_annotation_core::types::Annotation;
use std::env;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Parsed command-line options for a normal run.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    pretty: bool,
    strict: bool,
    marks: Option<PathBuf>,
    files: Vec<PathBuf>,
}

/// Top-level command distinguished by `parse_args`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Cmd {
    Run(Options),
    Help,
    Version,
}

fn usage() -> String {
    "Usage: lit-annotation [OPTIONS] [FILE]...\n\
     \n\
     Parse Lit annotation comments and emit JSON on stdout.\n\
     \n\
     Options:\n\
       --pretty         Pretty-print JSON (2-space indent)\n\
       --strict         Exit 2 if any annotation is unstructured\n\
       --marks <path>   Load mark codes from a TOML file (overlay on builtins)\n\
       -h, --help       Print help\n\
       --version        Print version\n\
     \n\
     With no FILE args, read stdin. Multiple files yield one combined JSON array\n\
     in file order. Exit codes: 0 success, 1 I/O or usage error, 2 strict violation.\n"
        .to_string()
}

/// Parse CLI arguments (excluding argv[0]).
fn parse_args(args: &[String]) -> Result<Cmd, String> {
    let mut pretty = false;
    let mut strict = false;
    let mut marks: Option<PathBuf> = None;
    let mut files: Vec<PathBuf> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-h" | "--help" => return Ok(Cmd::Help),
            "--version" => return Ok(Cmd::Version),
            "--pretty" => pretty = true,
            "--strict" => strict = true,
            "--marks" => {
                i += 1;
                match args.get(i) {
                    Some(path) if !path.starts_with('-') || path == "-" => {
                        marks = Some(PathBuf::from(path));
                    }
                    _ => return Err("missing value for --marks".to_string()),
                }
            }
            s if s.starts_with("--marks=") => {
                let path = &s["--marks=".len()..];
                if path.is_empty() {
                    return Err("missing value for --marks".to_string());
                }
                marks = Some(PathBuf::from(path));
            }
            s if s.starts_with('-') => {
                return Err(format!("unknown flag: {s}"));
            }
            _ => files.push(PathBuf::from(arg)),
        }
        i += 1;
    }

    Ok(Cmd::Run(Options {
        pretty,
        strict,
        marks,
        files,
    }))
}

/// Load mark codes: builtin fast path when `marks` is `None`, otherwise parse
/// the TOML file and overlay it on the builtin config (file wins per code).
fn load_mark_codes(marks: Option<&Path>) -> Result<Vec<String>, String> {
    match marks {
        None => Ok(builtin_mark_codes().to_vec()),
        Some(path) => {
            let content = std::fs::read_to_string(path)
                .map_err(|e| format!("failed to read marks file {}: {e}", path.display()))?;
            let overrides: MarkConfig = toml::from_str(&content)
                .map_err(|e| format!("invalid marks TOML {}: {e}", path.display()))?;
            let mut merged = builtin_config().clone();
            for (code, def) in overrides.0 {
                merged.0.insert(code, def);
            }
            Ok(sorted_mark_codes(&merged))
        }
    }
}

/// Parse one input blob into annotations.
///
/// - Content containing `<!---` is run through `parse_annotations`.
/// - Otherwise the trimmed input is treated as bare annotation text and
///   dispatched via `is_block_form` to the block or compact parser.
/// - Whitespace-only bare input yields `[]`.
fn parse_input(content: &str, codes: &[String]) -> Vec<Annotation> {
    if content.contains("<!---") {
        return parse_annotations(content, codes);
    }

    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let mut ann = if is_block_form(trimmed) {
        parse_block(trimmed, codes)
    } else {
        parse_compact(trimmed, codes)
    };
    ann.char_start = 0;
    ann.char_end = utf16_len(trimmed);
    ann.original = trimmed.to_string();
    vec![ann]
}

fn read_inputs(files: &[PathBuf]) -> Result<Vec<String>, String> {
    if files.is_empty() {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("failed to read stdin: {e}"))?;
        return Ok(vec![buf]);
    }

    let mut contents = Vec::with_capacity(files.len());
    for path in files {
        let s = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        contents.push(s);
    }
    Ok(contents)
}

fn run(opts: Options) -> Result<i32, String> {
    let codes = load_mark_codes(opts.marks.as_deref())?;
    let inputs = read_inputs(&opts.files)?;

    let mut annotations = Vec::new();
    for content in &inputs {
        annotations.extend(parse_input(content, &codes));
    }

    let json = if opts.pretty {
        serde_json::to_string_pretty(&annotations)
    } else {
        serde_json::to_string(&annotations)
    }
    .map_err(|e| format!("failed to serialize JSON: {e}"))?;

    let mut stdout = io::stdout().lock();
    stdout
        .write_all(json.as_bytes())
        .and_then(|_| stdout.write_all(b"\n"))
        .map_err(|e| format!("failed to write stdout: {e}"))?;

    if opts.strict && annotations.iter().any(|a| !a.is_structured) {
        return Ok(2);
    }
    Ok(0)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match parse_args(&args) {
        Ok(Cmd::Help) => {
            print!("{}", usage());
            ExitCode::SUCCESS
        }
        Ok(Cmd::Version) => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Cmd::Run(opts)) => match run(opts) {
            Ok(0) => ExitCode::SUCCESS,
            Ok(2) => ExitCode::from(2),
            Ok(code) => ExitCode::from(code as u8),
            Err(msg) => {
                eprintln!("error: {msg}");
                eprintln!("{}", usage());
                ExitCode::from(1)
            }
        },
        Err(msg) => {
            eprintln!("error: {msg}");
            eprintln!("{}", usage());
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lit_annotation_core::types::{
        AnnotationForm, AnnotationType, Certainty, Scope, ScopeKind,
    };
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    // --- parse_args -------------------------------------------------------

    #[test]
    fn parse_args_defaults() {
        let cmd = parse_args(&s(&[])).unwrap();
        match cmd {
            Cmd::Run(opts) => {
                assert!(!opts.pretty);
                assert!(!opts.strict);
                assert!(opts.marks.is_none());
                assert!(opts.files.is_empty());
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_args_pretty() {
        let cmd = parse_args(&s(&["--pretty"])).unwrap();
        match cmd {
            Cmd::Run(opts) => assert!(opts.pretty),
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_args_strict() {
        let cmd = parse_args(&s(&["--strict"])).unwrap();
        match cmd {
            Cmd::Run(opts) => assert!(opts.strict),
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_args_marks() {
        let cmd = parse_args(&s(&["--marks", "path/to/marks.toml"])).unwrap();
        match cmd {
            Cmd::Run(opts) => {
                assert_eq!(opts.marks.as_deref(), Some(Path::new("path/to/marks.toml")));
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_args_marks_equals() {
        let cmd = parse_args(&s(&["--marks=foo.toml"])).unwrap();
        match cmd {
            Cmd::Run(opts) => {
                assert_eq!(opts.marks.as_deref(), Some(Path::new("foo.toml")));
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_args_file_args() {
        let cmd = parse_args(&s(&["a.md", "b.md"])).unwrap();
        match cmd {
            Cmd::Run(opts) => {
                assert_eq!(
                    opts.files,
                    vec![PathBuf::from("a.md"), PathBuf::from("b.md")]
                );
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_args_combinations() {
        let cmd = parse_args(&s(&[
            "--pretty",
            "--strict",
            "--marks",
            "m.toml",
            "doc.md",
        ]))
        .unwrap();
        match cmd {
            Cmd::Run(opts) => {
                assert!(opts.pretty);
                assert!(opts.strict);
                assert_eq!(opts.marks.as_deref(), Some(Path::new("m.toml")));
                assert_eq!(opts.files, vec![PathBuf::from("doc.md")]);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_args_help() {
        assert_eq!(parse_args(&s(&["--help"])).unwrap(), Cmd::Help);
        assert_eq!(parse_args(&s(&["-h"])).unwrap(), Cmd::Help);
    }

    #[test]
    fn parse_args_version() {
        assert_eq!(parse_args(&s(&["--version"])).unwrap(), Cmd::Version);
    }

    #[test]
    fn parse_args_unknown_flag() {
        let err = parse_args(&s(&["--nope"])).unwrap_err();
        assert!(err.contains("unknown flag"), "err={err}");
    }

    #[test]
    fn parse_args_marks_missing_value() {
        let err = parse_args(&s(&["--marks"])).unwrap_err();
        assert!(err.contains("missing value"), "err={err}");
    }

    // --- parse_input: fenced path -----------------------------------------

    #[test]
    fn parse_input_fenced_compact() {
        let content = r#"<!--- n? ^"dharma" | Possibly a technical term here. --->"#;
        let codes = builtin_mark_codes();
        let anns = parse_input(content, codes);
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].form, AnnotationForm::Compact);
        assert_eq!(anns[0].annotation_type, AnnotationType::Note);
        assert_eq!(anns[0].certainty, Certainty::Tentative);
        assert_eq!(anns[0].scope, Scope::Anchor("dharma".to_string()));
        assert_eq!(
            anns[0].body,
            Some("Possibly a technical term here.".to_string())
        );
        assert!(anns[0].is_structured);
    }

    #[test]
    fn parse_input_skips_fenced_code_blocks() {
        let content = "```\n<!--- skip me --->\n```\n<!--- q? | keep --->";
        let anns = parse_input(content, builtin_mark_codes());
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].annotation_type, AnnotationType::Question);
        assert_eq!(anns[0].body, Some("keep".to_string()));
    }

    // --- parse_input: bare path -------------------------------------------

    #[test]
    fn parse_input_bare_block() {
        let content = r#"
n
^"viracitaḥ"
---
Past participle of vi + √rac ("to arrange, compose") - "composed by, authored by."
"#;
        let anns = parse_input(content, builtin_mark_codes());
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].form, AnnotationForm::Block);
        assert_eq!(anns[0].annotation_type, AnnotationType::Note);
        assert_eq!(anns[0].scope, Scope::Anchor("viracitaḥ".to_string()));
        assert_eq!(anns[0].char_start, 0);
        let trimmed = content.trim();
        assert_eq!(anns[0].char_end, utf16_len(trimmed));
        assert_eq!(anns[0].original, trimmed);
    }

    #[test]
    fn parse_input_bare_compact() {
        let content = "  n: | quick note  \n";
        let anns = parse_input(content, builtin_mark_codes());
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].form, AnnotationForm::Compact);
        assert_eq!(anns[0].annotation_type, AnnotationType::Note);
        assert_eq!(anns[0].body, Some("quick note".to_string()));
        assert_eq!(anns[0].char_start, 0);
        assert_eq!(anns[0].char_end, utf16_len("n: | quick note"));
        assert_eq!(anns[0].original, "n: | quick note");
    }

    #[test]
    fn parse_input_whitespace_only_yields_empty() {
        assert!(parse_input("   \n\t  \n", builtin_mark_codes()).is_empty());
        assert!(parse_input("", builtin_mark_codes()).is_empty());
    }

    // --- parse_input: scope coverage --------------------------------------

    #[test]
    fn parse_input_scope_tokens() {
        // (scope token embedded in compact annotation, expected Scope)
        let cases: Vec<(&str, Scope)> = vec![
            ("n: | body", Scope::Sentence(1)), // default / omitted
            ("n _ | body", Scope::Words(1)),
            ("n ___ | body", Scope::Words(3)),
            (r"n \s | body", Scope::Sentence(1)),
            (r"n \ss | body", Scope::Sentence(2)),
            (r"n \s__ | body", Scope::Sentence(2)),
            (r"n \p | body", Scope::Paragraph(1)),
            (r"n \pp | body", Scope::Paragraph(2)),
            (r"n \p__ | body", Scope::Paragraph(2)),
            (r"n \f | body", Scope::Page(1)),
            (r"n \ff | body", Scope::Page(2)),
            (r"n \h | body", Scope::Section),
            (r"n \d | body", Scope::Document),
            (r#"n ^"anuttara" | body"#, Scope::Anchor("anuttara".to_string())),
            (
                r#"n ^"he said \"hi\"" | body"#,
                Scope::Anchor(r#"he said "hi""#.to_string()),
            ),
            (
                "n 3_1 | body",
                Scope::Asymmetric {
                    unit: ScopeKind::Word,
                    before: 3,
                    after: 1,
                },
            ),
            (
                r"n 2\p1 | body",
                Scope::Asymmetric {
                    unit: ScopeKind::Paragraph,
                    before: 2,
                    after: 1,
                },
            ),
            (
                r"n 0\s2 | body",
                Scope::Asymmetric {
                    unit: ScopeKind::Sentence,
                    before: 0,
                    after: 2,
                },
            ),
            (
                r"n 2\f0 | body",
                Scope::Asymmetric {
                    unit: ScopeKind::Page,
                    before: 2,
                    after: 0,
                },
            ),
        ];

        let codes = builtin_mark_codes();
        for (input, expected) in cases {
            let anns = parse_input(input, codes);
            assert_eq!(anns.len(), 1, "input={input:?}");
            assert_eq!(anns[0].scope, expected, "input={input:?}");
        }
    }

    // --- load_mark_codes --------------------------------------------------

    #[test]
    fn load_mark_codes_none_returns_builtin() {
        let codes = load_mark_codes(None).unwrap();
        assert_eq!(codes, builtin_mark_codes());
    }

    #[test]
    fn load_mark_codes_custom_overlay() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(
            tmp,
            r#"[zz]
label = "Custom ZZ"
"#
        )
        .unwrap();
        let codes = load_mark_codes(Some(tmp.path())).unwrap();
        assert!(codes.contains(&"zz".to_string()), "codes={codes:?}");
        // Builtin codes still present
        assert!(codes.iter().any(|c| c == "nb"), "codes={codes:?}");
        // Longest-first ordering
        for w in codes.windows(2) {
            assert!(
                w[0].len() >= w[1].len(),
                "not longest-first: {} then {}",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn load_mark_codes_nonexistent_path() {
        let err = load_mark_codes(Some(Path::new("/nonexistent/marks.toml"))).unwrap_err();
        assert!(err.contains("failed to read") || err.contains("No such"), "err={err}");
    }

    #[test]
    fn load_mark_codes_invalid_toml() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "not = [valid toml").unwrap();
        let err = load_mark_codes(Some(tmp.path())).unwrap_err();
        assert!(err.contains("invalid marks TOML"), "err={err}");
    }
}
