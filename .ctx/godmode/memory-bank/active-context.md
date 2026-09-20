# Active Context

## Current focus

The ingestion, standalone classification, and grounded chunked-distillation slices are released.
The next planned slice is private search across `zk` and msgvault using local BAML calls.

## Current state

- All 18 tasks in `.ctx/godmode/tasks.yaml` are done.
- `main` is synchronized with private `origin`; GitHub releases v0.1.0 and v0.1.1 are published.
- `episteme-local` v0.1.1 is published on crates.io while the binary and library remain `episteme`.
- The full Cargo gate, package dry-run, Crux release dry-run, direct live ingestion, and chunked live
  ingestion passed.
- Only the pre-existing local `.gitignore` modification remains unstaged.

## Active decisions

- The private Obsidian vault is canonical; DuckDB is rebuildable derived state.
- BAML runtime and generated client versions are pinned to `0.221.0`.
- Model endpoints must use explicit loopback IPs; scoped Apfel tests use dedicated port `18181` and
  must not use Ollama's `11434` port.
- Current ingestion accepts locally trusted documents only. Automatic public downloads remain
  disabled until networkless converter sandboxing and aggregate resource limits exist.
- Inbox originals are retained for explicit cleanup; Episteme archives immutable copies and never
  automatically deletes user-owned source files.
- Generated research prose is Markdown-escaped and evidence quotes are verified against extracted
  source text before vault persistence.
- Large-document evidence is model-selected by stable source span ID, then quote text and character
  locations are reconstructed deterministically in Rust.
- The crates.io package name is `episteme-local`; `episteme` is retained for the CLI and Rust library.
- Release orchestration uses `orchestrate.crux` with typed Rust plugin handlers. Dry-runs bypass
  credential injection, and resumable state lives under `target/.release-state.json`.

## Next actions

1. Design the private-search source policy and query result schema.
2. Add `zk` and msgvault search ports plus BAML answer synthesis behind local-only clients.
3. Decide whether the existing crates.io `episteme` knowledge graph merits an optional aliased
   integration after the search boundary is designed.

## Open questions

- Should private search initially cover the entire vault or explicit profile allowlists only?
- Which macOS isolation mechanism should gate future untrusted public-document conversion?
- Should the GitHub repository eventually become public now that crates.io exposes package source?
