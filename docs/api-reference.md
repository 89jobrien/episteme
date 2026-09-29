# API reference

Developer-facing reference for the `episteme` library crate. The user-facing CLI surface is
documented in [README.md](../README.md); release procedure is in
[release-runbook.md](release-runbook.md).

The package published to crates.io is `episteme-local`; the library and binary are both `episteme`
(see `Cargo.toml` `[package.name]` and `[lib.name]`).

## Contents

- [Module layout](#module-layout)
- [Ports](#ports) — traits every external integration implements
- [Domain types](#domain-types) — validated core types
- [Document intelligence](#document-intelligence) — the grounded graph
- [Classification](#classification)
- [Staging, ingestion, and watch](#staging,-ingestion,-and-watch)
- [Doctor](#doctor)
- [Configuration](#configuration)
- [CLI reference](#cli-reference)
- [Errors](#errors)

## Module layout

`src/lib.rs` exports eleven modules. `baml_client` is generated from `baml_src/` and must never be
edited by hand; it is excluded from the hand-written lint policy for that reason.

| Module           | Path                  | Responsibility                                  |
| ---------------- | --------------------- | ----------------------------------------------- |
| `adapters`       | `src/adapters/`       | Concrete implementations of the `ports` traits  |
| `baml_client`    | `src/baml_client/`    | Generated. Regenerate with `baml-cli generate`. |
| `classification` | `src/classification/` | Routing, caching policy, and batch execution    |
| `config`         | `src/config/`         | `Settings` loading and loopback enforcement     |
| `doctor`         | `src/doctor/`         | Configuration and dependency checks             |
| `domain`         | `src/domain/`         | Validated core types and their invariants       |
| `ingest`         | `src/ingest/`         | Recoverable ingestion application service       |
| `intelligence`   | `src/intelligence/`   | Intelligence pipeline orchestration             |
| `ports`          | `src/ports/`          | Hexagonal port traits and their error types     |
| `stage`          | `src/stage/`          | Immutable, symlink-aware source staging         |
| `watch`          | `src/watch/`          | Two-observation inbox poller                    |

The split is hexagonal: `ports` declares what the application needs, `adapters` supplies it, and the
middle modules orchestrate. `domain` depends on neither.

## Ports

Thirteen traits live in `src/ports/mod.rs`. Each is `Send + Sync`.

### Document conversion

```rust
#[async_trait]
pub trait DocumentTools: Send + Sync {
    async fn pdf_text(&self, source: &Path) -> Result<String, ExtractionError>;
    async fn pdf_ocr(&self, source: &Path) -> Result<String, ExtractionError>;
    async fn image_text(&self, source: &Path) -> Result<String, ExtractionError>;
    async fn html_text(&self, source: &Path) -> Result<String, ExtractionError>;
}
```

Primitive per-tool operations. `DocumentExtractor` sits above this and returns a complete
`ExtractedDocument` with the method recorded.

### Inference

| Trait                          | Method                                                   |
| ------------------------------ | -------------------------------------------------------- |
| `DocumentClassifier`           | `cache_keys(extracted_characters)`, `classify(document)` |
| `ProfileClassifier`            | `classify_with_profile(document, profile)`               |
| `ModelProbe`                   | `probe(profile)`                                         |
| `ResearchAnalyzer`             | `analyze(document)`                                      |
| `DocumentIntelligenceAnalyzer` | `analyze_intelligence(document, classification)`         |

`DocumentClassifier::cache_keys` returns **ordered** keys eligible for the given extracted size.
Order is significant — the routed classifier tries profiles in configuration order.

### Storage

| Trait                       | Methods                                                          | Backing implementation   |
| --------------------------- | ---------------------------------------------------------------- | ------------------------ |
| `VaultStore`                | `find_by_digest`, `create_note`, `archive_source`                | `adapters::vault_store`  |
| `IngestionStore`            | `load(digest)`, `save(run)`                                      | `adapters::duckdb_store` |
| `ClassificationStore`       | `load(digest, key)`, `save(key, record)`                         | `adapters::duckdb_store` |
| `ClassificationRunStore`    | `begin_batch`, `save_attempt`, `latest_attempts`, `finish_batch` | `adapters::duckdb_store` |
| `DocumentIntelligenceStore` | `load_intelligence`, `save_intelligence`                         | `adapters::duckdb_store` |

`VaultIndexer::index()` refreshes the external search index.

`ClassificationStore`, `ClassificationRunStore`, and `DocumentIntelligenceStore` each have a blanket
`impl for Arc<T>` so pipelines can hold a shared handle without generics at every call site.

**The Obsidian vault is canonical; DuckDB is rebuildable derived state.** Every store above except
`VaultStore` may be deleted and reconstructed. `create_note` is no-clobber: it returns
`VaultError::AlreadyExists` rather than overwriting.

## Domain types

`src/domain/` holds the validated core. Constructors enforce invariants; fields are public for
serialization but assume construction through the validating constructors.

### Identifiers

```rust
pub struct SourceDigest(String);      // 64 lowercase hex, BLAKE3
pub struct SourceFileName(String);    // rejects path escapes
```

```rust
SourceDigest::from_bytes(bytes: &[u8]) -> Self
SourceDigest::parse(value: impl Into<String>) -> Result<Self, DomainError>
SourceFileName::new(value: impl Into<String>) -> Result<Self, DomainError>
```

### Staging and extraction

```rust
pub struct StagedSource { /* private fields */ }
```

An immutable private copy of a source. Accessors: `path()`, `original_path()`, `digest()`,
`source_name()`, `kind()`. Built by `stage::stage_source` with `StagedSource::new` and
`with_original`.

```rust
pub struct ExtractedDocument { /* ... */ }
```

Accessors: `source()`, `text()`, `method()`. `ExtractionMethod` and `DocumentKind` are the two
deterministic enums recording how a document was read.

### Caching

```rust
pub struct ClassificationCacheKey { /* model, policy */ }
pub struct IntelligenceCacheKey { pub model: String, pub policy_version: String }
```

Policy versions are compile-time constants that change the meaning of a cached result:

```rust
pub const CLASSIFICATION_POLICY_VERSION: &str = "classification-v2";
pub const INTELLIGENCE_POLICY_VERSION: &str = "intelligence-v1";
```

Bump these when output shape or validation rules change, or stale cache entries will be reused
under a policy they do not satisfy.

## Document intelligence

The intelligence graph is the 0.2.0 feature. Its central guarantee: **the model selects evidence by
ID, Rust reconstructs the quote and validates it.** A model never authors a citation string.

### Grounding primitives

```rust
pub struct EvidenceReference {
    pub quote: String,     // verbatim excerpt from extracted text
    pub location: String,  // page, section, or deterministic text location
}
```

### Graph nodes and edges

```rust
pub struct IntelligenceSummary { pub text: String, pub key_points: Vec<String>, pub evidence: Vec<EvidenceReference> }

pub struct IntelligenceClaim { pub id: String, pub text: String, pub kind: ClaimKind, pub confidence_percent: u8, pub evidence: Vec<EvidenceReference> }

pub struct IntelligenceEntity { pub id: String, pub name: String, pub aliases: Vec<String>, pub kind: EntityKind, pub description: String, pub evidence: Vec<EvidenceReference> }

pub struct SemanticRelation { pub id: String, pub source_id: String, pub target_id: String, pub relation_type: SemanticRelationType, pub confidence_percent: u8, pub evidence: Vec<EvidenceReference> }
```

Constructors normalize whitespace, validate, and derive the ID:

```rust
IntelligenceClaim::new(text, kind, confidence_percent, evidence) -> Result<Self, DomainError>
IntelligenceEntity::new(name, kind, description, evidence) -> Result<Self, DomainError>
SemanticRelation::new(source_id, target_id, relation_type, confidence_percent, evidence, source_digest) -> Result<Self, DomainError>
```

`aliases` is initialized empty by `IntelligenceEntity::new`; it is not populated by the constructor.

### Stable IDs

```rust
IntelligenceClaim::stable_id(text: &str) -> String
IntelligenceEntity::stable_id(kind: EntityKind, name: &str) -> String
```

Both fold case and collapse whitespace before hashing with BLAKE3 under a namespace prefix
(`claim:`, the entity kind, `relation:`), so the same content yields the same ID across runs.

Relation IDs additionally include `source_id`, the relation type, `target_id`, and the source
digest — so a relation is distinct per source document.

### Enumerations

| Enum                   | Variants                                                                                                                |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `ClaimKind`            | `Fact`, `Inference`, `Recommendation`, `Critique`                                                                       |
| `EntityKind`           | `Person`, `Organization`, `Project`, `Technology`, `Concept`, `Method`, `Dataset`, `Benchmark`, `Document`              |
| `SemanticRelationType` | `Supports`, `Contradicts`, `Implements`, `Evaluates`, `DependsOn`, `Extends`, `Uses`, `Causes`, `PartOf`, `EvolvesFrom` |

All serialize `snake_case` and expose `as_str()`. `DependsOn` and `EvolvesFrom` are the two
multi-word cases worth noting.

### The complete graph

```rust
pub struct DocumentIntelligence {
    pub source_digest: SourceDigest,
    pub classification: DocumentClassification,
    pub summary: IntelligenceSummary,
    pub claims: Vec<IntelligenceClaim>,
    pub entities: Vec<IntelligenceEntity>,
    pub relations: Vec<SemanticRelation>,
    pub analysis: AnalysisProvenance,
}
```

```rust
DocumentIntelligence::validate_against(self, source: &str) -> Result<Self, DomainError>
```

This is the integrity gate. It enforces, in order:

1. The embedded classification normalizes.
2. The summary is non-empty, has at least one key point, and has evidence.
3. Claim and entity IDs are unique across **both** collections — a single `HashSet` covers both,
   so a claim and an entity cannot collide.
4. Every `source_id` and `target_id` on a relation resolves to a known node.
5. **Every evidence quote — on the summary, claims, entities, and relations alike — appears
   verbatim in the source text**, with non-empty `quote` and `location`.

Rule 5 is what makes the graph evidence-grounded rather than model-asserted. It is a literal
`source.contains(&item.quote)` check, so a model paraphrase cannot pass.

A failed check returns `DomainError::InvalidIntelligence`; nothing partial is returned.

### Provenance

```rust
pub struct AnalysisProvenance {
    pub function: String,        // BAML function responsible for the final draft
    pub client: String,          // configured local BAML client
    pub model: String,           // configured local model identifier
    pub pipeline_version: String,
    pub processed_at: String,    // RFC 3339, supplied by the application boundary
}
```

`processed_at` is supplied by the caller, not by the model or the clock inside the domain, which
keeps `domain` free of time and I/O dependencies.

## Classification

```rust
pub struct ClassificationPipeline<E, C, S>;  // extract -> classify -> persist
pub struct RoutedDocumentClassifier<C, P>;   // profile routing with fallback
pub struct ClassificationBatch<P, S>;        // bounded-concurrency batch execution
```

```rust
pub enum CachePolicy { Use, Refresh }
```

`--force` maps to `Refresh`; the default is `Use`. Under `Use`, a matching digest-plus-policy entry
short-circuits extraction and inference entirely.

```rust
pub struct BatchOptions { pub cache_policy: CachePolicy, pub retry_failed: bool }
```

`ClassifyBatch` reconciles one final status per source. Failures carry a typed code via
`ClassificationFailureCode`, split into retryable and terminal variants:

```rust
ClassificationFailure::retryable(code, message) -> Self
ClassificationFailure::terminal(code, message) -> Self
```

`ClassificationFailure` is a struct that also derives `Error` — it is both a returned value and an
error type:

```rust
pub struct ClassificationFailure {
    pub code: ClassificationFailureCode,
    pub message: String,               // bounded; no source content, paths, or backtraces
    pub retryable: bool,
    pub model: Option<String>,
    pub extracted_characters: Option<u64>,
}
```

Only a `retryable` attempt is eligible for `--retry-failed`. The `message` field is contractually
free of source content, response bodies, absolute paths, and stack backtraces; `code` is the
machine-readable category callers should branch on. Both properties are covered by
`classification_processor_redacts_extraction_paths`.

## Staging, ingestion, and watch

### Staging

```rust
pub fn stage_source(
    inbox_root: &Path,
    source: &Path,
    staging_root: &Path,
    maximum_source_bytes: u64,
) -> Result<StagedSource, StageError>
```

The entry point to every pipeline. Validates that `source` resolves beneath `inbox_root`, then
produces an immutable private copy under `staging_root`. Every rejection reason is a distinct
`StageError` variant, so callers can distinguish a path problem from an unsupported format:

| Variant               | Cause                                                |
| --------------------- | ---------------------------------------------------- |
| `Filesystem`          | A filesystem call failed.                            |
| `NotRegularFile`      | The path is not a regular file.                      |
| `OutsideInbox`        | The path escaped the configured inbox root.          |
| `InvalidFileName`     | The filename could escape or corrupt a trusted root. |
| `UnsupportedDocument` | The extension is not a supported input type.         |
| `SourceTooLarge`      | The file exceeded `maximum_source_bytes`.            |

Symlink handling is covered by `source_open_rejects_symlink_substitution`,
`staging_rejects_symlinked_parent_components`, and `staging_uses_an_immutable_private_copy`.

### Ingestion

```rust
pub struct Ingestor<E, A, V, S, I>;   // extractor, analyzer, vault, store, indexer
```

Coordinates one source through extraction, research analysis, vault persistence, and indexing. Like
the other pipelines it takes its ports as generic parameters, so a test can substitute all five.

```rust
pub enum IngestError { Extraction, Analysis, Vault, Store, Index }
```

Five unit variants, one per stage — the same content-free error discipline as `IntelligenceError`.

### Watch

```rust
pub enum WatchDecision {
    Ignore,           // outside policy, or not a regular file
    Pending,          // another unchanged observation is required
    Ready(PathBuf),   // stable and safe to enqueue
}
```

The two-observation rule from the README is implemented here: a file is `Pending` until it has been
seen unchanged twice, then becomes `Ready`. `watch_ignores_unstable_or_out_of_root_files` covers
the `Ignore` and `Pending` transitions.

```rust
pub struct StableFileTracker;   // internal fingerprint state
pub enum WatchError { Filesystem(..) }
```

### Doctor

```rust
pub struct RequiredTool { pub name: String, pub path: PathBuf }
pub struct ToolCheck    { pub name: String, pub path: PathBuf, pub available: bool }
pub struct DoctorReport { pub checks: Vec<ToolCheck> }

DoctorReport::is_healthy(&self) -> bool
Doctor::check_tools(tools: &[RequiredTool]) -> DoctorReport
```

`check_tools` is pure: it inspects the configured paths and returns a report without side effects.
`is_healthy` is true only when every check is available — `doctor_reports_missing_dependency`
exercises the negative case.

## Configuration

```rust
pub struct Settings {
    pub vault_root: PathBuf,
    pub inbox_directory: PathBuf,
    pub research_directory: PathBuf,
    pub archive_directory: PathBuf,
    pub database_path: PathBuf,
    pub tools: ToolPaths,
    pub classifier: ClassifierProfile,
    pub classifier_fallbacks: Vec<ClassifierProfile>,
    pub distiller: LocalModelEndpoint,
    pub minimum_text_characters: usize,
    pub process_timeout_seconds: u64,
    pub maximum_output_bytes: usize,
    pub maximum_source_bytes: u64,
    pub watch_interval_seconds: u64,
}
```

`ToolPaths` carries `pdftotext`, `pdftoppm`, `tesseract`, `pandoc`, and `zk`. `ClassifierProfile`
carries `name`, `endpoint: LocalModelEndpoint`, and `maximum_input_characters`.

`Settings::load` **rejects any non-loopback model endpoint** — the test
`settings_reject_non_loopback_model_endpoint` covers this. `classifier_fallbacks` are tried in
order, then the primary `classifier` is prepended when building the routed classifier
(`src/main.rs`, `analyze_path`).

## CLI reference

Verified against `episteme --help`. Global option: `--config <CONFIG>`, default `episteme.toml`.

| Command          | Purpose                                                        |
| ---------------- | -------------------------------------------------------------- |
| `doctor`         | Check configuration and required executables                   |
| `init`           | Create configured runtime directories                          |
| `ingest`         | Ingest one stable source from the configured inbox             |
| `classify`       | Classify one stable source                                     |
| `classify-batch` | Classify every supported source beneath the inbox              |
| `analyze`        | Extract evidence-grounded summary, claims, entities, relations |
| `watch`          | Poll the inbox and ingest stable files                         |

`classify-batch` options:

| Flag             | Default | Purpose                                         |
| ---------------- | ------- | ----------------------------------------------- |
| `--force`        | off     | Ignore matching versioned cache entries         |
| `--retry-failed` | off     | Retry sources whose latest attempt is retryable |
| `--concurrency`  | `2`     | Maximum concurrent local model requests         |
| `--batch-id`     | `inbox` | Durable batch identity for resume               |
| `--summary`      | none    | Path for one atomically reconciled JSON summary |

`classify` and `analyze` each take `<SOURCE>` and accept `--force`. Note that `analyze`'s `--force`
carries no help text in `--help` output — it is declared in `src/main.rs` without a doc comment,
unlike `classify --force`.

`ingest`, `classify`, `classify-batch`, and `analyze` all print JSON to stdout; `ingest` prints
`"{digest}: {stage}"` instead of a JSON object.

## Errors

Errors are `thiserror` enums. Per `AGENTS.md`, library code uses explicit typed errors and
`anyhow` appears only at the CLI boundary in `src/main.rs`.

| Error                   | Location         | Notes                                                                       |
| ----------------------- | ---------------- | --------------------------------------------------------------------------- |
| `DomainError`           | `domain`         | 9 variants. Constructors return these.                                      |
| `ClassificationFailure` | `domain`         | Struct deriving `Error`; not an enum.                                       |
| `ExtractionError`       | `ports`          | `Tool`, `Empty`, `Invalid`, `SourceChanged`.                                |
| `AnalysisError`         | `ports`          | `Model`, `Invalid`.                                                         |
| `VaultError`            | `ports`          | `AlreadyExists`, `Filesystem`, `Rendering`, `DigestMismatch`, `UnsafePath`. |
| `IntelligenceError`     | `intelligence`   | 4 variants, deliberately opaque.                                            |
| `ClassificationError`   | `classification` | Pipeline-level failure.                                                     |

Store errors are newtype structs wrapping `String`: `IngestionStoreError`,
`ClassificationStoreError`, `IntelligenceStoreError`, and `IndexError` — four of the seven error
types in `ports`. The other three are the `ExtractionError`, `AnalysisError`, and `VaultError`
enums above.

`IntelligenceError` collapses the four pipeline stages into opaque unit variants. That is
intentional — it keeps model and filesystem detail out of the error surface, consistent with the
sanitization applied to classification failures.
