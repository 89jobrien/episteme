# Design: Evidence-Grounded Document Intelligence

## Goal

Extract reusable summaries, claims, entities, and typed semantic relations from local documents,
ground every factual graph artifact in exact source spans, and persist a versioned rebuildable graph
in DuckDB.

## Approved Approach

Use typed BAML map/reduce over deterministic source spans, followed by Rust validation,
canonicalization, deduplication, stable ID generation, and DuckDB graph persistence.

## Context Map

### Files to Modify

| File                            | Purpose                       | Changes Needed                                                                         |
| ------------------------------- | ----------------------------- | -------------------------------------------------------------------------------------- |
| `baml_src/research.baml`        | Typed local analysis          | Add claim/entity/relation enums, chunk output, aggregate output, and two functions     |
| `src/domain/mod.rs`             | Validated intelligence values | Add summaries, claims, entities, graph nodes, relations, evidence, IDs, and provenance |
| `src/ports/mod.rs`              | Hexagonal boundaries          | Add document intelligence analyzer and persistence ports                               |
| `src/adapters/baml_analyzer.rs` | BAML inference                | Reuse source spans, run bounded map/reduce, namespace local IDs, validate references   |
| `src/adapters/duckdb_store.rs`  | Derived graph persistence     | Save and load versioned intelligence and graph records                                 |
| `src/main.rs`                   | CLI composition               | Add `analyze`, `analyze-batch`, force/retry/concurrency/summary options                |
| `src/lib.rs`                    | Module exports                | Export intelligence services                                                           |
| `README.md`                     | User documentation            | Document intelligence output and graph-query examples                                  |
| `CHANGELOG.md`                  | Release history               | Record the new evidence-grounded intelligence pipeline                                 |

### Files to Add

| File                                       | Purpose                                                        |
| ------------------------------------------ | -------------------------------------------------------------- |
| `src/intelligence/mod.rs`                  | Single-document intelligence service                           |
| `src/intelligence/batch.rs`                | Resumable bounded batch service                                |
| `src/intelligence/spans.rs`                | Shared deterministic span construction and evidence resolution |
| `migrations/004_document_intelligence.sql` | Versioned intelligence, graph, and batch schema                |
| `tests/intelligence.rs`                    | Domain, canonicalization, grounding, and service tests         |
| `tests/intelligence_batch.rs`              | Resume, retry, cache, and summary tests                        |

### Dependencies

| File                                 | Relationship                                                              |
| ------------------------------------ | ------------------------------------------------------------------------- |
| `src/classification/mod.rs`          | Supplies normalized document classification and selected model provenance |
| `src/classification/batch.rs`        | Reference pattern for durable bounded batch orchestration                 |
| `src/adapters/document_extractor.rs` | Produces deterministic extracted source text                              |
| `src/domain/mod.rs`                  | Existing `EvidenceReference` and source digest types remain canonical     |
| `src/stage/mod.rs`                   | Existing no-follow staging remains the only source entry point            |

### Test Coverage

| Existing test                              | Reused guarantee                                    |
| ------------------------------------------ | --------------------------------------------------- |
| `tests/analysis.rs`                        | Rejects ungrounded quotes                           |
| `tests/classification_batch.rs`            | Durable bounded batch behavior                      |
| `tests/live_ingestion.rs`                  | Temporary-vault local model integration             |
| `src/adapters/baml_analyzer.rs` unit tests | Span IDs, offsets, chunk bounds, and map provenance |

### Risk

- Graph IDs and relation types become durable query contracts.
- Model-local IDs are untrusted and must never become persisted IDs directly.
- Summary prose is synthetic; claims, entities, and relations require exact evidence spans.
- Relation endpoints may reference claims or entities and must resolve before persistence.
- Batch output is larger and requires strict per-document and aggregate bounds.

## Crate Ownership

- **Owner crate**: `episteme-local` -- intelligence composes existing local extraction,
  classification, BAML, and DuckDB boundaries.
- **Affected crates**: none; this repository contains one package.

## BAML API

