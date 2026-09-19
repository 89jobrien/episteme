# Design: Chunked Research Distillation

## Goal

Distill documents larger than a local model's context window by extracting typed, grounded findings
from bounded text chunks and aggregating those findings into the existing validated research draft.

## Approved Approach

Use typed map/reduce: `DistillResearchChunk` maps bounded source chunks to structured findings, and
`AggregateResearchChunks` reduces ordered findings into `AggregatedResearchOutput` for deterministic
Rust evidence reconstruction.

## Context Map

### Files to Modify

| File                            | Purpose                         | Changes Needed                                                |
| ------------------------------- | ------------------------------- | ------------------------------------------------------------- |
| `baml_src/research.baml`        | Typed model contracts           | Add chunk output type and map/reduce functions                |
| `src/adapters/baml_analyzer.rs` | Local BAML research adapter     | Split large text, run bounded chunk calls, aggregate findings |
| `tests/live_ingestion.rs`       | End-to-end local model coverage | Add an ignored large-document ingestion case                  |
| `README.md`                     | Operator documentation          | Document automatic long-document distillation                 |

### Generated Files

`baml-cli generate` updates `src/baml_client/`. Generated files remain unedited by hand.

### Dependencies

| File                    | Relationship                                                              |
| ----------------------- | ------------------------------------------------------------------------- |
| `src/ports/mod.rs`      | `ResearchAnalyzer` contract remains unchanged                             |
| `src/domain/mod.rs`     | Existing `ResearchDraft::validate_against` verifies aggregated evidence   |
| `src/ingest/mod.rs`     | Existing ingestion pipeline receives the final validated draft unchanged  |
| `baml_src/clients.baml` | Both new functions use the existing loopback-only `LocalDistiller` client |

### Existing Test Coverage

| Test                      | Coverage                                                           |
| ------------------------- | ------------------------------------------------------------------ |
| `tests/analysis.rs`       | Rejects final drafts with ungrounded evidence                      |
| `tests/live_ingestion.rs` | Exercises classification, distillation, note writing, and archival |

### Risk

- Generated BAML APIs change additively; handwritten generated code is forbidden.
- Concurrent chunk calls must remain bounded below the local server's concurrency limit.
- Aggregation can omit or invent span IDs, so Rust validates map provenance before reconstruction.
- Excessive chunk counts must fail before model requests rather than overflow aggregation context.

## Crate Ownership

- **Owner crate**: `episteme` -- chunking is orchestration inside the existing local BAML adapter.
- **Affected crates**: none; this repository contains one package.

## BAML API

```baml
class ResearchChunkOutput {
  summary string
  key_ideas string[]
  implementation_notes string[]
  critique_points string[]
  evidence_span_ids string[]
}

class SourceSpanInput {
  id string
  text string
  location string
}

class AggregatedResearchOutput {
  title string
  citation string
  topics string[]
  summary string
  key_ideas string[]
  implementation_notes string[]
  critique string
  evidence_span_ids string[]
}

function DistillResearchChunk(
  spans: SourceSpanInput[],
  classification: DocumentClassification
) -> ResearchChunkOutput

function AggregateResearchChunks(
  classification: DocumentClassification,
  chunks: ResearchChunkOutput[]
) -> AggregatedResearchOutput
```

## Rust API

No public Rust signature changes. `BamlResearchAnalyzer::new` and `ResearchAnalyzer::analyze` remain
compatible.

Private adapter types and functions:

```rust
struct DocumentChunk {
    index: usize,
    spans: Vec<SourceSpan>,
}

struct SourceSpan {
    id: String,
    text: String,
    location: String,
}

fn chunk_document(document: &str) -> Result<Vec<DocumentChunk>, AnalysisError>;

impl BamlResearchAnalyzer {
  async fn distill_chunks(
    &self,
    chunks: Vec<DocumentChunk>,
    classification: &BamlClassification,
  ) -> Result<ResearchDraft, AnalysisError>;
}
```

Adapter constants:

```rust
const DISTILLATION_CHUNK_CHARACTERS: usize = 12_000;
const MAX_DISTILLATION_CHUNKS: usize = 24;
const MAX_CONCURRENT_CHUNKS: usize = 2;
```

## Data Flow

1. Classification still runs once against the complete extracted document using the configured
   larger-context classifier.
2. Documents at or below 12,000 characters continue through the existing `DistillResearch`
   function unchanged.
3. Rust divides source paragraphs into Unicode-safe spans of at most 500 characters, assigns stable
   IDs and exact character ranges, then packs those spans into bounded chunks.
4. Up to two `DistillResearchChunk` calls run concurrently against ordered chunks; each returns
   concise findings and selected span IDs rather than generated quote text.
5. Rust rejects unknown IDs, bounds all model output, sorts chunk results by source order, and
   passes them once to `AggregateResearchChunks`.
6. Rust resolves the aggregate's selected IDs back to exact source text and locations, creates the
   `ResearchDraft`, and runs `validate_against` before any vault mutation.

## Prompt Safety

- Source chunks and intermediate findings are explicitly marked as untrusted data.
- Chunk prompts select only supplied span IDs; Rust rejects invented IDs.
- Aggregation selects IDs from chunk findings and never generates quote text or locations.
- Rust reconstructs verbatim evidence and stable locations from the original extracted source.
- Chunk outputs are requested to remain concise with bounded idea, note, critique, and evidence
  counts to protect the aggregation context.

## Failure Behavior

- More than 24 chunks returns `AnalysisError::Invalid` before inference.
- Any chunk request failure cancels remaining work and returns a content-free model error.
- Prompt-requested list limits are enforced programmatically through deterministic truncation.
- Aggregation failure returns a content-free model error.
- Invalid or ungrounded aggregate output fails existing domain validation and causes no note,
  archive, or index mutation.

## Out of Scope

- Hierarchical multi-level aggregation beyond 24 chunks
- Persisting intermediate chunk outputs
- Resuming partially completed chunk maps
- User-configurable chunk or concurrency settings
- Changing standalone classification caching

## Risk

- [x] Breaking API changes: no
- [x] New external dependency: no; Tokio `JoinSet` supplies bounded concurrency
- [x] Feature flag required: no
- [x] Canonical vault mutation before final validation: no
