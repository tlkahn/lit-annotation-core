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

The crate also ships a thin `lit-annotation` binary for shell-level inspection and CI checks. The library API is unchanged; the binary is a thin shell over `parser` / `block` / `compact` / `marks`.

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

# pipe-friendly (broken pipe from `head`/`jq` exits 0 quietly)
$ cat doc.md | lit-annotation | jq '.[].body'

# one or more file args instead of stdin; each annotation carries a `file` field
$ lit-annotation notes.md chapter2.md

# dash-named files via end-of-options separator
$ lit-annotation -- --strict

# CI lint: fail if any annotation is unstructured (diagnostics on stderr)
$ lit-annotation --strict notes.md > /dev/null
```

| Flag | Effect |
|------|--------|
| `--pretty` | Pretty-print JSON (default: compact single-line) |
| `--strict` | Exit 2 if any parsed annotation has `is_structured: false`; writes a count line and per-offender diagnostics to stderr |
| `--bare` | Treat input as a single fence-free annotation body (opt-in; default is document scan) |
| `--marks <path>` | Load mark codes from a TOML file (overlay on builtins). Rejects `-` and dash-leading values. |
| `--` | End of options; remaining args are file paths |
| `-h`, `--help` / `--version` | Standard |

With no `FILE` args, read stdin (`file` is `null` on each annotation). Multiple files yield one combined JSON array in file order; each annotation includes a `file` field with the path as given. Zero annotations yields `[]`, not an error. A closed stdout pipe (e.g. `lit-annotation doc.md \| head -c 20`) exits 0 with empty stderr.

| Exit code | Meaning |
|-----------|---------|
| 0 | Success (including zero annotations, including broken pipe) |
| 1 | I/O or usage error (unreadable file, bad flag); short diagnostics on stderr (full usage only for parse errors) |
| 2 | `--strict` only: at least one annotation parsed as unstructured |

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
