//! CLI for the Lit annotation DSL.
//!
//! Reads a Lit document from stdin or file arguments and emits the parsed
//! annotation AST as a JSON array on stdout. Default is document mode (scan for
//! `<!--- ... --->` and legacy `%%! ... %%` fences). Pass `--bare` to treat the
//! input as a single fence-free annotation body.

use lit_annotation_core::block::{is_block_form, parse_block};
use lit_annotation_core::compact::parse_compact;
use lit_annotation_core::marks::{
    builtin_mark_codes, overlay_on_builtins, sorted_mark_codes, MarkConfig,
};
use lit_annotation_core::parser::parse_annotations;
use lit_annotation_core::scanner::utf16_len;
use lit_annotation_core::types::{Annotation, AnnotationType};
use serde::Serialize;
use std::env;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// CLI output wrapper: library `Annotation` plus optional source-file attribution.
#[derive(Debug, Serialize)]
struct OutputAnnotation {
    #[serde(flatten)]
    annotation: Annotation,
    /// Path as given on the CLI, or `null` for stdin.
    file: Option<String>,
}

/// A positional input: a filesystem path, or `-` meaning stdin.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Input {
    Stdin,
    Path(PathBuf),
}

/// Parsed command-line options for a normal run.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    pretty: bool,
    strict: bool,
    /// Opt into fence-free single-annotation parsing. Default is document mode.
    bare: bool,
    marks: Option<PathBuf>,
    inputs: Vec<Input>,
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
       --strict         Exit 2 if any annotation is unstructured or untyped (bare)\n\
       --bare           Treat input as a single fence-free annotation\n\
       --marks <path>   Load mark codes from a TOML file (overlay on builtins)\n\
       --               End of options; remaining args are file paths\n\
       -h, --help       Print help\n\
       --version        Print version\n\
     \n\
     Default is document mode: scan for <!--- ... ---> (and legacy %%! ... %%)\n\
     fences. Use --bare for a single fence-free annotation body.\n\
     With no FILE args, read stdin. `-` also reads stdin (at most once).\n\
     Multiple inputs yield one combined JSON array in arg order (each\n\
     annotation carries a `file` field; null for stdin). Exit codes: 0\n\
     success, 1 I/O or usage error, 2 strict violation.\n"
        .to_string()
}

/// Push a positional input, mapping `-` to stdin and rejecting a second `-`.
fn push_input(inputs: &mut Vec<Input>, arg: &str) -> Result<(), String> {
    if arg == "-" {
        if inputs.iter().any(|i| matches!(i, Input::Stdin)) {
            return Err("stdin (-) may be given at most once".to_string());
        }
        inputs.push(Input::Stdin);
    } else {
        inputs.push(Input::Path(PathBuf::from(arg)));
    }
    Ok(())
}

/// Parse CLI arguments (excluding argv[0]).
fn parse_args(args: &[String]) -> Result<Cmd, String> {
    let mut pretty = false;
    let mut strict = false;
    let mut bare = false;
    let mut marks: Option<PathBuf> = None;
    let mut inputs: Vec<Input> = Vec::new();

    let mut i = 0;
    let mut positional_only = false;
    while i < args.len() {
        let arg = &args[i];
        if positional_only {
            push_input(&mut inputs, arg)?;
            i += 1;
            continue;
        }
        match arg.as_str() {
            "-h" | "--help" => return Ok(Cmd::Help),
            "--version" => return Ok(Cmd::Version),
            "--pretty" => pretty = true,
            "--strict" => strict = true,
            "--bare" => bare = true,
            "--" => {
                // End of options: remaining args are positional inputs.
                positional_only = true;
            }
            "--marks" => {
                i += 1;
                match args.get(i).map(String::as_str) {
                    None => return Err("missing value for --marks".to_string()),
                    // Next arg looks like a flag: clearer than "invalid marks path".
                    Some(next) if next.starts_with('-') => {
                        return Err(format!(
                            "missing value for --marks (next argument '{next}' looks like a flag)"
                        ));
                    }
                    Some(path) => marks = Some(parse_marks_value(path)?),
                }
            }
            s if s.starts_with("--marks=") => {
                let path = &s["--marks=".len()..];
                if path.is_empty() {
                    return Err("missing value for --marks".to_string());
                }
                marks = Some(parse_marks_value(path)?);
            }
            "-" => push_input(&mut inputs, "-")?,
            s if s.starts_with('-') => {
                return Err(format!("unknown flag: {s}"));
            }
            other => push_input(&mut inputs, other)?,
        }
        i += 1;
    }

    Ok(Cmd::Run(Options {
        pretty,
        strict,
        bare,
        marks,
        inputs,
    }))
}

