# Architecture

Cross-cutting structure of `episteme`: how the pieces fit, what owns what, and which invariants
hold across every feature.

For the per-feature rationale, see the design documents in [`designs/`](designs/) — this document
deliberately does not repeat them:

| Design                                                                                                          | Covers                                      |
| --------------------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| [Local Document Ingestion](designs/2026-09-18-document-ingestion-design.md)                                     | Ingest pipeline and ownership boundaries    |
| [Standalone Document Classification](designs/2026-09-19-standalone-document-classification-design.md)           | `classify` as an independent service        |
| [Resumable Classification Batches](designs/2026-09-19-resumable-classification-batches-design.md)               | `classify-batch` state authority            |
| [Chunked Research Distillation](designs/2026-09-19-chunked-research-distillation-design.md)                     | Typed map/reduce over long documents        |
| [Evidence-Grounded Document Intelligence](designs/2026-09-19-evidence-grounded-document-intelligence-design.md) | The summary/claims/entities/relations graph |

The public API is in [api-reference.md](api-reference.md). Release procedure is in
[release-runbook.md](release-runbook.md).

## Contents

- [Shape](#shape)
- [Canonical versus derived state](#canonical-versus-derived-state)
- [Data flow](#data-flow)
- [Trust boundary](#trust-boundary)
- [Model interaction](#model-interaction)
- [Caching](#caching)
- [Error handling](#error-handling)
- [Extending](#extending)

## Shape

Episteme is a single crate, `episteme-local`, structured hexagonally. The dependency direction is
strict and one-way:

```text
        ┌──────────────────────────────────────────────┐
        │  src/main.rs        CLI boundary, anyhow      │
        └──────────────────────┬───────────────────────┘
                               │
        ┌──────────────────────▼───────────────────────┐
        │  orchestration                                 │
        │  ingest · classification · intelligence · watch │
        │  domain (no I/O, no clock, no network)         │
        └──────────────────────┬───────────────────────┘
                               │ depends only on traits
        ┌──────────────────────▼───────────────────────┐
        │  src/ports        13 traits, 7 error types     │
        └──────────────────────▲───────────────────────┘
                               │ implements
        ┌──────────────────────┴───────────────────────┐
        │  src/adapters    DuckDB · vault · poppler ·   │
        │                  tesseract · pandoc · zk ·    │
        │                  loopback HTTP                │
        └──────────────────────────────────────────────┘
```

`domain` depends on no adapter, no port, and no I/O. It is the only module that both orchestration
layers and adapters may reference. Everything crossing into it is validated on the way in, which is
why `StagedSource` and `ExtractedDocument` expose accessors rather than public fields — a value
that exists has already passed its invariants.

## Canonical versus derived state

This is the single most important structural decision in the project.

| Store                  | Status        | Consequence                                      |
| ---------------------- | ------------- | ------------------------------------------------ |
| The Obsidian vault     | **Canonical** | Never overwritten. No-clobber writes.            |
| Immutable archive area | **Canonical** | Verbatim originals, digest-verified on archival. |
| DuckDB                 | Rebuildable   | May be deleted and reconstructed at any time.    |

Everything in DuckDB is derived: ingestion progress, classifications, batch state, and the
intelligence graph. Deleting the database loses no source content, because the vault retains both
the rendered notes and the archived originals. The reverse is not true — a note exists only
because Episteme wrote it, and nothing regenerates it from DuckDB.

The schema grows by migration, one file per feature:

| Migration                        | Introduces                                                                                                                               |
| -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `001_ingestion.sql`              | `ingestion_runs`                                                                                                                         |
| `002_classification.sql`         | `document_classifications`                                                                                                               |
| `003_classification_batches.sql` | `document_classification_versions`, `classification_batches`, `classification_attempts`                                                  |
| `004_document_intelligence.sql`  | `document_intelligence_versions`, `intelligence_claims`, `intelligence_entities`, `document_intelligence_entities`, `semantic_relations` |

The intelligence tables are normalized — claims and entities are stored once and joined through
`document_intelligence_entities`, so a shared entity is not duplicated per document.

## Data flow

Every command shares one spine. The three model-facing commands differ only in which stage they
stop at.

```text
  inbox file
      │
      ▼
  ┌────────────────────────────────────────────┐
  │ stage        validate, copy to private      │  symlink-aware, size-capped,
  │              immutable copy, BLAKE3 digest  │  rejects unsafe filenames
  └────────────────────┬───────────────────────┘
                       │ StagedSource
      ┌────────────────┴────────────────┐
      │                                 │
      ▼                                 ▼
  classify / classify-batch          ingest / watch
      │                                 │
      │  DocumentClassification          ├── ResearchAnalyzer (typed BAML)
      │                                 │     └─ map/reduce over source spans
      ▼                                 │     └─ chunked past 12,000 chars
  DocumentIntelligenceStore             │     └─ ResearchDraft
      │                                 │            │
      │  (analyze)                       │            ▼
      ▼                            VaultStore
  DocumentIntelligence ────────────────▶  note (atomic, no-clobber)
  summary · claims · entities          archive original (digest-verified)
  · relations
```

`analyze` runs the full spine through the intelligence stage. `classify` stops after
classification. `ingest` and `watch` run extraction, research analysis, and the vault write, but
never touch the classification or intelligence stores.

Caching short-circuits the model path but not validation: a digest-plus-policy cache hit returns the
stored record without extraction or inference, while a miss runs the full spine.

## Trust boundary

The project's operating constraint is that private content and model requests never leave the
machine. Four mechanisms enforce it.

**Network.** `Settings::load` rejects any model endpoint that is not an explicit loopback address;
there is no cloud fallback path. `HttpModelProbe` preflights the endpoint and refuses redirects, so
a loopback target cannot be redirected off-host.

**Process execution.** External tools are invoked with fixed argument arrays, never through a
shell, and are bounded by both a timeout and a maximum output size. Document content reaches a
subprocess only as an argument, never as shell syntax. The execution helper is a private module
inside `src/adapters/`; it is not part of the public API.

**Staging.** Sources are validated and copied to an immutable private copy before anything reads
them. `stage` rejects symlinks, path escapes, oversized files, and unsafe filenames, and records
identity including ctime so a substituted file is detected rather than silently read.

**Grounding.** Generated content is never trusted. Models select evidence by identifier; Rust
reconstructs the quote from extracted text and verifies it appears verbatim
(`DocumentIntelligence::validate_against`, `ResearchDraft::validate`). A model paraphrase cannot
become a citation. `Episteme` never publishes, deletes user content, or overwrites existing notes.

Known limit, stated plainly: Poppler, Tesseract, and Pandoc are not yet executed inside a
networkless OS sandbox, so automatic public downloads remain disabled. The boundary is enforced at
the application layer, not below the process boundary.

## Model interaction

Typed BAML functions in `baml_src/` — `clients.baml` (loopback endpoint definitions),
`generators.baml`, and `research.baml` — generate `src/baml_client/`, which is committed but must
never be hand-edited. Regenerate with `baml-cli generate` after any schema change; the generated
module is exempt from the hand-written lint policy for that reason.

Two patterns recur, and they are the reason grounding is trustworthy:

**Map/reduce for long documents.** Past 12,000 characters a document exceeds the local context
window, so `DistillResearchChunk` maps bounded chunks to typed findings and
`AggregateResearchChunks` reduces them. Rust assigns stable IDs to exact source spans up front;
model calls return IDs, never free text.

**Deterministic validation after generation.** Every generated artifact is re-validated in Rust
after the model returns. Normalization, deduplication, stable ID generation, and referential
integrity all happen outside the model, so a model cannot talk the system into accepting ungrounded
output.

The general rule: **the model proposes, Rust disposes.**

## Caching

Two independent caches, both keyed on content digest plus a policy version.

| Cache          | Key                      | Policy constant                                       |
| -------------- | ------------------------ | ----------------------------------------------------- |
| Classification | `ClassificationCacheKey` | `CLASSIFICATION_POLICY_VERSION` = `classification-v2` |
| Intelligence   | `IntelligenceCacheKey`   | `INTELLIGENCE_POLICY_VERSION` = `intelligence-v1`     |

The policy version is a compile-time constant that changes the _meaning_ of a cached result.
Bump it whenever output shape or validation rules change. Without a bump, entries written under the
old rules are reused as though they satisfied the new ones — the cache will silently serve results
the current validator would reject.

`CachePolicy::Use` reads; `CachePolicy::Refresh` (the `--force` flag) bypasses. Batch classification
adds `document_classification_versions` so a refresh does not destroy the prior entry.

## Error handling

Typed `thiserror` errors in the library; `anyhow` only at the CLI boundary in `src/main.rs`.

The deliberate choice is that generated and infrastructure detail is kept _out_ of error surfaces.
`IntelligenceError` collapses four pipeline stages into four opaque unit variants.
`ClassificationFailure` is content-free by contract: a typed `code` for branching, a bounded
`message`, and no paths, response bodies, or backtraces. Both exist so that a failure can be logged
or persisted without leaking the private document it came from.

## Extending

Add an integration by implementing the narrowest trait that fits, in `src/adapters/`, and re-export
it from `src/adapters/mod.rs`. The orchestration modules take adapters as generic parameters
(`DocumentIntelligencePipeline<E, C, A, S>`), so a new implementation needs no change above it and
tests can substitute a stub without a network or filesystem.

Three store traits carry a blanket `impl for Arc<T>` precisely so callers can share one handle
without threading generics through every signature.