```baml
enum ClaimKind {
  Fact
  Inference
  Recommendation
  Critique
}

enum EntityKind {
  Person
  Organization
  Project
  Technology
  Concept
  Method
  Dataset
  Benchmark
  Document
}

enum SemanticRelationType {
  Supports
  Contradicts
  Implements
  Evaluates
  DependsOn
  Extends
  Uses
  Causes
  PartOf
  EvolvesFrom
}

class ClaimOutput {
  local_id string
  text string
  kind ClaimKind
  confidence_percent int
  evidence_span_ids string[]
}

class EntityOutput {
  local_id string
  name string
  aliases string[]
  kind EntityKind
  description string
  evidence_span_ids string[]
}

class SemanticRelationOutput {
  source_local_id string
  target_local_id string
  relation_type SemanticRelationType
  confidence_percent int
  evidence_span_ids string[]
}

class IntelligenceChunkOutput {
  summary string
  key_points string[]
  summary_span_ids string[]
  claims ClaimOutput[]
  entities EntityOutput[]
  relations SemanticRelationOutput[]
}

class DocumentIntelligenceOutput {
  summary string
  key_points string[]
  summary_span_ids string[]
  claims ClaimOutput[]
  entities EntityOutput[]
  relations SemanticRelationOutput[]
}

function ExtractIntelligenceChunk(
  spans: SourceSpanInput[],
  classification: DocumentClassification
) -> IntelligenceChunkOutput

function AggregateDocumentIntelligence(
  classification: DocumentClassification,
  chunks: IntelligenceChunkOutput[]
) -> DocumentIntelligenceOutput
```

Prompts treat source spans and intermediate outputs as untrusted data. Every claim, entity, relation,
summary, and key point selects only supplied span IDs. Relation endpoints reference output-local
claim or entity IDs. Confidence below 60 is excluded by policy.

## Public API

