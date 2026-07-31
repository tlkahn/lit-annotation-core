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
