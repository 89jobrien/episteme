# Release v0.1.0

## What's New

- Episteme can ingest PDF, HTML, and image sources into a private Obsidian research workflow while
  keeping model requests and document content on this Mac.
- The new `classify <path>` command extracts document metadata independently and stores reusable
  results locally without creating notes or moving source files.
- Large documents are processed through bounded chunking and aggregation, allowing local models to
  handle sources that exceed a single request's context window.

## Improvements

- Research evidence is now reconstructed from validated source-span IDs, ensuring saved quotes and
  character locations come directly from the original extracted text.
- Digest-based recovery prevents duplicate output and lets interrupted ingestion resume safely.
- Source staging rejects symlink substitutions and preserves immutable private copies throughout
  processing.
