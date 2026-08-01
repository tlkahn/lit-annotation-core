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
        panic!("stdout is not JSON: {e}\nstdout={s:?}\nstderr={:?}", stderr_str(out))
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

    assert_eq!(arr[0]["scope"], serde_json::json!({"kind":"words","value":1}));
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
    let out = run_args(&[a.path().to_str().unwrap(), b.path().to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["body"], "first");
    assert_eq!(arr[1]["body"], "second");
    assert_eq!(arr[0]["annotation_type"], "note");
    assert_eq!(arr[1]["annotation_type"], "question");
}

#[test]
fn nonexistent_file_exits_1_no_partial_json() {
    let out = run_args(&["/nonexistent/path/does-not-exist.md"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(!stderr_str(&out).is_empty());
    // No partial JSON on stdout
    let stdout = stdout_str(&out).trim();
    assert!(
        stdout.is_empty() || serde_json::from_str::<Value>(stdout).is_err(),
        "stdout should not be valid partial JSON, got: {stdout:?}"
    );
    // Stronger: stdout should be empty on I/O error
    assert!(
        stdout.is_empty(),
        "expected empty stdout on unreadable file, got {stdout:?}"
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
fn strict_with_only_structured_exits_0() {
    let input = r#"<!--- n: | structured note --->"#;
    let out = run_stdin(&["--strict"], input);
    assert_eq!(out.status.code(), Some(0), "stderr={}", stderr_str(&out));
    let v = parse_json(&out);
    assert_eq!(v[0]["is_structured"], true);
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
    assert!(err.contains("unknown flag") || err.contains("Usage:"), "stderr={err}");
}

// --- Bare input via CLI ---------------------------------------------------

#[test]
fn bare_input_via_stdin() {
    let input = r#"n
^"viracitaḥ"
---
Past participle body.
"#;
    let out = run_stdin(&["--pretty"], input);
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
