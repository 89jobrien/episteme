# Design: Standalone Document Classification

## Goal

Classify a staged inbox document independently of research distillation, persist the typed result in
rebuildable DuckDB state, and return the persisted record as JSON without writing or archiving vault
content.

## Approved Approach

Use a dedicated classification service with narrow classifier and persistence ports, digest-based
idempotency, and a `classify <path>` CLI command.

## Context Map

### Files to Modify

| File                            | Purpose                  | Changes Needed                                                 |
| ------------------------------- | ------------------------ | -------------------------------------------------------------- |
| `src/domain/mod.rs`             | Validated domain data    | Add classification and persisted-record types                  |
| `src/ports/mod.rs`              | Hexagonal boundaries     | Add classifier and classification-store ports                  |
| `src/adapters/baml_analyzer.rs` | Local BAML inference     | Add the classifier adapter and reuse it from research analysis |
| `src/adapters/duckdb_store.rs`  | Rebuildable DuckDB state | Persist and load classifications                               |
| `src/adapters/mod.rs`           | Adapter exports          | Export the classifier adapter                                  |
| `src/lib.rs`                    | Library module exports   | Export the classification service module                       |
| `src/main.rs`                   | CLI composition root     | Add and compose `classify <path>`                              |
| `tests/duckdb.rs`               | DuckDB adapter coverage  | Verify classification round trips and upserts                  |
| `tests/classification.rs`       | Service coverage         | Verify persistence, cache reuse, and failure behavior          |

### Files to Add

| File                                | Purpose                                          |
| ----------------------------------- | ------------------------------------------------ |
| `src/classification/mod.rs`         | Side-effect-limited classification orchestration |
| `migrations/002_classification.sql` | Idempotent classification schema                 |

### Dependencies

| File                                 | Relationship                                                       |
| ------------------------------------ | ------------------------------------------------------------------ |
| `src/ingest/mod.rs`                  | Continues consuming `ResearchAnalyzer`; behavior remains unchanged |
| `src/stage/mod.rs`                   | Supplies safe immutable staged sources to both pipelines           |
| `src/adapters/document_extractor.rs` | Supplies deterministic extracted text to both pipelines            |
| `baml_src/research.baml`             | Already defines `ClassifyDocument`; no schema change required      |

### Reference Patterns

| File                           | Pattern to Follow                                          |
| ------------------------------ | ---------------------------------------------------------- |
| `src/ingest/mod.rs`            | Generic application service over narrow ports              |
| `src/adapters/duckdb_store.rs` | Mutex-protected connection and typed record reconstruction |
| `migrations/001_ingestion.sql` | Idempotent rebuildable-state migration                     |

### Risk

- No existing public signature changes; `BamlResearchAnalyzer::new` remains compatible.
- The database schema is additive and idempotent.
- Classification output becomes a public serialized shape and must remain stable.
- No new crate or external dependency is required.

## Crate Ownership

- **Owner crate**: `episteme` -- classification is part of the existing document intelligence
  domain and reuses its staging, extraction, BAML, and DuckDB adapters.
- **Affected crates**: none; this repository contains one package.

## Public API

### Traits

```rust
#[async_trait]
pub trait DocumentClassifier: Send + Sync {
    async fn classify(
        &self,
        document: &ExtractedDocument,
    ) -> Result<DocumentClassification, AnalysisError>;
}

pub trait ClassificationStore: Send + Sync {
    fn load(
        &self,
        digest: &SourceDigest,
    ) -> Result<Option<ClassificationRecord>, ClassificationStoreError>;

    fn save(
        &self,
        record: &ClassificationRecord,
    ) -> Result<(), ClassificationStoreError>;
}
```

### Types

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentClassification {
    pub title: String,
    pub authors: Vec<String>,
    pub source_type: String,
    pub language: String,
    pub topics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassificationRecord {
    pub source_digest: SourceDigest,
    pub source_name: SourceFileName,
    pub extraction_method: ExtractionMethod,
    pub classification: DocumentClassification,
    pub analysis: AnalysisProvenance,
}

#[derive(Debug)]
pub struct ClassificationPipeline<E, C, S> {
    extractor: E,
    classifier: C,
    store: S,
}

#[derive(Debug, Clone)]
pub struct BamlDocumentClassifier {
    endpoint: LocalModelEndpoint,
    timeout: Duration,
}
```

### Functions

```rust
impl DocumentClassification {
    pub fn validate(self) -> Result<Self, DomainError>;
}

impl<E, C, S> ClassificationPipeline<E, C, S> {
    pub const fn new(extractor: E, classifier: C, store: S) -> Self;
}

impl<E, C, S> ClassificationPipeline<E, C, S>
where
    E: DocumentExtractor,
    C: DocumentClassifier,
    S: ClassificationStore,
{
    pub async fn classify(
        &self,
        source: &StagedSource,
        provenance: AnalysisProvenance,
    ) -> Result<ClassificationRecord, ClassificationError>;
}

impl BamlDocumentClassifier {
    pub const fn new(endpoint: LocalModelEndpoint, timeout: Duration) -> Self;
}
```

## Data Flow

1. The CLI validates and stages a source beneath the configured inbox using `stage_source`.
2. `ClassificationPipeline` returns an existing record when the source digest is already stored.
3. Otherwise, `DocumentExtractor` produces deterministic text and extraction provenance.
4. `BamlDocumentClassifier` calls the existing typed `ClassifyDocument` BAML function locally.
5. The domain classification is validated, combined with source and model provenance, and upserted
   through `ClassificationStore`.
6. The CLI serializes the returned `ClassificationRecord` as pretty JSON to stdout.

## Hexagonal Boundaries

- **Classifier port**: `DocumentClassifier` in `episteme::ports`.
- **Classifier adapter**: `BamlDocumentClassifier` in `episteme::adapters`.
- **Persistence port**: `ClassificationStore` in `episteme::ports`.
- **Persistence adapter**: `DuckDbIngestionStore` in `episteme::adapters`, implementing both
  ingestion and classification storage against the same rebuildable database.
- **Application service**: `ClassificationPipeline` in `episteme::classification`.

## Persistence

`document_classifications` uses `source_digest` as its primary key. Scalar classification fields
remain directly queryable; `authors` and `topics` are encoded as JSON arrays in text columns. The
record also stores extraction method and complete local-analysis provenance. Re-running
classification for an unchanged digest returns the stored record without inference. Refresh and
model-version invalidation are intentionally deferred.

## Out of Scope

- Distillation, research-note creation, source archival, or `zk` indexing
- Recursive directory classification or automatic classification watching
- Classification refresh or `--force`
- Markdown extraction support
- Changes to the existing BAML schema or generated client code

## Risk

- [x] Breaking API changes: no
- [x] New external dependency: no
- [x] Feature flag required: no
- [x] Canonical vault mutation: no; classification writes only rebuildable DuckDB state
