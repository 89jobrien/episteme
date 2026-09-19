# Plan: Document to Research Note

## Goal

Deliver the first Episteme vertical slice from a stable inbox document to an indexed, unprocessed,
source-grounded research note.

## Completed tasks

1. Validate explicit loopback model endpoints.
2. Add safe source names and BLAKE3 digest domain types.
3. Persist rebuildable ingestion provenance in DuckDB.
4. Route PDF, HTML, and image extraction with deterministic OCR fallback.
5. Generate native Rust BAML clients and validate grounded research drafts.
6. Write research notes and archive copies atomically without clobbering vault content.
7. Resume safely after partial archival failure.
8. Report missing external dependencies.
9. Accept only stable regular files beneath the inbox root.
10. Guard live tests against the real vault and wire the operational CLI.

## Verification

```text
baml-cli generate
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --all-features
cargo test --doc
```
