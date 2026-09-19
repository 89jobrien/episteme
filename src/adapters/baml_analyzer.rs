//! Typed BAML adapter for local document classification and distillation.

use std::collections::HashSet;
use std::time::Duration;

use async_trait::async_trait;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::baml_client::B;
use crate::baml_client::types::{
    AggregatedResearchOutput, DocumentClassification as BamlClassification, ResearchChunkOutput,
    ResearchDraftOutput, SourceSpanInput,
};
use crate::config::LocalModelEndpoint;
use crate::domain::{DocumentClassification, EvidenceReference, ExtractedDocument, ResearchDraft};
use crate::ports::{AnalysisError, DocumentClassifier, ResearchAnalyzer};

const DISTILLATION_CHUNK_CHARACTERS: usize = 12_000;
const MAX_DISTILLATION_CHUNKS: usize = 24;
const MAX_CONCURRENT_CHUNKS: usize = 2;
const MAX_AGGREGATE_FINDINGS_CHARACTERS: usize = 60_000;
const EVIDENCE_SPAN_CHARACTERS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DocumentChunk {
    index: usize,
    spans: Vec<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceSpan {
    id: String,
    text: String,
    location: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DistillationPlan {
    Direct,
    Chunked(Vec<DocumentChunk>),
}

/// Runs Episteme's typed classification function against a configured local client.
#[derive(Debug, Clone)]
pub struct BamlDocumentClassifier {
    endpoint: LocalModelEndpoint,
    timeout: Duration,
}

impl BamlDocumentClassifier {
    /// Creates a classifier using a validated local endpoint.
    #[must_use]
    pub const fn new(endpoint: LocalModelEndpoint, timeout: Duration) -> Self {
        Self { endpoint, timeout }
    }
}

#[async_trait]
impl DocumentClassifier for BamlDocumentClassifier {
    async fn classify(
        &self,
        document: &ExtractedDocument,
    ) -> Result<DocumentClassification, AnalysisError> {
        let classification =
            classify_with_baml(&self.endpoint, self.timeout, document.text()).await?;
        Ok(DocumentClassification {
            title: classification.title,
            authors: classification.authors,
            source_type: classification.source_type,
            language: classification.language,
            topics: classification.topics,
        })
    }
}

/// Runs Episteme's typed BAML functions against configured local clients.
#[derive(Debug, Clone)]
pub struct BamlResearchAnalyzer {
    classifier: LocalModelEndpoint,
    distiller: LocalModelEndpoint,
    timeout: Duration,
}

impl BamlResearchAnalyzer {
    /// Creates an analyzer using validated local endpoints.
    #[must_use]
    pub const fn new(
        classifier: LocalModelEndpoint,
        distiller: LocalModelEndpoint,
        timeout: Duration,
    ) -> Self {
        Self {
            classifier,
            distiller,
            timeout,
        }
    }
}

#[async_trait]
impl ResearchAnalyzer for BamlResearchAnalyzer {
    async fn analyze(&self, document: &ExtractedDocument) -> Result<ResearchDraft, AnalysisError> {
        timeout(self.timeout, self.analyze_within_deadline(document))
            .await
            .map_err(|_| AnalysisError::Model("research analysis timed out".to_owned()))?
    }
}

impl BamlResearchAnalyzer {
    async fn analyze_within_deadline(
        &self,
        document: &ExtractedDocument,
    ) -> Result<ResearchDraft, AnalysisError> {
        let plan = plan_distillation(document.text())?;
        let classification =
            classify_with_baml(&self.classifier, self.timeout, document.text()).await?;
        let draft = match plan {
            DistillationPlan::Direct => {
                let output = self
                    .distill_direct(document.text(), &classification)
                    .await?;
                validate_research_output(&output)?;
                research_draft(output)
            }
            DistillationPlan::Chunked(chunks) => {
                self.distill_chunks(chunks, &classification).await?
            }
        };
        draft
            .validate_against(document.text())
            .map_err(|error| AnalysisError::Invalid(error.to_string()))
    }

    async fn distill_direct(
        &self,
        document: &str,
        classification: &BamlClassification,
    ) -> Result<ResearchDraftOutput, AnalysisError> {
        B.DistillResearch
            .with_env_var(
                "EPISTEME_DISTILL_BASE_URL",
                self.distiller.base_url().as_str(),
            )
            .with_env_var("EPISTEME_DISTILL_MODEL", self.distiller.model())
            .with_cancellation_token(Some(baml::CancellationToken::new_with_timeout(
                self.timeout,
            )))
            .call(document, classification)
            .await
            .map_err(|_| AnalysisError::Model("distiller request failed".to_owned()))
    }

    async fn distill_chunks(
        &self,
        chunks: Vec<DocumentChunk>,
        classification: &BamlClassification,
    ) -> Result<ResearchDraft, AnalysisError> {
        let spans = chunks
            .iter()
            .flat_map(|chunk| chunk.spans.iter().cloned())
            .collect::<Vec<_>>();
        let mut pending = chunks.into_iter();
        let mut workers = JoinSet::new();
        for chunk in pending.by_ref().take(MAX_CONCURRENT_CHUNKS) {
            spawn_chunk_worker(
                &mut workers,
                chunk,
                classification.clone(),
                self.distiller.clone(),
                self.timeout,
            );
        }

        let mut findings = Vec::new();
        while let Some(result) = workers.join_next().await {
            let finding = result
                .map_err(|_| AnalysisError::Model("chunk distiller task failed".to_owned()))??;
            findings.push(finding);
            if let Some(chunk) = pending.next() {
                spawn_chunk_worker(
                    &mut workers,
                    chunk,
                    classification.clone(),
                    self.distiller.clone(),
                    self.timeout,
                );
            }
        }
        findings.sort_by_key(|(index, _)| *index);
        let ordered = findings
            .into_iter()
            .map(|(_, output)| output)
            .collect::<Vec<_>>();
        validate_aggregate_findings(&ordered)?;
        let allowed_evidence = ordered
            .iter()
            .flat_map(|output| output.evidence_span_ids.iter().cloned())
            .collect::<HashSet<_>>();

        let output = B
            .AggregateResearchChunks
            .with_env_var(
                "EPISTEME_DISTILL_BASE_URL",
                self.distiller.base_url().as_str(),
            )
            .with_env_var("EPISTEME_DISTILL_MODEL", self.distiller.model())
            .with_cancellation_token(Some(baml::CancellationToken::new_with_timeout(
                self.timeout,
            )))
            .call(classification, &ordered)
            .await
            .map_err(|_| AnalysisError::Model("research aggregation failed".to_owned()))?;
        validate_aggregated_output(&output)?;
        validate_aggregate_evidence_ids(&output, &allowed_evidence)?;
        aggregated_research_draft(output, &spans)
    }
}

fn spawn_chunk_worker(
    workers: &mut JoinSet<Result<(usize, ResearchChunkOutput), AnalysisError>>,
    chunk: DocumentChunk,
    classification: BamlClassification,
    endpoint: LocalModelEndpoint,
    timeout: Duration,
) {
    workers.spawn(async move {
        let spans = chunk
            .spans
            .iter()
            .map(|span| SourceSpanInput {
                id: span.id.clone(),
                text: span.text.clone(),
                location: span.location.clone(),
            })
            .collect::<Vec<_>>();
        let output = B
            .DistillResearchChunk
            .with_env_var("EPISTEME_DISTILL_BASE_URL", endpoint.base_url().as_str())
            .with_env_var("EPISTEME_DISTILL_MODEL", endpoint.model())
            .with_cancellation_token(Some(baml::CancellationToken::new_with_timeout(timeout)))
            .call(&spans, &classification)
            .await
            .map_err(|_| AnalysisError::Model("chunk distiller request failed".to_owned()))?;
        validate_chunk_evidence_ids(&chunk, &output.evidence_span_ids)?;
        let output = normalize_chunk_output(output);
        validate_chunk_output(&chunk, &output)?;
        Ok((chunk.index, output))
    });
}

fn normalize_chunk_output(mut output: ResearchChunkOutput) -> ResearchChunkOutput {
    output.summary = truncate_characters(&output.summary, 2_000);
    truncate_list(&mut output.key_ideas, 5, 500);
    truncate_list(&mut output.implementation_notes, 5, 500);
    truncate_list(&mut output.critique_points, 3, 500);
    let mut seen = HashSet::new();
    output
        .evidence_span_ids
        .retain(|id| seen.insert(id.clone()));
    output.evidence_span_ids.truncate(5);
    output
}

fn truncate_list(values: &mut Vec<String>, maximum_items: usize, maximum_characters: usize) {
    values.truncate(maximum_items);
    for value in values {
        *value = truncate_characters(value, maximum_characters);
    }
}

fn truncate_characters(value: &str, maximum_characters: usize) -> String {
    value.chars().take(maximum_characters).collect()
}

fn validate_chunk_evidence_ids(chunk: &DocumentChunk, ids: &[String]) -> Result<(), AnalysisError> {
    let available = chunk
        .spans
        .iter()
        .map(|span| span.id.as_str())
        .collect::<HashSet<_>>();
    if ids
        .iter()
        .any(|id| !valid_model_text(id, 32) || !available.contains(id.as_str()))
    {
        return Err(AnalysisError::Invalid(
            "chunk findings selected an unknown evidence span".to_owned(),
        ));
    }
    Ok(())
}

fn validate_chunk_output(
    chunk: &DocumentChunk,
    output: &ResearchChunkOutput,
) -> Result<(), AnalysisError> {
    if !valid_model_text(&output.summary, 2_000) {
        return Err(AnalysisError::Invalid(
            "chunk summary exceeded validation limits".to_owned(),
        ));
    }
    let lists_valid = output.key_ideas.len() <= 5
        && output.implementation_notes.len() <= 5
        && output.critique_points.len() <= 3
        && output.evidence_span_ids.len() <= 5
        && output
            .key_ideas
            .iter()
            .all(|value| valid_model_text(value, 500))
        && output
            .implementation_notes
            .iter()
            .all(|value| valid_model_text(value, 500))
        && output
            .critique_points
            .iter()
            .all(|value| valid_model_text(value, 500));
    if !lists_valid {
        return Err(AnalysisError::Invalid(
            "chunk finding lists exceeded validation limits".to_owned(),
        ));
    }
    validate_chunk_evidence_ids(chunk, &output.evidence_span_ids)?;
    Ok(())
}

fn validate_aggregate_findings(findings: &[ResearchChunkOutput]) -> Result<(), AnalysisError> {
    let total = findings.iter().fold(0_usize, |sum, output| {
        sum.saturating_add(chunk_output_characters(output))
    });
    if total > MAX_AGGREGATE_FINDINGS_CHARACTERS {
        return Err(AnalysisError::Invalid(
            "chunk findings exceed the aggregation limit".to_owned(),
        ));
    }
    Ok(())
}

fn validate_aggregate_evidence_ids(
    output: &AggregatedResearchOutput,
    allowed: &HashSet<String>,
) -> Result<(), AnalysisError> {
    if output
        .evidence_span_ids
        .iter()
        .any(|id| !allowed.contains(id))
    {
        return Err(AnalysisError::Invalid(
            "aggregate selected evidence not chosen by chunk findings".to_owned(),
        ));
    }
    Ok(())
}

fn chunk_output_characters(output: &ResearchChunkOutput) -> usize {
    std::iter::once(output.summary.as_str())
        .chain(output.key_ideas.iter().map(String::as_str))
        .chain(output.implementation_notes.iter().map(String::as_str))
        .chain(output.critique_points.iter().map(String::as_str))
        .chain(output.evidence_span_ids.iter().map(String::as_str))
        .fold(0_usize, |sum, value| {
            sum.saturating_add(value.chars().count())
        })
}

fn validate_research_output(output: &ResearchDraftOutput) -> Result<(), AnalysisError> {
    let lists_valid = !output.topics.is_empty()
        && output.topics.len() <= 64
        && output.key_ideas.len() <= 64
        && output.implementation_notes.len() <= 64
        && !output.evidence.is_empty()
        && output.evidence.len() <= 64
        && output
            .topics
            .iter()
            .all(|value| valid_model_text(value, 128))
        && output
            .key_ideas
            .iter()
            .all(|value| valid_model_text(value, 1_000))
        && output
            .implementation_notes
            .iter()
            .all(|value| valid_model_text(value, 1_000))
        && output.evidence.iter().all(|reference| {
            valid_model_text(&reference.quote, 1_000) && valid_model_text(&reference.location, 256)
        });
    let scalars_valid = valid_model_text(&output.title, 512)
        && valid_model_text(&output.citation, 1_024)
        && valid_model_text(&output.summary, 8_000)
        && valid_model_text(&output.critique, 8_000);
    if !lists_valid || !scalars_valid {
        return Err(AnalysisError::Invalid(
            "aggregated research output exceeded validation limits".to_owned(),
        ));
    }
    Ok(())
}

fn validate_aggregated_output(output: &AggregatedResearchOutput) -> Result<(), AnalysisError> {
    let lists_valid = !output.topics.is_empty()
        && output.topics.len() <= 64
        && output.key_ideas.len() <= 64
        && output.implementation_notes.len() <= 64
        && !output.evidence_span_ids.is_empty()
        && output.evidence_span_ids.len() <= 64
        && output
            .topics
            .iter()
            .all(|value| valid_model_text(value, 128))
        && output
            .key_ideas
            .iter()
            .all(|value| valid_model_text(value, 1_000))
        && output
            .implementation_notes
            .iter()
            .all(|value| valid_model_text(value, 1_000))
        && output
            .evidence_span_ids
            .iter()
            .all(|value| valid_model_text(value, 32));
    let scalars_valid = valid_model_text(&output.title, 512)
        && valid_model_text(&output.citation, 1_024)
        && valid_model_text(&output.summary, 8_000)
        && valid_model_text(&output.critique, 8_000);
    if !lists_valid || !scalars_valid {
        return Err(AnalysisError::Invalid(
            "aggregated research output exceeded validation limits".to_owned(),
        ));
    }
    Ok(())
}

fn valid_model_text(value: &str, maximum_characters: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= maximum_characters
}

fn research_draft(output: ResearchDraftOutput) -> ResearchDraft {
    ResearchDraft {
        title: output.title,
        citation: output.citation,
        topics: output.topics,
        summary: output.summary,
        key_ideas: output.key_ideas,
        implementation_notes: output.implementation_notes,
        critique: output.critique,
        evidence: output
            .evidence
            .into_iter()
            .map(|reference| EvidenceReference {
                quote: reference.quote,
                location: reference.location,
            })
            .collect(),
    }
}

fn aggregated_research_draft(
    output: AggregatedResearchOutput,
    spans: &[SourceSpan],
) -> Result<ResearchDraft, AnalysisError> {
    let evidence = evidence_from_span_ids(&output.evidence_span_ids, spans)?;
    Ok(ResearchDraft {
        title: output.title,
        citation: output.citation,
        topics: output.topics,
        summary: output.summary,
        key_ideas: output.key_ideas,
        implementation_notes: output.implementation_notes,
        critique: output.critique,
        evidence,
    })
}

fn source_spans(document: &str) -> Result<Vec<SourceSpan>, AnalysisError> {
    let mut spans = Vec::new();
    let mut segment_start_character = 0;
    for segment in document.split_inclusive("\n\n") {
        let content = segment.trim();
        if content.is_empty() {
            segment_start_character += segment.chars().count();
            continue;
        }
        let content_byte_offset = segment
            .find(content)
            .ok_or_else(|| AnalysisError::Invalid("source span offset was invalid".to_owned()))?;
        let content_start_character =
            segment_start_character + segment[..content_byte_offset].chars().count();
        let mut local_byte = 0;
        let mut local_character = 0;
        while local_byte < content.len() {
            let remaining = &content[local_byte..];
            let split = remaining
                .char_indices()
                .nth(EVIDENCE_SPAN_CHARACTERS)
                .map_or(remaining.len(), |(index, _)| index);
            let text = remaining[..split].to_owned();
            let character_count = text.chars().count();
            let start = content_start_character + local_character;
            let end = start + character_count.saturating_sub(1);
            spans.push(SourceSpan {
                id: format!("S{:06}", spans.len() + 1),
                text,
                location: format!("characters {start}-{end}"),
            });
            local_byte += split;
            local_character += character_count;
        }
        segment_start_character += segment.chars().count();
    }
    if spans.is_empty() {
        return Err(AnalysisError::Invalid(
            "document contained no evidence spans".to_owned(),
        ));
    }
    Ok(spans)
}

fn evidence_from_span_ids(
    ids: &[String],
    spans: &[SourceSpan],
) -> Result<Vec<EvidenceReference>, AnalysisError> {
    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            continue;
        }
        let span = spans.iter().find(|span| span.id == *id).ok_or_else(|| {
            AnalysisError::Invalid("model selected an unknown evidence span".to_owned())
        })?;
        selected.push(EvidenceReference {
            quote: span.text.clone(),
            location: span.location.clone(),
        });
    }
    Ok(selected)
}

