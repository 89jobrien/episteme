# Changelog

## Unreleased

### Features

- Add native resumable batch classification with bounded concurrency, force refresh, retry controls,
  automatic local classifier fallback, durable attempt metrics, and atomic reconciled summaries.
- Add model-aware versioned classification caching and redirect-free loopback endpoint preflight.
- Add evidence-grounded document intelligence extraction with summaries, claims, canonical entities,
  typed semantic relations, stable graph IDs, and DuckDB persistence.

### Fixes

- Normalize document source types, human-language codes, authors, and topics while rejecting prompt
  leakage and schema placeholders.
- Preserve safe typed failure categories instead of storing generic errors, absolute paths, or stack
  backtraces.

## 0.1.1 - 2026-09-19

### Packaging

- Publish the package to crates.io as `episteme-local` while retaining `episteme` as the library
  and executable name.
- Add complete crates.io metadata and dual MIT/Apache-2.0 license texts.

## 0.1.0 - 2026-09-19

### Features

- Add a local-first document ingestion pipeline with guarded staging, deterministic extraction,
  typed local analysis, atomic vault writes, archival, indexing, and resumable provenance.
- Add standalone document classification with digest-based DuckDB persistence, JSON output, and
  hardened no-follow source staging.
- Add bounded chunked research distillation with typed BAML map/reduce, deterministic source-span
  evidence reconstruction, strict grounding validation, and direct and chunked live coverage.