### Enums

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimKind {
    Fact,
    Inference,
    Recommendation,
    Critique,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Person,
    Organization,
    Project,
    Technology,
    Concept,
    Method,
    Dataset,
    Benchmark,
    Document,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticRelationType {
    Supports,
    Contradicts,
    Implements,
    Evaluates,
    DependsOn,
    Extends,
    Uses,
    Causes,
    PartOf,
    EvolvesFrom,
}
```

### Intelligence Types

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelligenceSummary {
    pub text: String,
    pub key_points: Vec<String>,
    pub evidence: Vec<EvidenceReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelligenceClaim {
    pub id: String,
    pub text: String,
    pub kind: ClaimKind,
    pub confidence_percent: u8,
    pub evidence: Vec<EvidenceReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelligenceEntity {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub kind: EntityKind,
    pub description: String,
    pub evidence: Vec<EvidenceReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticRelation {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation_type: SemanticRelationType,
    pub confidence_percent: u8,
    pub evidence: Vec<EvidenceReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentIntelligence {
    pub source_digest: SourceDigest,
    pub classification: DocumentClassification,
    pub summary: IntelligenceSummary,
    pub claims: Vec<IntelligenceClaim>,
    pub entities: Vec<IntelligenceEntity>,
    pub relations: Vec<SemanticRelation>,
    pub analysis: AnalysisProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IntelligenceCacheKey {
    pub model: String,
    pub policy_version: String,
}
```

Stable IDs are lowercase BLAKE3 hashes over canonical values:

- claim: normalized claim text
- entity: entity kind plus normalized canonical name
- relation: source ID, relation type, target ID, and source digest

Model-generated local IDs are namespaced per chunk, validated, and discarded after references are
resolved.

### Ports

```rust
#[async_trait]
pub trait DocumentIntelligenceAnalyzer: Send + Sync {
    async fn analyze(
        &self,
        document: &ExtractedDocument,
        classification: &DocumentClassification,
    ) -> Result<DocumentIntelligenceDraft, IntelligenceFailure>;
}

pub trait DocumentIntelligenceStore: Send + Sync {
    fn load(
        &self,
        digest: &SourceDigest,
        key: &IntelligenceCacheKey,
    ) -> Result<Option<DocumentIntelligence>, IntelligenceStoreError>;

    fn save(
        &self,
        key: &IntelligenceCacheKey,
        intelligence: &DocumentIntelligence,
    ) -> Result<(), IntelligenceStoreError>;
}

pub trait IntelligenceRunStore: Send + Sync {
    fn begin_batch(&self, batch_id: &str, started_at: &str)
        -> Result<(), IntelligenceStoreError>;
    fn save_attempt(&self, attempt: &IntelligenceAttemptRecord)
        -> Result<(), IntelligenceStoreError>;
    fn latest_attempts(&self, batch_id: &str)
        -> Result<Vec<IntelligenceAttemptRecord>, IntelligenceStoreError>;
    fn finish_batch(&self, summary: &IntelligenceBatchSummary)
        -> Result<(), IntelligenceStoreError>;
}
```

### Services

```rust
#[derive(Debug)]
pub struct DocumentIntelligencePipeline<E, C, A, S> {
    extractor: E,
    classifier: C,
    analyzer: A,
    store: S,
}

impl<E, C, A, S> DocumentIntelligencePipeline<E, C, A, S> {
    pub async fn analyze(
        &self,
        source: &StagedSource,
        cache_policy: CachePolicy,
    ) -> Result<DocumentIntelligence, IntelligenceError>;
}

#[derive(Debug)]
pub struct IntelligenceBatch<P, S> {
    processor: Arc<P>,
    store: Arc<S>,
    maximum_concurrency: usize,
}
```

The intelligence pipeline extracts once. It classifies the resulting `ExtractedDocument` through a
new `ClassificationPipeline::classify_extracted` entry point, then analyzes the same immutable text.

## Validation and Canonicalization

1. Build deterministic spans of at most 500 Unicode scalar values with exact character ranges.
2. Pack spans into 12,000-character map chunks, at most 24 chunks per document.
3. Namespace map-local claim/entity IDs with the chunk index before aggregation.
4. Reject unknown span IDs, duplicate local IDs, unresolved relation endpoints, confidence outside
   60..=100, excessive counts, control characters, and bounded-field violations.
5. Aggregate once, then require all aggregate span IDs and node references to come from map output.
6. Normalize whitespace, aliases, and canonical names; deduplicate claims/entities/relations.
7. Reconstruct every evidence quote and location from original source spans in Rust.
8. Generate stable IDs and run final referential-integrity checks before persistence.

## Persistence

`004_document_intelligence.sql` adds:

- `document_intelligence_versions`: source digest, model, policy, summary, key points, provenance.
- `intelligence_claims`: versioned claims and evidence JSON.
- `intelligence_entities`: canonical global entity IDs, kind, name, aliases, description.
- `document_intelligence_entities`: document-version/entity membership and evidence.
- `semantic_relations`: document-version relation edges, confidence, and evidence.
- `intelligence_batches` and `intelligence_attempts`: resumable batch state and safe metrics.

All tables are rebuildable derived state. The Obsidian vault remains canonical source storage.

## CLI

```text
episteme analyze [--force] <source>
episteme analyze-batch [--force] [--retry-failed] [--concurrency <1..5>]
                      [--batch-id <id>] [--summary <new-path>]
```

Output JSON includes the complete document intelligence object for `analyze`, and one reconciled
relative-path batch summary for `analyze-batch`. Summary writes are atomic and no-clobber.

## Data Flow

1. Safely stage and extract the source once.
2. Reuse or create normalized classification for the extracted text.
3. Reuse versioned intelligence cache when allowed.
4. Map source spans to typed chunk intelligence with bounded local concurrency.
5. Namespace and validate map output, then aggregate once.
6. Resolve source span evidence, canonicalize graph nodes/edges, generate stable IDs, and validate
   referential integrity.
7. Persist the complete version atomically in DuckDB.
8. Batch mode checkpoints every attempt and emits one reconciled summary.

## Hexagonal Boundaries

- **Classification port**: existing `DocumentClassifier`.
- **Intelligence port**: `DocumentIntelligenceAnalyzer`; adapter: BAML map/reduce analyzer.
- **Graph persistence port**: `DocumentIntelligenceStore`; adapter: DuckDB.
- **Batch ledger port**: `IntelligenceRunStore`; adapter: DuckDB.
- **Application services**: `DocumentIntelligencePipeline` and `IntelligenceBatch`.

## Out of Scope

- Cross-document relation inference without shared canonical entities
- Embeddings or vector similarity
- Writing graph nodes directly into Obsidian or kgx
- Automatic contradiction adjudication
- User-defined relation types
- Cloud model fallback

## Risk

- [x] New durable graph contract: yes -- IDs, entity kinds, and relation types
- [x] New external dependency: no
- [x] Database migration: additive
- [x] Canonical vault mutation: no
- [x] Feature flag required: no
