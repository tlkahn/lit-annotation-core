//! End-to-end integration tests for the `lit-annotation` binary.

use serde_json::Value;
use std::io::Write;
use std::process::{Command, Output, Stdio};
use tempfile::NamedTempFile;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lit-annotation"))
}

fn run_stdin(args: &[&str], stdin: &str) -> Output {
    let mut child = bin()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn lit-annotation");
    {
        let mut h = child.stdin.take().expect("stdin");
        h.write_all(stdin.as_bytes()).expect("write stdin");
    }
    child.wait_with_output().expect("wait")
}

fn run_args(args: &[&str]) -> Output {
    bin()
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run lit-annotation")
}

fn stdout_str(out: &Output) -> &str {
    std::str::from_utf8(&out.stdout).expect("stdout utf8")
}

fn stderr_str(out: &Output) -> &str {
    std::str::from_utf8(&out.stderr).expect("stderr utf8")
}

fn parse_json(out: &Output) -> Value {
    let s = stdout_str(out).trim();
    serde_json::from_str(s).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON: {e}\nstdout={s:?}\nstderr={:?}",
            stderr_str(out)
        )
    })
}

// --- 1. Scaffold ----------------------------------------------------------

#[test]
fn empty_stdin_yields_empty_array() {
    let out = run_stdin(&[], "");
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    assert_eq!(stdout_str(&out).trim(), "[]");
}

// --- 5. Scope JSON wire shape --------------------------------------------

#[test]
fn scope_json_wire_shape() {
    // One annotation per interesting scope kind so jq consumers can rely on shape.
    let input = r#"
<!--- n _ | words --->
<!--- n ^"dharma" | anchor --->
<!--- n \d | document --->
<!--- n 3_1 | asymmetric --->
"#;
    let out = run_stdin(&[], input);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    let arr = v.as_array().expect("array");
    assert_eq!(arr.len(), 4);

    assert_eq!(
        arr[0]["scope"],
        serde_json::json!({"kind":"words","value":1})
    );
    assert_eq!(
        arr[1]["scope"],
        serde_json::json!({"kind":"anchor","value":"dharma"})
    );
    assert_eq!(arr[2]["scope"], serde_json::json!({"kind":"document"}));
    assert_eq!(
        arr[3]["scope"],
        serde_json::json!({
            "kind": "asymmetric",
            "value": {"unit": "word", "before": 3, "after": 1}
        })
    );
    // stdin => file is null on every annotation
    for a in arr {
        assert!(a["file"].is_null(), "file={:?}", a["file"]);
    }
}

// --- 7. Block-form example with --pretty ---------------------------------

#[test]
fn block_form_pretty_matches_issue_fields() {
    let input = r#"<!---
n
^"viracitaḥ"
---
Past participle of vi + √rac ("to arrange, compose") - "composed by, authored by."
--->"#;
    let out = run_stdin(&["--pretty"], input);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    let ann = &v[0];
    assert_eq!(ann["form"], "block");
    assert_eq!(ann["annotation_type"], "note");
    assert_eq!(ann["certainty"], "neutral");
    assert_eq!(
        ann["scope"],
        serde_json::json!({"kind":"anchor","value":"viracitaḥ"})
    );
    assert_eq!(
        ann["body"],
        "Past participle of vi + √rac (\"to arrange, compose\") - \"composed by, authored by.\""
    );
    assert!(ann["is_structured"].as_bool().unwrap());
    assert_eq!(ann["char_start"], 0);
    assert!(ann["char_end"].as_u64().unwrap() > 0);
    assert!(ann["original"].as_str().unwrap().contains("viracitaḥ"));
    assert!(ann["file"].is_null(), "file={:?}", ann["file"]);
}

// --- 8. Compact vs pretty output -----------------------------------------

