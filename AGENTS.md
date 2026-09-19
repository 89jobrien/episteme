# Episteme Agent Guide

Episteme is a private, local-first Rust application for document ingestion,
knowledge intelligence, and publication workflows.

## Constraints

- Private content and model requests stay on this Mac.
- Model endpoints must use explicit loopback IP addresses.
- External tools are invoked with argument arrays, never through a shell.
- The Obsidian vault is canonical; DuckDB contains rebuildable derived state.
- Tests use temporary vaults and databases. Never target the real vault.
- No operation may delete, publish, or overwrite user content automatically.

## Quality gates

```text
baml-cli generate
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --all-features
cargo test --doc
```

## Rust conventions

- Rust 2024 edition.
- `anyhow` at the CLI boundary; explicit `thiserror` errors in library code.
- No `unwrap()` or `expect()` in production code.
- Generated `src/baml_client/` code is exempt from hand-written lint policy and must never be
  edited directly; regenerate it from `baml_src/`.
- Keep external integrations behind narrow traits.
- Keep `main.rs` thin and test behavior through the library.