fn plan_distillation(document: &str) -> Result<DistillationPlan, AnalysisError> {
    if document.chars().count() <= DISTILLATION_CHUNK_CHARACTERS {
        Ok(DistillationPlan::Direct)
    } else {
        chunk_document(document).map(DistillationPlan::Chunked)
    }
}

fn chunk_document(document: &str) -> Result<Vec<DocumentChunk>, AnalysisError> {
    let maximum_characters = DISTILLATION_CHUNK_CHARACTERS * MAX_DISTILLATION_CHUNKS;
    if document.chars().count() > maximum_characters {
        return Err(AnalysisError::Invalid(format!(
            "document requires more than {MAX_DISTILLATION_CHUNKS} distillation chunks"
        )));
    }
    let spans = source_spans(document)?;
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    let mut current_characters = 0_usize;
    for span in spans {
        let span_characters = span.text.chars().count();
        if !current.is_empty()
            && current_characters.saturating_add(span_characters) > DISTILLATION_CHUNK_CHARACTERS
        {
            ensure_chunk_capacity(chunks.len())?;
            chunks.push(DocumentChunk {
                index: chunks.len(),
                spans: std::mem::take(&mut current),
            });
            current_characters = 0;
        }
        current_characters = current_characters.saturating_add(span_characters);
        current.push(span);
    }
    if !current.is_empty() {
        ensure_chunk_capacity(chunks.len())?;
        chunks.push(DocumentChunk {
            index: chunks.len(),
            spans: current,
        });
    }
    Ok(chunks)
}

