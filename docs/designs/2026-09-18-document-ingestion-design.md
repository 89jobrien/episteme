# Design: Local Document Ingestion

## Goal

Convert stable private documents into source-grounded Obsidian research drafts without cloud model
access, destructive overwrites, or authoritative database state.

## Ownership

- Episteme owns orchestration, policy, BAML schemas, and derived DuckDB state.
- The private Obsidian vault owns canonical notes and archived originals.
- Configured local OpenAI-compatible services own inference runtime and model lifecycle.

## Data flow

1. Validate a regular non-symlink source beneath the configured inbox.
2. Compute its BLAKE3 content identity.
3. Extract text through Poppler, Pandoc, or Tesseract with deterministic OCR fallback.
4. Classify and distill through typed BAML functions using loopback endpoints only.
5. Validate required prose and verbatim evidence.
6. Create an atomic no-clobber research note.
7. Verify and archive an immutable source copy; retain the inbox file for explicit cleanup.
8. Persist rebuildable progress in DuckDB and refresh `zk`.

## Failure model

Each durable stage is persisted. Retrying discovers an existing note by source digest and resumes
archival or indexing without repeating extraction, inference, or note creation. DuckDB does not
override filesystem truth.

## Out of scope

- Public downloads and web research
- Cloud inference fallback
- Automatic publication or deletion
- Model installation and process management
- Pueue or Pipelight orchestration
