# Design: Resumable Classification Batches

## Goal

Provide trustworthy, resumable, model-aware batch classification with versioned caching, typed
failures, reconciled reporting, automatic local fallback routing, and normalized metadata.

## Approved Approach

Use DuckDB as the sole batch-state authority and route documents through an ordered, explicitly
bounded chain of loopback-only classifier profiles.

## Context Map

### Files to Modify

| File                            | Purpose                         | Changes Needed                                                                 |
| ------------------------------- | ------------------------------- | ------------------------------------------------------------------------------ |
| `Cargo.toml`                    | Dependency and package metadata | Add direct `reqwest` dependency for local model probes                         |
| `baml_src/research.baml`        | Typed classification schema     | Add canonical source-type enum and explicit human-language contract            |
| `src/domain/mod.rs`             | Validated domain values         | Add canonical classification, cache, batch, attempt, status, and failure types |
| `src/config/mod.rs`             | Runtime policy                  | Parse primary and ordered fallback classifier profiles with input limits       |
| `src/ports/mod.rs`              | Hexagonal boundaries            | Make classifier outcomes provenance-aware and add probe/run-ledger ports       |
| `src/classification/mod.rs`     | Single-item application service | Add force policy, versioned cache lookup, normalization, and typed failures    |
| `src/classification/batch.rs`   | Batch application service       | Add bounded, resumable, reconciled batch orchestration                         |
| `src/adapters/baml_analyzer.rs` | Local BAML adapter              | Preserve safe failure categories and normalize generated classifications       |
| `src/adapters/duckdb_store.rs`  | Derived-state adapter           | Store versioned results, batches, and attempts                                 |
| `src/adapters/model_probe.rs`   | Local endpoint adapter          | Probe loopback `/models` endpoints without redirects                           |
| `src/adapters/mod.rs`           | Adapter exports                 | Export model probe and routed classifier adapters                              |
| `src/main.rs`                   | CLI composition                 | Add `classify-batch`, `--force`, concise top-level errors, and JSON export     |
| `src/lib.rs`                    | Public modules                  | Export batch classification service                                            |
| `episteme.toml.example`         | Operator configuration          | Document primary and fallback profile limits                                   |
| `README.md`                     | User documentation              | Document batch, retry, force, routing, and summary behavior                    |

### Files to Add

| File                                        | Purpose                                                            |
| ------------------------------------------- | ------------------------------------------------------------------ |
| `migrations/003_classification_batches.sql` | Versioned classification cache and batch ledger schema             |
| `tests/classification_batch.rs`             | Batch resume, retry, metrics, relative paths, and summary coverage |
| `tests/model_probe.rs`                      | Loopback probe behavior with a local test server                   |

### Dependencies

| File                                 | Relationship                                                             |
| ------------------------------------ | ------------------------------------------------------------------------ |
| `src/stage/mod.rs`                   | Safely stages each discovered inbox source                               |
| `src/adapters/document_extractor.rs` | Supplies deterministic text and character counts                         |
| `src/watch/mod.rs`                   | Existing ingestion watcher remains independent                           |
| `tests/classification.rs`            | Existing cache behavior evolves to include model/policy keys             |
| `tests/duckdb.rs`                    | Existing single-record round trip expands to history and ledger coverage |
| `tests/classification_cli.rs`        | Existing command help coverage expands to batch flags                    |

### Reference Patterns

| File                            | Pattern to Follow                                                      |
| ------------------------------- | ---------------------------------------------------------------------- |
| `src/ingest/mod.rs`             | Generic application service over narrow ports with durable checkpoints |
| `src/classification/mod.rs`     | Existing extraction, validation, persistence, and provenance flow      |
| `src/adapters/duckdb_store.rs`  | Mutex-protected DuckDB access and domain reconstruction                |
| `src/adapters/baml_analyzer.rs` | Loopback-only BAML environment injection and content-free errors       |

### Risk

- `DocumentClassification.source_type` changes from free text to a canonical enum.
- `DocumentClassifier` returns classification plus selected-model provenance and typed failures.
- Existing digest-only rows remain readable for audit but are never accepted as v2 cache hits.
- JSON CLI output gains stable status/error fields and no longer emits absolute local source paths.
- One new network dependency is permitted only behind a loopback-validating probe adapter.

## Crate Ownership

- **Owner crate**: `episteme-local` -- batch classification composes existing staging, extraction,
  inference, and DuckDB capabilities.
- **Affected crates**: none; this repository contains one package.

## BAML API