fn ensure_chunk_capacity(chunk_count: usize) -> Result<(), AnalysisError> {
    if chunk_count >= MAX_DISTILLATION_CHUNKS {
        return Err(AnalysisError::Invalid(format!(
            "document requires more than {MAX_DISTILLATION_CHUNKS} distillation chunks"
        )));
    }
    Ok(())
}

async fn classify_with_baml(
    endpoint: &LocalModelEndpoint,
    timeout: Duration,
    document: &str,
) -> Result<BamlClassification, AnalysisError> {
    B.ClassifyDocument
        .with_env_var("EPISTEME_CLASSIFY_BASE_URL", endpoint.base_url().as_str())
        .with_env_var("EPISTEME_CLASSIFY_MODEL", endpoint.model())
        .with_cancellation_token(Some(baml::CancellationToken::new_with_timeout(timeout)))
        .call(document)
        .await
        .map_err(|_| AnalysisError::Model("classifier request failed".to_owned()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::baml_client::types::{AggregatedResearchOutput, ResearchChunkOutput};

    use super::{
        DISTILLATION_CHUNK_CHARACTERS, DistillationPlan, MAX_AGGREGATE_FINDINGS_CHARACTERS,
        chunk_document, evidence_from_span_ids, normalize_chunk_output, plan_distillation,
        source_spans, validate_aggregate_evidence_ids, validate_aggregate_findings,
        validate_chunk_output,
    };

    #[test]
    fn chunk_document_preserves_text_and_bounds_chunks() -> Result<(), Box<dyn std::error::Error>> {
        let oversized_paragraph = "é".repeat(DISTILLATION_CHUNK_CHARACTERS + 17);
        let document = format!("first paragraph\n\n{oversized_paragraph}\n\nlast paragraph");

        let chunks = chunk_document(&document)?;

        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|chunk| {
            chunk
                .spans
                .iter()
                .map(|span| span.text.chars().count())
                .sum::<usize>()
                <= DISTILLATION_CHUNK_CHARACTERS
        }));
        assert!(
            chunks
                .iter()
                .flat_map(|chunk| &chunk.spans)
                .all(|span| { document.contains(&span.text) && span.text.chars().count() <= 500 })
        );
        assert_eq!(chunks[0].index, 0);
        Ok(())
    }

    #[test]
    fn large_documents_use_chunked_distillation() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            plan_distillation("short document")?,
            DistillationPlan::Direct
        );

        let document = "paragraph\n\n".repeat(DISTILLATION_CHUNK_CHARACTERS);
        let DistillationPlan::Chunked(chunks) = plan_distillation(&document)? else {
            return Err("large document did not select chunked distillation".into());
        };
        assert!(chunks.len() > 1);
        assert!(
            chunks
                .windows(2)
                .all(|pair| pair[0].index + 1 == pair[1].index)
        );
        Ok(())
    }

    #[test]
    fn distillation_plan_enforces_threshold_and_chunk_limit()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            plan_distillation(&"x".repeat(DISTILLATION_CHUNK_CHARACTERS))?,
            DistillationPlan::Direct
        );
        assert!(matches!(
            plan_distillation(&"x".repeat(DISTILLATION_CHUNK_CHARACTERS + 1))?,
            DistillationPlan::Chunked(_)
        ));
        assert!(
            plan_distillation(
                &"x".repeat(DISTILLATION_CHUNK_CHARACTERS * super::MAX_DISTILLATION_CHUNKS + 1)
            )
            .is_err()
        );
        let fragmented = vec!["x".repeat(401); 697].join("\n\n");
        assert!(fragmented.chars().count() < DISTILLATION_CHUNK_CHARACTERS * 24);
        assert!(plan_distillation(&fragmented).is_err());
        Ok(())
    }

    #[test]
    fn chunk_findings_require_bounded_grounded_evidence() {
        let chunk = super::DocumentChunk {
            index: 0,
            spans: vec![super::SourceSpan {
                id: "S000001".to_owned(),
                text: "grounded source quote".to_owned(),
                location: "characters 0-20".to_owned(),
            }],
        };
        let invalid = ResearchChunkOutput {
            summary: "summary".to_owned(),
            key_ideas: vec!["idea".to_owned()],
            implementation_notes: Vec::new(),
            critique_points: Vec::new(),
            evidence_span_ids: vec!["S999999".to_owned()],
        };

        assert!(validate_chunk_output(&chunk, &invalid).is_err());
    }

    #[test]
    fn normalization_does_not_hide_unknown_span_ids() {
        let chunk = super::DocumentChunk {
            index: 0,
            spans: vec![super::SourceSpan {
                id: "S000001".to_owned(),
                text: "grounded source quote".to_owned(),
                location: "characters 0-20".to_owned(),
            }],
        };
        let output = ResearchChunkOutput {
            summary: "summary".to_owned(),
            key_ideas: Vec::new(),
            implementation_notes: Vec::new(),
            critique_points: Vec::new(),
            evidence_span_ids: vec!["S999999".to_owned()],
        };

        let normalized = normalize_chunk_output(output);

        assert!(validate_chunk_output(&chunk, &normalized).is_err());
    }

    #[test]
    fn aggregate_rejects_span_ids_not_selected_by_chunk_maps() {
        let output = AggregatedResearchOutput {
            title: "title".to_owned(),
            citation: "citation".to_owned(),
            topics: vec!["topic".to_owned()],
            summary: "summary".to_owned(),
            key_ideas: vec!["idea".to_owned()],
            implementation_notes: vec!["note".to_owned()],
            critique: "critique".to_owned(),
            evidence_span_ids: vec!["S000002".to_owned()],
        };
        let allowed = HashSet::from(["S000001".to_owned()]);

        assert!(validate_aggregate_evidence_ids(&output, &allowed).is_err());
    }

    #[test]
    fn aggregate_findings_reject_excessive_input() {
        let oversized = ResearchChunkOutput {
            summary: "x".repeat(MAX_AGGREGATE_FINDINGS_CHARACTERS + 1),
            key_ideas: Vec::new(),
            implementation_notes: Vec::new(),
            critique_points: Vec::new(),
            evidence_span_ids: Vec::new(),
        };

        assert!(validate_aggregate_findings(&[oversized]).is_err());
    }

    #[test]
    fn aggregate_output_rejects_excessive_metadata() {
        let oversized = AggregatedResearchOutput {
            title: "x".repeat(513),
            citation: "citation".to_owned(),
            topics: vec!["topic".to_owned()],
            summary: "summary".to_owned(),
            key_ideas: vec!["idea".to_owned()],
            implementation_notes: vec!["note".to_owned()],
            critique: "critique".to_owned(),
            evidence_span_ids: vec!["S000001".to_owned()],
        };

        assert!(super::validate_aggregated_output(&oversized).is_err());
    }

    #[test]
    fn source_span_ids_reconstruct_exact_evidence() -> Result<(), Box<dyn std::error::Error>> {
        let document = "First grounded paragraph.\n\nSecond grounded paragraph.";
        let spans = source_spans(document)?;
        let selected = vec![spans[1].id.clone(), spans[0].id.clone()];

        let evidence = evidence_from_span_ids(&selected, &spans)?;

        assert_eq!(spans[0].id, "S000001");
        assert_eq!(spans[0].location, "characters 0-24");
        assert_eq!(spans[1].id, "S000002");
        assert_eq!(spans[1].location, "characters 27-52");
        assert_eq!(evidence[0].quote, spans[1].text);
        assert_eq!(evidence[0].location, spans[1].location);
        assert_eq!(evidence[1].quote, spans[0].text);
        assert!(evidence.iter().all(|item| document.contains(&item.quote)));
        assert!(evidence_from_span_ids(&["unknown".to_owned()], &spans).is_err());
        Ok(())
    }
}