#[test]
fn default_output_is_single_line() {
    let input = r#"<!--- n? ^"dharma" | Possibly a technical term here. --->"#;
    let out = run_stdin(&[], input);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let s = stdout_str(&out).trim_end_matches('\n');
    assert!(
        !s.contains('\n'),
        "expected single compact line, got: {s:?}"
    );
    let v: Value = serde_json::from_str(s).unwrap();
    assert!(v.is_array());
}

#[test]
fn pretty_output_is_multiline() {
    let input = r#"<!--- n? ^"dharma" | Possibly a technical term here. --->"#;
    let out = run_stdin(&["--pretty"], input);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let s = stdout_str(&out);
    assert!(s.contains('\n'), "expected multi-line pretty output");
    let v = parse_json(&out);
    assert_eq!(v[0]["annotation_type"], "note");
    assert_eq!(v[0]["certainty"], "tentative");
    assert_eq!(
        v[0]["scope"],
        serde_json::json!({"kind":"anchor","value":"dharma"})
    );
}

// --- 9. File args ---------------------------------------------------------

#[test]
fn single_file_arg() {
    let mut f = NamedTempFile::new().unwrap();
    write!(f, r#"<!--- n: | from file --->"#).unwrap();
    let out = run_args(&[f.path().to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["body"], "from file");
}

#[test]
fn multiple_file_args_combined_in_order() {
    let mut a = NamedTempFile::new().unwrap();
    let mut b = NamedTempFile::new().unwrap();
    write!(a, r#"<!--- n: | first --->"#).unwrap();
    write!(b, r#"<!--- q: | second --->"#).unwrap();
    let a_path = a.path().to_str().unwrap();
    let b_path = b.path().to_str().unwrap();
    let out = run_args(&[a_path, b_path]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["body"], "first");
    assert_eq!(arr[1]["body"], "second");
    assert_eq!(arr[0]["annotation_type"], "note");
    assert_eq!(arr[1]["annotation_type"], "question");
    // Per-file attribution: each annotation carries the path-as-given.
    assert_eq!(arr[0]["file"], a_path);
    assert_eq!(arr[1]["file"], b_path);
    // Both start at char 0; file field disambiguates.
    assert_eq!(arr[0]["char_start"], 0);
    assert_eq!(arr[1]["char_start"], 0);
}

#[test]
fn stdin_annotations_have_null_file() {
    let input = r#"<!--- n: | from stdin --->"#;
    let out = run_stdin(&[], input);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert!(v[0]["file"].is_null(), "file={:?}", v[0]["file"]);
}

#[test]
fn nonexistent_file_exits_1_no_partial_json() {
    let out = run_args(&["/nonexistent/path/does-not-exist.md"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_str(&out);
    assert!(
        err.contains("error: failed to read"),
        "stderr should mention failed to read, got: {err}"
    );
    // Runtime errors must NOT dump the full usage text.
    assert!(
        !err.contains("Usage:"),
        "runtime error should not dump Usage:, got: {err}"
    );
    // A short help hint is fine.
    assert!(
        err.contains("try 'lit-annotation --help'") || err.contains("--help"),
        "stderr should hint at --help, got: {err}"
    );
    // No partial JSON on stdout
    let stdout = stdout_str(&out).trim();
    assert!(
        stdout.is_empty(),
        "expected empty stdout on unreadable file, got {stdout:?}"
    );
}

#[test]
fn unknown_flag_still_dumps_usage() {
    let out = run_args(&["--not-a-real-flag"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_str(&out);
    assert!(err.contains("Usage:"), "stderr={err}");
}

#[test]
fn marks_missing_value_dumps_usage() {
    let out = run_args(&["--marks"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_str(&out);
    assert!(err.contains("Usage:"), "stderr={err}");
    assert!(err.contains("missing value"), "stderr={err}");
}

#[test]
fn broken_pipe_exits_0_quietly() {
    // Produce far more than 64 KiB of stdout so writing past a short-lived
    // reader reliably hits EPIPE (pipe capacity is typically 64 KiB).
    let mut input = String::new();
    for i in 0..1000 {
        input.push_str(&format!(
            "<!--- n: | annotation number {i:04} with substantial padding text to grow the JSON payload well past the OS pipe buffer capacity --->\n"
        ));
    }

    let mut child = bin()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");

    {
        let mut h = child.stdin.take().expect("stdin");
        h.write_all(input.as_bytes()).expect("write stdin");
    }

    // Read only a few bytes, then drop the pipe (simulates `... | head -c 20`).
    {
        use std::io::Read;
        let mut stdout = child.stdout.take().expect("stdout");
        let mut buf = [0u8; 20];
        let _ = stdout.read(&mut buf);
        // drop stdout => closed pipe
    }

    let mut stderr_buf = String::new();
    if let Some(mut err) = child.stderr.take() {
        use std::io::Read;
        let _ = err.read_to_string(&mut stderr_buf);
    }
    let status = child.wait().expect("wait");
    assert_eq!(
        status.code(),
        Some(0),
        "broken pipe must exit 0, got {status:?}; stderr={stderr_buf:?}"
    );
    assert!(
        stderr_buf.is_empty(),
        "broken pipe must be silent on stderr, got: {stderr_buf:?}"
    );
}

#[test]
fn strict_violation_survives_broken_pipe() {
    // 1 unstructured violation + enough structured annotations that stdout
    // exceeds pipe capacity. Closing stdout early must still exit 2.
    let mut input = String::from("<!--- compare Vasugupta SpK 1.1 --->\n");
    for i in 0..1000 {
        input.push_str(&format!(
            "<!--- n: | annotation number {i:04} with substantial padding text to grow the JSON payload well past the OS pipe buffer capacity --->\n"
        ));
    }

    let mut child = bin()
        .args(["--strict"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");

    {
        let mut h = child.stdin.take().expect("stdin");
        h.write_all(input.as_bytes()).expect("write stdin");
    }

    {
        use std::io::Read;
        let mut stdout = child.stdout.take().expect("stdout");
        let mut buf = [0u8; 20];
        let _ = stdout.read(&mut buf);
    }

    let mut stderr_buf = String::new();
    if let Some(mut err) = child.stderr.take() {
        use std::io::Read;
        let _ = err.read_to_string(&mut stderr_buf);
    }
    let status = child.wait().expect("wait");
    assert_eq!(
        status.code(),
        Some(2),
        "strict violation must survive broken pipe, got {status:?}; stderr={stderr_buf:?}"
    );
    assert!(
        stderr_buf.contains("strict:") && stderr_buf.contains("violation"),
        "stderr must report strict violation, got: {stderr_buf:?}"
    );
}

// --- 10. --strict ---------------------------------------------------------

#[test]
fn strict_with_unstructured_exits_2_and_prints_json() {
    // Bare free-text inside fences is unstructured (is_structured == false)
    let input = "<!--- compare Vasugupta SpK 1.1 --->";
    let out = run_stdin(&["--strict"], input);
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["is_structured"], false);
}

#[test]
fn strict_emits_stderr_diagnostics() {
    let input = "<!--- compare Vasugupta SpK 1.1 --->";
    let out = run_stdin(&["--strict"], input);
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr_str(&out));
    // JSON still on stdout
    let v = parse_json(&out);
    assert_eq!(v.as_array().unwrap().len(), 1);

    let err = stderr_str(&out);
    assert!(
        err.contains("strict: 1 violation(s) (unstructured or untyped)"),
        "stderr missing count line: {err}"
    );
    assert!(
        err.contains("<stdin>"),
        "stderr missing <stdin> attribution: {err}"
    );
    assert!(
        err.contains("..") || err.contains("char"),
        "stderr missing char range: {err}"
    );
    // original truncated into the diagnostic
    assert!(
        err.contains("compare Vasugupta") || err.contains("SpK"),
        "stderr missing original snippet: {err}"
    );
}

#[test]
fn strict_diagnostics_include_file_path() {
    let mut f = NamedTempFile::new().unwrap();
    write!(f, "<!--- bare free text note --->").unwrap();
    let path = f.path().to_str().unwrap();
    let out = run_args(&["--strict", path]);
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr_str(&out));
    let err = stderr_str(&out);
    assert!(
        err.contains("strict: 1 violation(s) (unstructured or untyped)"),
        "stderr={err}"
    );
    assert!(err.contains(path), "stderr should include file path: {err}");
}

#[test]
fn strict_with_only_structured_exits_0() {
    let input = r#"<!--- n: | structured note --->"#;
    let out = run_stdin(&["--strict"], input);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert_eq!(v[0]["is_structured"], true);
}

#[test]
fn strict_rejects_bare_with_lone_pipe() {
    let input = "<!--- | just pipe --->";
    let out = run_stdin(&["--strict"], input);
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr_str(&out));
    let err = stderr_str(&out);
    assert!(
        err.contains("strict:") && err.contains("violation"),
        "stderr={err}"
    );
}

#[test]
fn strict_rejects_bare_date_only() {
    let input = "<!--- @2026-03 --->";
    let out = run_stdin(&["--strict"], input);
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr_str(&out));
}

#[test]
fn strict_rejects_bare_empty_block_head() {
    let input = "<!---\n---\njust a note body\n--->";
    let out = run_stdin(&["--strict"], input);
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr_str(&out));
}

#[test]
fn strict_accepts_typed_and_mark() {
    let input = r#"<!--- n: | typed note --->
<!--- sic | x --->"#;
    let out = run_stdin(&["--strict"], input);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert_eq!(v.as_array().unwrap().len(), 2);
}

// --- 11. --marks ----------------------------------------------------------

#[test]
fn marks_flag_recognizes_custom_code() {
    let mut marks = NamedTempFile::new().unwrap();
    writeln!(
        marks,
        r#"[zz]
label = "Custom ZZ"
"#
    )
    .unwrap();

    let input = "<!--- zz _ --->";
    let marks_path = marks.path().to_str().unwrap();

    let with = run_stdin(&["--marks", marks_path], input);
    assert_eq!(with.status.code(), Some(0), "stderr={}", stderr_str(&with));
    let v = parse_json(&with);
    assert_eq!(v[0]["annotation_type"], "mark");
    assert_eq!(v[0]["mark"], "zz");

    let without = run_stdin(&[], input);
    assert_eq!(
        without.status.code(),
        Some(0),
        "stderr={}",
        stderr_str(&without)
    );
    let v2 = parse_json(&without);
    assert_eq!(v2[0]["annotation_type"], "bare");
}

#[test]
fn marks_nonexistent_exits_1() {
    let out = run_stdin(&["--marks", "/nonexistent/marks.toml"], "<!--- n: | x --->");
    assert_eq!(out.status.code(), Some(1));
    assert!(!stderr_str(&out).is_empty());
}

#[test]
fn marks_stdin_dash_space_form_is_missing_value() {
    // Space-form `--marks -`: next arg looks like a flag (A5a).
    let out = run_args(&["--marks", "-"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_str(&out);
    assert!(err.contains("missing value for --marks"), "stderr={err}");
    assert!(err.contains("Usage:"), "stderr={err}");
}

#[test]
fn marks_stdin_dash_equals_form_rejected() {
    let out = run_args(&["--marks=-"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_str(&out);
    assert!(
        err.contains("marks cannot be read from stdin") || err.contains("stdin"),
        "stderr={err}"
    );
}

#[test]
fn marks_flag_like_next_arg_is_missing_value() {
    let out = run_args(&["--marks", "--strict"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_str(&out);
    assert!(err.contains("missing value for --marks"), "stderr={err}");
    assert!(err.contains("looks like a flag"), "stderr={err}");
    assert!(err.contains("Usage:"), "stderr={err}");
}

#[test]
fn marks_dash_leading_space_form_is_missing_value() {
    let out = run_args(&["--marks", "-foo"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_str(&out);
    assert!(err.contains("missing value for --marks"), "stderr={err}");
    assert!(err.contains("Usage:"), "stderr={err}");
}

#[test]
fn marks_dash_leading_equals_form_rejected() {
    let out = run_args(&["--marks=-foo"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_str(&out);
    assert!(
        !err.contains("missing value"),
        "equals-form dash-leading marks value must not look like missing value; err={err}"
    );
    assert!(err.contains("Usage:"), "err={err}");
}

#[test]
fn end_of_options_treats_dash_named_file_as_positional() {
    use tempfile::TempDir;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("--strict");
    std::fs::write(&path, r#"<!--- n: | from dash file --->"#).unwrap();
    let out = run_args(&["--", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["body"], "from dash file");
}

#[test]
fn end_of_options_alone_reads_stdin() {
    let out = run_stdin(&["--"], r#"<!--- n: | via dash dash --->"#);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert_eq!(v[0]["body"], "via dash dash");
}

// --- L5: `-` reads stdin --------------------------------------------------

#[test]
fn dash_reads_stdin() {
    let out = run_stdin(&["-"], r#"<!--- n: | x --->"#);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["body"], "x");
    assert!(v[0]["file"].is_null(), "file={:?}", v[0]["file"]);
}

#[test]
fn dash_after_double_dash_reads_stdin() {
    let out = run_stdin(&["--", "-"], r#"<!--- n: | via dash after eoopt --->"#);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert_eq!(v[0]["body"], "via dash after eoopt");
    assert!(v[0]["file"].is_null());
}

#[test]
fn dash_mixed_with_file() {
    let mut f = NamedTempFile::new().unwrap();
    write!(f, r#"<!--- n: | from file --->"#).unwrap();
    let path = f.path().to_str().unwrap();

    // file then stdin
    let out = run_stdin(&[path, "-"], r#"<!--- q: | from stdin --->"#);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["body"], "from file");
    assert_eq!(arr[0]["file"], path);
    assert_eq!(arr[1]["body"], "from stdin");
    assert!(arr[1]["file"].is_null());

    // stdin then file
    let out = run_stdin(&["-", path], r#"<!--- q: | from stdin first --->"#);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["body"], "from stdin first");
    assert!(arr[0]["file"].is_null());
    assert_eq!(arr[1]["body"], "from file");
    assert_eq!(arr[1]["file"], path);
}

#[test]
fn duplicate_dash_is_usage_error() {
    let out = run_args(&["-", "-"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_str(&out);
    assert!(
        err.contains("stdin") || err.contains("-"),
        "stderr should mention duplicate stdin/- , got: {err}"
    );
    assert!(err.contains("Usage:"), "stderr={err}");
}

// --- 12. help / version / unknown ----------------------------------------

#[test]
fn help_exits_0() {
    let out = run_args(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let s = stdout_str(&out);
    assert!(s.contains("Usage:"), "stdout={s}");
    assert!(s.contains("--pretty"), "stdout={s}");
}

#[test]
fn version_exits_0() {
    let out = run_args(&["--version"]);
    assert_eq!(out.status.code(), Some(0));
    let s = stdout_str(&out).trim();
    assert_eq!(s, env!("CARGO_PKG_VERSION"));
}

#[test]
fn unknown_flag_exits_1_with_usage() {
    let out = run_args(&["--not-a-real-flag"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_str(&out);
    assert!(
        err.contains("unknown flag") || err.contains("Usage:"),
        "stderr={err}"
    );
}

// --- Document mode default (no auto bare) ---------------------------------

#[test]
fn plain_prose_stdin_yields_empty_array() {
    let out = run_stdin(&[], "just some prose without annotations");
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    assert_eq!(stdout_str(&out).trim(), "[]");
}

#[test]
fn plain_prose_under_strict_exits_0() {
    let out = run_stdin(&["--strict"], "just some prose without annotations");
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    assert_eq!(stdout_str(&out).trim(), "[]");
}

#[test]
fn thematic_break_markdown_yields_empty_array() {
    // Data-loss repro: markdown with `---` thematic breaks and no fences must
    // not become one synthetic bare annotation.
    let mut f = NamedTempFile::new().unwrap();
    write!(
        f,
        "# Title\n\nIntro paragraph.\n\n---\n\nSecond section text.\n"
    )
    .unwrap();
    let out = run_args(&[f.path().to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    assert_eq!(stdout_str(&out).trim(), "[]");
}

#[test]
fn prose_containing_pipe_yields_empty_array() {
    // Truncation repro: prose with `|` must not be treated as compact form.
    let out = run_stdin(&[], "a | b | c without fences");
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    assert_eq!(stdout_str(&out).trim(), "[]");
}

#[test]
fn legacy_percent_bang_document_is_scanned() {
    let input = "hello %%! n | legacy note %% more";
    let out = run_stdin(&[], input);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["annotation_type"], "note");
    assert_eq!(v[0]["body"], "legacy note");
    assert_eq!(v[0]["is_structured"], true);
}

#[test]
fn fenced_code_block_annotations_are_skipped() {
    // Issue #1 AC at CLI level: fence-escaped annotations are ignored;
    // live ones are kept.
    let input = "```\n<!--- skip me --->\n```\n<!--- q? | keep --->";
    let out = run_stdin(&[], input);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["annotation_type"], "question");
    assert_eq!(v[0]["body"], "keep");
}

// --- --bare flag ----------------------------------------------------------

#[test]
fn bare_flag_compact_text() {
    let out = run_stdin(&["--bare"], "n: | quick note");
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["form"], "compact");
    assert_eq!(v[0]["annotation_type"], "note");
    assert_eq!(v[0]["body"], "quick note");
    assert_eq!(v[0]["char_start"], 0);
    assert!(v[0]["char_end"].as_u64().unwrap() > 0);
    assert_eq!(v[0]["original"], "n: | quick note");
}

#[test]
fn bare_flag_block_text() {
    let input = r#"n
^"viracitaḥ"
---
Past participle body.
"#;
    let out = run_stdin(&["--bare", "--pretty"], input);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert_eq!(v[0]["form"], "block");
    assert_eq!(v[0]["annotation_type"], "note");
    assert_eq!(
        v[0]["scope"],
        serde_json::json!({"kind":"anchor","value":"viracitaḥ"})
    );
    assert_eq!(v[0]["body"], "Past participle body.");
    assert_eq!(v[0]["char_start"], 0);
}

#[test]
fn bare_flag_whitespace_only_yields_empty() {
    let out = run_stdin(&["--bare"], "   \n\t  \n");
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    assert_eq!(stdout_str(&out).trim(), "[]");
}

#[test]
fn empty_fence_body_is_null_and_unstructured() {
    let input = "<!---   --->";
    let out = run_stdin(&["--strict"], input);
    assert_eq!(out.status.code(), Some(2), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert!(v[0]["body"].is_null(), "body={:?}", v[0]["body"]);
    assert_eq!(v[0]["is_structured"], false);
}

// --- Bare input via CLI (legacy name kept as --bare path) -----------------

#[test]
fn bare_input_via_stdin() {
    let input = r#"n
^"viracitaḥ"
---
Past participle body.
"#;
    let out = run_stdin(&["--bare", "--pretty"], input);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert_eq!(v[0]["form"], "block");
    assert_eq!(v[0]["annotation_type"], "note");
    assert_eq!(
        v[0]["scope"],
        serde_json::json!({"kind":"anchor","value":"viracitaḥ"})
    );
    assert_eq!(v[0]["body"], "Past participle body.");
    assert_eq!(v[0]["char_start"], 0);
}
