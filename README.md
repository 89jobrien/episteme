# Episteme

Episteme is a privacy-preserving, local-first document ingestion pipeline for an Obsidian vault. It uses
deterministic extraction tools, typed BAML functions backed by explicitly configured local model
endpoints, atomic note persistence, DuckDB provenance, and `zk` indexing.

Install the `episteme` CLI from crates.io with:

```text
cargo install episteme-local
```

The package is named `episteme-local`; the installed executable and library crate remain
`episteme`.

## Current slice

Supported inputs:

- Text and scanned PDFs through Poppler and Tesseract
- HTML through Pandoc
- PNG, JPEG, TIFF, and WebP images through Tesseract

The pipeline creates `status: unprocessed` research notes and copies immutable originals into the
archive only after a note has been persisted successfully. The inbox copy remains until explicit
cleanup. Episteme does not publish, delete user-owned source files, or use cloud model fallbacks.

## Current security boundary

This slice accepts locally trusted documents only. It uses immutable staging copies, shell-free
bounded subprocesses, source quote verification, inert Markdown rendering, and symlink-aware vault
paths. Poppler, Tesseract, and Pandoc are not yet executed inside a networkless OS sandbox, so
automatic public downloads remain disabled.

## Configure

Install Poppler, Tesseract, Pandoc, and `zk`, then ensure a local OpenAI-compatible model endpoint
is running on an explicit loopback address. On macOS with Homebrew:

```text
brew install poppler tesseract pandoc zk
```

Create `episteme.toml` from the packaged example or repository example. Replace every `/path/to`
value and tool path with an absolute local path. Set both model sections to existing
OpenAI-compatible endpoints bound to explicit loopback IP addresses. Episteme requires those
services to be managed externally. The example uses dedicated port `18181` and intentionally avoids
Ollama's conventional `11434` port.

## Use

```text
episteme --config episteme.toml doctor
episteme --config episteme.toml init
episteme --config episteme.toml classify "/path/inside/configured/inbox/document.pdf"
episteme --config episteme.toml ingest "/path/inside/configured/inbox/document.pdf"
episteme --config episteme.toml watch
```

`classify` extracts one inbox document, persists typed metadata in rebuildable DuckDB state, and
prints the stored record as JSON. It does not create a note, archive the source, or refresh `zk`.
Repeated classification of unchanged content reuses the digest-matched record.

`watch` requires two unchanged observations before processing a file. Manual and watched ingestion
use the same recoverable application service.

Research distillation automatically uses typed map/reduce for extracted documents over 12,000
characters. Rust assigns stable IDs to exact source spans; model calls select IDs rather than
generating quotes, and Rust reconstructs verbatim evidence and character locations. The final
aggregate must pass the same source-grounding validation before Episteme writes to the vault.

## Verify

```text
just ci
```

The ignored live-test guard can be run explicitly without touching the real vault:

```text
apfel --serve --host 127.0.0.1 --port 18181
just test-live-apfel
```

Apfel is managed separately and the recipe never uses or stops Ollama on port `11434`.

## Deferred roadmap

1. Private search across `zk` and msgvault
2. Decisions, commitments, claims, assumptions, and forecasts in the derived DuckDB index
3. Profile-scoped briefings and knowledge assurance
4. Governed automatic public research
5. Manifest-controlled Pandoc publication bundles
6. Dashboard, backup, restore, and declutter workflows