/// Validate a `--marks` value. Rejects stdin (`-`) and dash-leading paths so
/// both `--marks -foo` and `--marks=-foo` share one clear error.
fn parse_marks_value(path: &str) -> Result<PathBuf, String> {
    if path == "-" {
        return Err("marks cannot be read from stdin".to_string());
    }
    if path.starts_with('-') {
        return Err(format!(
            "invalid marks path '{path}': value must not start with '-'"
        ));
    }
    if path.is_empty() {
        return Err("missing value for --marks".to_string());
    }
    Ok(PathBuf::from(path))
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
            let merged = overlay_on_builtins(overrides);
            Ok(sorted_mark_codes(&merged))
        }
    }
}

/// Document mode: scan the full input for annotation fences
/// (`<!--- ... --->` and legacy `%%! ... %%`).
fn parse_document(content: &str, codes: &[String]) -> Vec<Annotation> {
    parse_annotations(content, codes)
}

/// Bare mode: treat the trimmed input as a single fence-free annotation body.
/// Whitespace-only input yields `[]`.
fn parse_bare(content: &str, codes: &[String]) -> Vec<Annotation> {
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

/// Route one input blob: document mode by default, bare mode only when flagged.
fn parse_input(content: &str, codes: &[String], bare: bool) -> Vec<Annotation> {
    if bare {
        parse_bare(content, codes)
    } else {
        parse_document(content, codes)
    }
}

/// Read stdin once into a string.
fn read_stdin() -> Result<String, String> {
    let mut buf = String::new();
    io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("failed to read stdin: {e}"))?;
    Ok(buf)
}

/// Read inputs paired with an optional file path (None for stdin).
/// Empty `inputs` means implicit stdin (no FILE args).
fn read_inputs(inputs: &[Input]) -> Result<Vec<(Option<String>, String)>, String> {
    if inputs.is_empty() {
        return Ok(vec![(None, read_stdin()?)]);
    }

    let mut contents = Vec::with_capacity(inputs.len());
    for input in inputs {
        match input {
            Input::Stdin => contents.push((None, read_stdin()?)),
            Input::Path(path) => {
                let s = std::fs::read_to_string(path)
                    .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
                contents.push((Some(path.display().to_string()), s));
            }
        }
    }
    Ok(contents)
}

/// Exit status distinguished from I/O/usage errors (`Err`).
enum RunStatus {
    Success,
    StrictViolation,
}

fn run(opts: Options) -> Result<RunStatus, String> {
    let codes = load_mark_codes(opts.marks.as_deref())?;
    let inputs = read_inputs(&opts.inputs)?;

    let mut output: Vec<OutputAnnotation> = Vec::new();
    for (file, content) in &inputs {
        for annotation in parse_input(content, &codes, opts.bare) {
            output.push(OutputAnnotation {
                annotation,
                file: file.clone(),
            });
        }
    }

    let json = if opts.pretty {
        serde_json::to_string_pretty(&output)
    } else {
        serde_json::to_string(&output)
    }
    .map_err(|e| format!("failed to serialize JSON: {e}"))?;

    // Compute strict offenders before writing stdout so a broken pipe cannot
    // skip the exit-2 path (A2).
    let offenders: Vec<&OutputAnnotation> = if opts.strict {
        output
            .iter()
            .filter(|a| {
                !a.annotation.is_structured || a.annotation.annotation_type == AnnotationType::Bare
            })
            .collect()
    } else {
        Vec::new()
    };
    let has_strict_violation = !offenders.is_empty();

    let mut stdout = io::stdout().lock();
    let write_result = stdout
        .write_all(json.as_bytes())
        .and_then(|_| stdout.write_all(b"\n"));
    match write_result {
        Ok(()) => {}
        // Downstream closed the pipe (e.g. `... | head`). Still honor --strict.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {
            if has_strict_violation {
                emit_strict_diagnostics(&offenders);
                return Ok(RunStatus::StrictViolation);
            }
            return Ok(RunStatus::Success);
        }
        Err(e) => return Err(format!("failed to write stdout: {e}")),
    }

    if has_strict_violation {
        emit_strict_diagnostics(&offenders);
        return Ok(RunStatus::StrictViolation);
    }
    Ok(RunStatus::Success)
}