```baml
enum DocumentSourceType {
  ResearchPaper
  Report
  Article
  Documentation
  Website
  SourceCode
  Repository
  Specification
  Tutorial
  PersonalProfile
  Other
}

class DocumentClassification {
  title string
  authors string[]
  source_type DocumentSourceType
  language_code string
  topics string[]
}
```

`ClassifyDocument` instructs the model to return an ISO 639-1 human-language code, classify source
type only through the enum, use `Other` when uncertain, and never emit schema words, prompt labels,
or programming languages as metadata values.

## Public API

### Domain Types

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentSourceType {
    ResearchPaper,
    Report,
    Article,
    Documentation,
    Website,
    SourceCode,
    Repository,
    Specification,
    Tutorial,
    PersonalProfile,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentClassification {
    pub title: String,
    pub authors: Vec<String>,
    pub source_type: DocumentSourceType,
    pub language_code: String,
    pub topics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClassificationCacheKey {
    pub model: String,
    pub policy_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassificationOutcome {
    pub classification: DocumentClassification,
    pub analysis: AnalysisProvenance,
    pub extracted_characters: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationFailureCode {
    Staging,
    Extraction,
    EndpointUnavailable,
    ModelUnavailable,
    InputTooLarge,
    ContextOverflow,
    ModelRequest,
    InvalidOutput,
    Persistence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[error("{message}")]
pub struct ClassificationFailure {
    pub code: ClassificationFailureCode,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelAvailability {
    pub advertised_models: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationAttemptStatus {
    Pending,
    Running,
    Succeeded,
    FailedRetryable,
    FailedTerminal,
    Cached,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassificationAttemptRecord {
    pub batch_id: String,
    pub source_path: String,
    pub source_digest: Option<SourceDigest>,
    pub attempt: u32,
    pub status: ClassificationAttemptStatus,
    pub model: Option<String>,
    pub extracted_characters: Option<u64>,
    pub duration_ms: u64,
    pub failure_code: Option<ClassificationFailureCode>,
    pub failure_message: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassificationBatchSource {
    Ready {
        relative_path: String,
        source: StagedSource,
    },
    Rejected {
        relative_path: String,
        failure: ClassificationFailure,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassificationBatchSummary {
    pub batch_id: String,
    pub total: usize,
    pub succeeded: usize,
    pub cached: usize,
    pub failed_retryable: usize,
    pub failed_terminal: usize,
    pub attempts: Vec<ClassificationAttemptRecord>,
}
```

### Configuration

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifierProfile {
    pub name: String,
    pub endpoint: LocalModelEndpoint,
    pub maximum_input_characters: usize,
}

pub struct Settings {
    pub classifier: ClassifierProfile,
    pub classifier_fallbacks: Vec<ClassifierProfile>,
    // existing fields unchanged
}
```

Existing `[classifier]` configurations remain valid. Missing `name` defaults to `primary`; missing
`maximum_input_characters` defaults to unlimited for backward compatibility. Optional
`[[classifier_fallbacks]]` entries require both fields explicitly.

### Ports

```rust
#[async_trait]
pub trait DocumentClassifier: Send + Sync {
    async fn classify(
        &self,
        document: &ExtractedDocument,
    ) -> Result<ClassificationOutcome, ClassificationFailure>;
}

#[async_trait]
pub trait ModelProbe: Send + Sync {
    async fn probe(
        &self,
        profile: &ClassifierProfile,
    ) -> Result<ModelAvailability, ClassificationFailure>;
}

pub trait ClassificationStore: Send + Sync {
    fn load(
        &self,
        digest: &SourceDigest,
        key: &ClassificationCacheKey,
    ) -> Result<Option<ClassificationRecord>, ClassificationStoreError>;

    fn save(
        &self,
        key: &ClassificationCacheKey,
        record: &ClassificationRecord,
    ) -> Result<(), ClassificationStoreError>;
}

pub trait ClassificationRunStore: Send + Sync {
    fn begin_batch(&self, batch_id: &str, started_at: &str)
        -> Result<(), ClassificationStoreError>;
    fn save_attempt(
        &self,
        attempt: &ClassificationAttemptRecord,
    ) -> Result<(), ClassificationStoreError>;
    fn latest_attempts(
        &self,
        batch_id: &str,
    ) -> Result<Vec<ClassificationAttemptRecord>, ClassificationStoreError>;
    fn finish_batch(&self, summary: &ClassificationBatchSummary)
        -> Result<(), ClassificationStoreError>;
}
```

### Application Services

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePolicy {
    Use,
    Refresh,
}

impl<E, C, S> ClassificationPipeline<E, C, S> {
    pub async fn classify(
        &self,
        source: &StagedSource,
        cache_policy: CachePolicy,
    ) -> Result<ClassificationRecord, ClassificationError>;
}

#[derive(Debug)]
pub struct ClassificationBatch<E, C, S> {
    extractor: E,
    classifier: C,
    store: S,
    maximum_concurrency: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchOptions {
    pub cache_policy: CachePolicy,
    pub retry_failed: bool,
}

impl<E, C, S> ClassificationBatch<E, C, S> {
    pub async fn run(
        &self,
        batch_id: &str,
        sources: Vec<ClassificationBatchSource>,
        options: BatchOptions,
    ) -> Result<ClassificationBatchSummary, ClassificationError>;
}
```

## Typed Failure Contract

`ClassificationFailure` exposes typed code, retryability, selected model, extracted character count,
and a bounded content-free message. BAML adapter errors retain safe context-overflow and endpoint
categories but never serialize prompts, extracted content, response bodies, stack traces, or
absolute paths.

Retryable codes: endpoint unavailable, model unavailable, context overflow when a later eligible
profile exists, and model request failures. Extraction, invalid output after all profiles, staging,
and persistence are terminal.

## Routing and Preflight

1. Extract once and count Unicode scalar values.
2. Select profiles whose configured maximum accepts the document, preserving configuration order.
3. Probe each selected profile's loopback `/models` endpoint with redirects disabled, bounded body
   size, strict JSON parsing, and a bounded timeout; require the configured model to be advertised.
4. Attempt classification. Context overflow or retryable transport failure advances to the next
   eligible profile. Invalid normalized output advances once, then becomes terminal after profiles
   are exhausted.
5. When no profile accepts the full source, build a deterministic bounded head/tail excerpt for the
   largest configured profile so metadata extraction remains possible for book-scale documents.
6. Return the selected profile provenance with the validated classification.

## Persistence

`003_classification_batches.sql` adds:

- `document_classification_versions`: composite primary key `(source_digest, analysis_model,
policy_version)` with normalized metadata and full provenance.
- `classification_batches`: batch identity, start/finish timestamps, and final counters.
- `classification_attempts`: one row per `(batch_id, source_path, attempt)` with status, selected
  model, metrics, and safe typed failure.

Legacy `document_classifications` remains immutable audit history. Valid rows migrate into the v2
table under policy `legacy-v1`, but v2 requests never use them as cache hits.

## CLI

```text
episteme classify [--force] <source>
episteme classify-batch [--force] [--retry-failed] [--concurrency <1..5>]
                        [--summary <path>]
```

Batch discovery recursively walks the configured inbox without following symlinks, stages sources
through existing safety checks, sorts by vault-relative path, and stores only that relative path.
The optional summary is regenerated from latest DuckDB attempts after completion and written
atomically with no-clobber semantics as one reconciled JSON document.

`main` delegates to `run() -> anyhow::Result<()>`, prints the compact error chain with `Display`,
and returns an explicit non-zero `ExitCode`; stack backtraces are never included in batch records.

## Data Flow

1. Discover safe inbox-relative paths and open one DuckDB batch.
2. Stage each source, recording typed staging failures without halting unrelated documents.
3. Extract once, capture character count, route through preflighted classifier profiles, normalize,
   and validate.
4. Use the versioned cache unless refresh is requested; save every model-specific successful result.
5. Save each attempt immediately, reconcile latest status from DuckDB, finish the batch, and
   optionally export one atomic JSON summary.

## Hexagonal Boundaries

- **Inference port**: `DocumentClassifier`; adapter: routed `BamlDocumentClassifier` chain.
- **Health port**: `ModelProbe`; adapter: `HttpModelProbe` with loopback-only no-redirect client.
- **Cache port**: versioned `ClassificationStore`; adapter: `DuckDbIngestionStore`.
- **Ledger port**: `ClassificationRunStore`; adapter: `DuckDbIngestionStore`.
- **Application services**: `ClassificationPipeline` and `ClassificationBatch`.

## Out of Scope

- Semantic document/entity graph extraction
- Automatic model installation or server lifecycle management
- Distributed workers or cross-machine batch coordination
- Retrying arbitrary terminal extraction and validation defects forever
- Deleting legacy classification history

## Risk

- [x] Breaking API changes: yes -- classification domain and port signatures change
- [x] New external dependency: yes -- `reqwest`, isolated behind `ModelProbe`
- [x] Serialization change: yes -- canonical source type and `language_code`
- [x] Database migration: additive, legacy table retained
- [x] Feature flag required: no
