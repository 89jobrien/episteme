# Progress

## Completed

- Repository bootstrap, Rust 2024 binary/library structure, project rules, design, and execution plan.
- Loopback-only model endpoint and absolute tool-path validation.
- Safe source names, BLAKE3 identity, immutable read-only staging, stable-file watcher filtering.
- PDF text extraction, PDF OCR fallback, image OCR, and HTML conversion through explicit bounded
  shell-free tool adapters.
- Native Rust BAML clients for `ClassifyDocument` and `DistillResearch` with separate trusted and
  untrusted prompt roles, local timeouts, and deterministic evidence verification.
- Atomic no-clobber Obsidian research notes, inert generated Markdown, JSON-compatible frontmatter,
  symlink-aware vault directories, and retry-safe archive copies.
- DuckDB ingestion provenance and recovery across partial failures.
- CLI commands: `doctor`, `init`, `ingest`, and `watch`.
- `zk` index adapter, external dependency doctor, Just recipes, and full documentation.
- Ten default integration tests plus an opt-in live Apfel end-to-end test using a temporary vault.
- Signed root commit `093f5dc` with pre-commit formatting, obfsck, gitleaks, and Cargo checks passing.
- Standalone `classify` command with validated metadata, digest-based DuckDB caching, and JSON output.
- Descriptor-relative source staging with final- and intermediate-symlink rejection.
- Typed BAML chunk/map-reduce for large documents with stable source span IDs and deterministic Rust
  evidence reconstruction.
- Direct and chunked live-ingestion coverage using temporary vaults and local Ollama models.
- Private GitHub repository, release notes and changelog, signed merge history, and GitHub releases
  v0.1.0 and v0.1.1.
- Public crates.io package `episteme-local` v0.1.1 with complete metadata, portable example config,
  and MIT/Apache-2.0 license texts.
- Crux-native Rust release orchestrator with dependency ordering, quality gates, retries, resumable
  state, trace output, and no Python implementation.

## Verification evidence

- Formatting, all-target checks, strict Clippy, 28 nextest tests, and doc tests passed.
- Direct and chunked live BAML ingestion passed with grounded source evidence in temporary vaults.
- `cargo publish --dry-run` packaged and compiled `episteme-local` v0.1.1; the Crux release trace
  recorded the successful real crates.io upload.
- Architecture, security, quality, and release reviews found no unresolved blockers.
- External dependency doctor found Poppler, Tesseract, Pandoc, and `zk` available.

## In progress

- Session closeout: handoff and closeout commit/push.

## Not started

- Private `zk` plus msgvault search.
- Decisions, commitments, claims, assumptions, and forecast intelligence records.
- Profile-scoped daily, weekly, monthly, and quarterly briefings.
- Knowledge assurance, public research, Pandoc publication bundles, dashboard, backups, and safe
  declutter workflows.
- Converter sandboxing required before accepting automatic public downloads.
