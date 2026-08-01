# lit-annotation-core

Scanner, parser, scope resolver, and emitter for the [Lit](https://github.com/tlkahn/lit) annotation DSL.

Shared by:

- **Lit** (desktop authoring app, Tauri 2)
- **Lif** (mobile reader, Flutter + Rust)

Both apps depend on this crate as a **pinned git tag**. Grammar behavior is identical across consumers because they share this code and the exact-pinned `sentencex` sentence segmenter.

## Modules

| Module | Role |
|--------|------|
| `types` | Core AST types (`Annotation`, `Scope`, certainty, etc.) |
| `scanner` | Byte/UTF-16 scanning, fenced-range detection, authored IDs |
| `parser` | Parse annotation comments into typed AST |
| `compact` | Compact (single-line) form parsing |
| `block` | Block (multi-line) form parsing |
| `marks` | Mark configuration (`marks_builtin.toml` + user overrides) |
| `lang` | Language tag canonicalization for segmentation |
| `scope_resolver` | Resolve annotation scopes against document body text |
| `emit` | Serialize annotations back to DSL source |

## Versioning contract

**Every grammar change gets a new tag. Consumers pin tags; never track `main`.**

```toml
# In a consumer Cargo.toml
lit-annotation-core = { git = "https://github.com/tlkahn/lit-annotation-core.git", tag = "v0.1.0" }
```

When the grammar changes:

1. Land the change on `main` of this repo.
2. Tag a new version (`v0.1.1`, `v0.2.0`, ...).
3. Bump the `tag =` pin in each consumer.
4. Re-run the consumer's full test suite before merging the pin bump.

## `sentencex` pin

`sentencex = "=0.1.30"` is pinned exactly. Sentence segmentation is load-bearing for scope resolution (lit#945): index time and live preview must agree on sentence boundaries. Bump deliberately and re-run the ctx parity sweeps.

## CLI

The crate also ships a `lit-annotation` binary for shell-level inspection and CI checks. It is a thin shell over `parser` / `block` / `compact` / `marks`, but shipping it did require a few deliberate library changes (see below).

```bash
cargo build --release
# binary at target/release/lit-annotation

# or without installing:
cargo run --bin lit-annotation -- --help
```

**Document mode is the default.** Input is scanned for `<!--- ... --->` fences and legacy `%%! ... %%` markers. Plain prose, markdown thematic breaks (`---`), and pipe characters are not treated as annotations - zero matches yields `[]`. Pass `--bare` only when the entire input is a single fence-free annotation body.

```console
# block form (document with annotation comments)
$ lit-annotation --pretty <<'EOF'
<!---
n
^"viracitaḥ"
---
Past participle of vi + √rac ("to arrange, compose") - "composed by, authored by."
--->
EOF

# compact (inline) form
$ echo '<!--- n? ^"dharma" | Possibly a technical term here. --->' | lit-annotation --pretty

# bare input (no fences): requires --bare
$ lit-annotation --bare <<'EOF'
n
^"viracitaḥ"
---
Past participle of vi + √rac ("to arrange, compose") - "composed by, authored by."
EOF

# `-` also reads stdin (useful mixed with file args)
$ echo '<!--- n: | x --->' | lit-annotation -

# pipe-friendly (broken pipe from `head`/`jq` exits 0 quietly when clean;
# `--strict` still exits 2 if there are violations)
$ cat doc.md | lit-annotation | jq '.[].body'

# one or more file args instead of stdin; each annotation carries a `file` field
$ lit-annotation notes.md chapter2.md

# dash-named files via end-of-options separator
$ lit-annotation -- --strict

# CI lint: fail if any annotation is unstructured or untyped/bare
$ lit-annotation --strict notes.md > /dev/null
```

| Flag | Effect |
|------|--------|
| `--pretty` | Pretty-print JSON (default: compact single-line) |
| `--strict` | Exit 2 if any parsed annotation has `is_structured: false` **or** `annotation_type: bare`; writes a count line (`strict: N violation(s) (unstructured or untyped)`) and per-offender diagnostics to stderr. Evaluated even when stdout is a broken pipe. |
| `--bare` | Treat each input blob as a single fence-free annotation body (opt-in; default is document scan). With multiple FILE args, each file is one blob - multi-annotation bare text is collapsed into one annotation per blob. |
| `--marks <path>` | Load mark codes from a TOML file (overlay on builtins). Rejects `-` and dash-leading values; a flag-like next arg after space-form `--marks` reports "missing value". |
| `--` | End of options; remaining args are file paths (or `-` for stdin) |
| `-h`, `--help` / `--version` | Standard |

With no `FILE` args, read stdin (`file` is `null` on each annotation). `-` as a FILE also reads stdin (`file: null`); it may be given at most once and can be mixed with path args in any order. Multiple inputs yield one combined JSON array in arg order; each annotation includes a `file` field with the path as given (not canonicalized). Zero annotations yields `[]`, not an error. A closed stdout pipe (e.g. `lit-annotation doc.md \| head -c 20`) exits 0 with empty stderr when there is no strict violation.

| Exit code | Meaning |
|-----------|---------|
| 0 | Success (including zero annotations, including broken pipe with no strict violations) |
| 1 | I/O or usage error (unreadable file, bad flag); short diagnostics on stderr (full usage only for parse errors) |
| 2 | `--strict` only: at least one annotation is unstructured or untyped (`bare`) |

### Deliberate library changes (alongside the CLI)

These are intentional grammar/API tightenings, not accidental CLI side effects:

- `overlay_on_builtins` for mark-config merging (file wins per code)
- Block form: unrecognized head lines flip `is_structured: false` (parsed fields retained)
- Empty unstructured bodies yield `body: null` (not `""`)
- Compact form: dates are trailing-only (`@YYYY-MM` / `@YYYY-MM-DD` at end of body); mid-body dates stay in the body text
- Compact form: non-empty unrecognized residue before `|` flips `is_structured: false` (mirrors block form) |

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