fn emit_strict_diagnostics(offenders: &[&OutputAnnotation]) {
    eprintln!(
        "strict: {} violation(s) (unstructured or untyped)",
        offenders.len()
    );
    for o in offenders {
        let file = o.file.as_deref().unwrap_or("<stdin>");
        let start = o.annotation.char_start;
        let end = o.annotation.char_end;
        let original = truncate_for_diag(&o.annotation.original, 60);
        eprintln!("{file}:{start}..{end}: {original}");
    }
}

/// Truncate `s` to at most `max` chars, appending `...` when clipped.
fn truncate_for_diag(s: &str, max: usize) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{head}...")
    } else {
        head
    }
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
            Ok(RunStatus::Success) => ExitCode::SUCCESS,
            Ok(RunStatus::StrictViolation) => ExitCode::from(2),
            Err(msg) => {
                // Runtime errors: short message + one-line hint, no full usage dump.
                eprintln!("error: {msg}");
                eprintln!("try 'lit-annotation --help' for usage");
                ExitCode::from(1)
            }
        },
        Err(msg) => {
            // Parse/usage errors still dump the full usage text.
            eprintln!("error: {msg}");
            eprintln!("{}", usage());
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lit_annotation_core::types::{AnnotationForm, AnnotationType, Certainty, Scope, ScopeKind};
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
                assert!(!opts.bare);
                assert!(opts.marks.is_none());
                assert!(opts.inputs.is_empty());
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_args_bare() {
        let cmd = parse_args(&s(&["--bare"])).unwrap();
        match cmd {
            Cmd::Run(opts) => assert!(opts.bare),
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
                    opts.inputs,
                    vec![
                        Input::Path(PathBuf::from("a.md")),
                        Input::Path(PathBuf::from("b.md")),
                    ]
                );
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_args_combinations() {
        let cmd = parse_args(&s(&["--pretty", "--strict", "--marks", "m.toml", "doc.md"])).unwrap();
        match cmd {
            Cmd::Run(opts) => {
                assert!(opts.pretty);
                assert!(opts.strict);
                assert_eq!(opts.marks.as_deref(), Some(Path::new("m.toml")));
                assert_eq!(opts.inputs, vec![Input::Path(PathBuf::from("doc.md"))]);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_args_dash_is_stdin() {
        let cmd = parse_args(&s(&["-"])).unwrap();
        match cmd {
            Cmd::Run(opts) => assert_eq!(opts.inputs, vec![Input::Stdin]),
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_args_duplicate_dash_rejected() {
        let err = parse_args(&s(&["-", "-"])).unwrap_err();
        assert!(err.contains("at most once"), "err={err}");
    }

    #[test]
    fn parse_args_marks_flag_like_next_arg() {
        let err = parse_args(&s(&["--marks", "--strict"])).unwrap_err();
        assert!(err.contains("missing value for --marks"), "err={err}");
        assert!(err.contains("looks like a flag"), "err={err}");
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

    #[test]
    fn parse_args_marks_stdin_space_form_looks_like_missing_value() {
        // Space-form `--marks -`: `-` looks like a flag, so A5a wins over the
        // dedicated stdin rejection (equals-form still uses parse_marks_value).
        let err = parse_args(&s(&["--marks", "-"])).unwrap_err();
        assert!(err.contains("missing value for --marks"), "err={err}");
    }

    #[test]
    fn parse_args_marks_stdin_equals_form_rejected() {
        let err = parse_args(&s(&["--marks=-"])).unwrap_err();
        assert!(
            err.contains("marks cannot be read from stdin") || err.contains("stdin"),
            "err={err}"
        );
    }

    #[test]
    fn parse_args_marks_dash_leading_space_form_is_missing_value() {
        let err = parse_args(&s(&["--marks", "-foo"])).unwrap_err();
        assert!(err.contains("missing value for --marks"), "err={err}");
        assert!(err.contains("looks like a flag"), "err={err}");
    }

    #[test]
    fn parse_args_marks_dash_leading_equals_form_rejected() {
        let err = parse_args(&s(&["--marks=-foo"])).unwrap_err();
        assert!(
            err.contains("dash") || err.contains("invalid") || err.contains("-"),
            "err={err}"
        );
        assert!(!err.contains("missing value"), "err={err}");
    }

    #[test]
    fn parse_args_end_of_options_separator() {
        let cmd = parse_args(&s(&["--", "--strict"])).unwrap();
        match cmd {
            Cmd::Run(opts) => {
                assert!(!opts.strict, "--strict after -- must be a file");
                assert_eq!(opts.inputs, vec![Input::Path(PathBuf::from("--strict"))]);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_args_end_of_options_alone_uses_stdin() {
        let cmd = parse_args(&s(&["--"])).unwrap();
        match cmd {
            Cmd::Run(opts) => {
                assert!(opts.inputs.is_empty());
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_args_dash_after_double_dash_is_stdin() {
        let cmd = parse_args(&s(&["--", "-"])).unwrap();
        match cmd {
            Cmd::Run(opts) => assert_eq!(opts.inputs, vec![Input::Stdin]),
            other => panic!("expected Run, got {other:?}"),
        }
    }

    // --- parse_input: fenced path -----------------------------------------

    #[test]
    fn parse_input_fenced_compact() {
        let content = r#"<!--- n? ^"dharma" | Possibly a technical term here. --->"#;
        let codes = builtin_mark_codes();
        let anns = parse_document(content, codes);
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
        let anns = parse_document(content, builtin_mark_codes());
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].annotation_type, AnnotationType::Question);
        assert_eq!(anns[0].body, Some("keep".to_string()));
    }

    // --- parse_document / parse_bare --------------------------------------

    #[test]
    fn parse_document_plain_prose_yields_empty() {
        assert!(parse_document("just prose", builtin_mark_codes()).is_empty());
    }

    #[test]
    fn parse_document_legacy_percent_bang() {
        let anns = parse_document("hello %%! n | legacy note %% more", builtin_mark_codes());
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].annotation_type, AnnotationType::Note);
        assert_eq!(anns[0].body, Some("legacy note".to_string()));
    }

    #[test]
    fn parse_input_routes_on_bare_flag() {
        let prose = "just prose";
        // document mode (default): prose is not an annotation
        assert!(parse_input(prose, builtin_mark_codes(), false).is_empty());
        // bare mode: prose becomes one unstructured annotation
        let anns = parse_input(prose, builtin_mark_codes(), true);
        assert_eq!(anns.len(), 1);
        assert!(!anns[0].is_structured);
    }

    #[test]
    fn parse_input_bare_block() {
        let content = r#"
n
^"viracitaḥ"
---
Past participle of vi + √rac ("to arrange, compose") - "composed by, authored by."
"#;
        let anns = parse_bare(content, builtin_mark_codes());
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
        let anns = parse_bare(content, builtin_mark_codes());
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
        assert!(parse_bare("   \n\t  \n", builtin_mark_codes()).is_empty());
        assert!(parse_bare("", builtin_mark_codes()).is_empty());
        assert!(parse_document("   \n\t  \n", builtin_mark_codes()).is_empty());
        assert!(parse_document("", builtin_mark_codes()).is_empty());
    }

    // --- parse_bare: scope coverage ---------------------------------------

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
            (
                r#"n ^"anuttara" | body"#,
                Scope::Anchor("anuttara".to_string()),
            ),
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
            let anns = parse_bare(input, codes);
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
        assert!(
            err.contains("failed to read") || err.contains("No such"),
            "err={err}"
        );
    }

    #[test]
    fn load_mark_codes_invalid_toml() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "not = [valid toml").unwrap();
        let err = load_mark_codes(Some(tmp.path())).unwrap_err();
        assert!(err.contains("invalid marks TOML"), "err={err}");
    }
}
