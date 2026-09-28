//! Typed BAML adapter for local document classification and distillation.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::baml_client::B;
use crate::baml_client::types::DocumentSourceType as BamlDocumentSourceType;
use crate::baml_client::types::{
    AggregatedResearchOutput, ClaimKind as BamlClaimKind,
    DocumentClassification as BamlClassification, DocumentIntelligenceOutput,
    EntityKind as BamlEntityKind, IntelligenceChunkOutput, ResearchChunkOutput,
    ResearchDraftOutput, SemanticRelationType as BamlRelationType, SourceSpanInput,
};
use crate::config::{ClassifierProfile, LocalModelEndpoint};
use crate::domain::{
    AnalysisProvenance, ClaimKind, ClassificationFailure, ClassificationFailureCode,
    DocumentClassification, DocumentIntelligence, DocumentSourceType, EntityKind,
    EvidenceReference, ExtractedDocument, IntelligenceClaim, IntelligenceEntity,
    IntelligenceSummary, ResearchDraft, SemanticRelation, SemanticRelationType,
};
use crate::ports::{
    AnalysisError, DocumentIntelligenceAnalyzer, ProfileClassifier, ResearchAnalyzer,
};

const DISTILLATION_CHUNK_CHARACTERS: usize = 12_000;
const MAX_DISTILLATION_CHUNKS: usize = 24;
const MAX_CONCURRENT_CHUNKS: usize = 2;
const MAX_AGGREGATE_FINDINGS_CHARACTERS: usize = 60_000;
const EVIDENCE_SPAN_CHARACTERS: usize = 500;
const CLASSIFICATION_OMISSION_MARKER: &str = "\n\n[... middle omitted for classification ...]\n\n";

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
    timeout: Duration,
}

impl BamlDocumentClassifier {
    /// Creates a classifier using a validated local endpoint.
    #[must_use]
    pub const fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

#[async_trait]
impl ProfileClassifier for BamlDocumentClassifier {
    async fn classify_with_profile(
        &self,
        document: &ExtractedDocument,
        profile: &ClassifierProfile,
    ) -> Result<DocumentClassification, ClassificationFailure> {
        let input = bounded_classification_input(document.text(), profile.maximum_input_characters);
        let classification = classify_with_baml(&profile.endpoint, self.timeout, &input)
            .await
            .map_err(|error| classification_failure(&error))?;
        Ok(DocumentClassification {
            title: classification.title,
            authors: classification.authors,
            source_type: document_source_type(&classification.source_type),
            language_code: classification.language_code,
            topics: classification.topics,
        })
    }
}

fn bounded_classification_input(document: &str, maximum_characters: usize) -> Cow<'_, str> {
    if document.chars().count() <= maximum_characters {
        return Cow::Borrowed(document);
    }
    let marker_characters = CLASSIFICATION_OMISSION_MARKER.chars().count();
    if maximum_characters <= marker_characters {
        return Cow::Owned(document.chars().take(maximum_characters).collect());
    }
    let available = maximum_characters - marker_characters;
    let head_characters = available * 4 / 5;
    let tail_characters = available - head_characters;
    let head = document.chars().take(head_characters).collect::<String>();
    let tail = document
        .chars()
        .rev()
        .take(tail_characters)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    Cow::Owned(format!("{head}{CLASSIFICATION_OMISSION_MARKER}{tail}"))
}

fn classification_failure(error: &AnalysisError) -> ClassificationFailure {
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("context")
        || message.contains("token limit")
        || message.contains("too many tokens")
    {
        ClassificationFailure::retryable(
            ClassificationFailureCode::ContextOverflow,
            "classifier context window was exceeded",
        )
    } else if message.contains("connect")
        || message.contains("timeout")
        || message.contains("unavailable")
    {
        ClassificationFailure::retryable(
            ClassificationFailureCode::EndpointUnavailable,
            "local classifier endpoint was unavailable",
        )
    } else {
        ClassificationFailure::retryable(
            ClassificationFailureCode::ModelRequest,
            "local classifier request failed",
        )
    }
}

const fn document_source_type(source_type: &BamlDocumentSourceType) -> DocumentSourceType {
    match source_type {
        BamlDocumentSourceType::ResearchPaper => DocumentSourceType::ResearchPaper,
        BamlDocumentSourceType::Report => DocumentSourceType::Report,
        BamlDocumentSourceType::Article => DocumentSourceType::Article,
        BamlDocumentSourceType::Documentation => DocumentSourceType::Documentation,
        BamlDocumentSourceType::Website => DocumentSourceType::Website,
        BamlDocumentSourceType::SourceCode => DocumentSourceType::SourceCode,
        BamlDocumentSourceType::Repository => DocumentSourceType::Repository,
        BamlDocumentSourceType::Specification => DocumentSourceType::Specification,
        BamlDocumentSourceType::Tutorial => DocumentSourceType::Tutorial,
        BamlDocumentSourceType::PersonalProfile => DocumentSourceType::PersonalProfile,
        BamlDocumentSourceType::Other => DocumentSourceType::Other,
    }
}

/// Runs Episteme's typed BAML functions against configured local clients.
#[derive(Debug, Clone)]
pub struct BamlResearchAnalyzer {
    classifier: LocalModelEndpoint,
    distiller: LocalModelEndpoint,
    timeout: Duration,
}

/// Runs evidence-grounded document intelligence map/reduce locally.
#[derive(Debug, Clone)]
pub struct BamlDocumentIntelligenceAnalyzer {
    endpoint: LocalModelEndpoint,
    timeout: Duration,
}

impl BamlDocumentIntelligenceAnalyzer {
    /// Creates an intelligence analyzer for one local endpoint and request timeout.
    #[must_use]
    pub const fn new(endpoint: LocalModelEndpoint, timeout: Duration) -> Self {
        Self { endpoint, timeout }
    }
}

#[async_trait]
impl DocumentIntelligenceAnalyzer for BamlDocumentIntelligenceAnalyzer {
    async fn analyze_intelligence(
        &self,
        document: &ExtractedDocument,
        classification: &DocumentClassification,
    ) -> Result<DocumentIntelligence, AnalysisError> {
        let baml_classification = to_baml_classification(classification);
        let chunks = chunk_document(document.text())?;
        let mut outputs = Vec::new();
        let all_spans = chunks
            .iter()
            .flat_map(|chunk| chunk.spans.iter().cloned())
            .collect::<Vec<_>>();
        for chunk in chunks {
            let spans = chunk
                .spans
                .iter()
                .map(|span| SourceSpanInput {
                    id: span.id.clone(),
                    text: span.text.clone(),
                    location: span.location.clone(),
                })
                .collect::<Vec<_>>();
            let mut output = B
                .ExtractIntelligenceChunk
                .with_env_var(
                    "EPISTEME_DISTILL_BASE_URL",
                    self.endpoint.base_url().as_str(),
                )
                .with_env_var("EPISTEME_DISTILL_MODEL", self.endpoint.model())
                .with_cancellation_token(Some(baml::CancellationToken::new_with_timeout(
                    self.timeout,
                )))
                .call(&spans, &baml_classification)
                .await
                .map_err(|_| {
                    AnalysisError::Model("intelligence chunk request failed".to_owned())
                })?;
            namespace_intelligence_chunk(chunk.index, &mut output);
            outputs.push(output);
        }
        let output = B
            .AggregateDocumentIntelligence
            .with_env_var(
                "EPISTEME_DISTILL_BASE_URL",
                self.endpoint.base_url().as_str(),
            )
            .with_env_var("EPISTEME_DISTILL_MODEL", self.endpoint.model())
            .with_cancellation_token(Some(baml::CancellationToken::new_with_timeout(
                self.timeout,
            )))
            .call(&baml_classification, &outputs)
            .await
            .map_err(|_| AnalysisError::Model("intelligence aggregation failed".to_owned()))?;
        intelligence_from_output(document, classification, output, &all_spans, &self.endpoint)
    }
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

fn namespace_intelligence_chunk(index: usize, output: &mut IntelligenceChunkOutput) {
    let prefix = format!("chunk-{index}-");
    for claim in &mut output.claims {
        let old = claim.local_id.clone();
        claim.local_id = format!("{prefix}{old}");
        for relation in &mut output.relations {
            if relation.source_local_id == old {
                relation.source_local_id.clone_from(&claim.local_id);
            }
            if relation.target_local_id == old {
                relation.target_local_id.clone_from(&claim.local_id);
            }
        }
    }
    for entity in &mut output.entities {
        let old = entity.local_id.clone();
        entity.local_id = format!("{prefix}{old}");
        for relation in &mut output.relations {
            if relation.source_local_id == old {
                relation.source_local_id.clone_from(&entity.local_id);
            }
            if relation.target_local_id == old {
                relation.target_local_id.clone_from(&entity.local_id);
            }
        }
    }
}

fn intelligence_from_output(
    document: &ExtractedDocument,
    classification: &DocumentClassification,
    output: DocumentIntelligenceOutput,
    spans: &[SourceSpan],
    endpoint: &LocalModelEndpoint,
) -> Result<DocumentIntelligence, AnalysisError> {
    let DocumentIntelligenceOutput {
        summary,
        key_points,
        summary_span_ids,
        claims: output_claims,
        entities: output_entities,
        relations: output_relations,
    } = output;
    let mut node_ids = HashMap::new();
    let mut claim_ids = HashSet::new();
    let mut entity_ids = HashSet::new();
    let mut claims = Vec::new();
    for claim in output_claims {
        let evidence = bounded_evidence(&claim.evidence_span_ids, spans, 8)?;
        let confidence = u8::try_from(claim.confidence_percent)
            .map_err(|_| AnalysisError::Invalid("claim confidence was invalid".to_owned()))?;
        let value =
            IntelligenceClaim::new(claim.text, claim_kind(&claim.kind), confidence, evidence)
                .map_err(|error| AnalysisError::Invalid(error.to_string()))?;
        if node_ids.insert(claim.local_id, value.id.clone()).is_some() {
            return Err(AnalysisError::Invalid(
                "duplicate intelligence node id".to_owned(),
            ));
        }
        claim_ids.insert(value.id.clone());
        claims.push(value);
    }
    let mut entities = Vec::new();
    for entity in output_entities {
        let evidence = bounded_evidence(&entity.evidence_span_ids, spans, 8)?;
        let mut value = IntelligenceEntity::new(
            entity.name,
            entity_kind(&entity.kind),
            entity.description,
            evidence,
        )
        .map_err(|error| AnalysisError::Invalid(error.to_string()))?;
        value.aliases = entity
            .aliases
            .into_iter()
            .map(|alias| alias.trim().to_owned())
            .filter(|alias| !alias.is_empty())
            .take(32)
            .collect();
        if node_ids.insert(entity.local_id, value.id.clone()).is_some() {
            return Err(AnalysisError::Invalid(
                "duplicate intelligence node id".to_owned(),
            ));
        }
        entity_ids.insert(value.id.clone());
        entities.push(value);
    }
    let mut relations = Vec::new();
    for relation in output_relations {
        let source_id = node_ids
            .get(&relation.source_local_id)
            .cloned()
            .ok_or_else(|| {
                AnalysisError::Invalid("unknown intelligence relation source".to_owned())
            })?;
        let target_id = node_ids
            .get(&relation.target_local_id)
            .cloned()
            .ok_or_else(|| {
                AnalysisError::Invalid("unknown intelligence relation target".to_owned())
            })?;
        let relation_type = relation_type(&relation.relation_type);
        let valid_endpoint_kinds = match relation_type {
            SemanticRelationType::Supports | SemanticRelationType::Contradicts => {
                claim_ids.contains(&source_id) && claim_ids.contains(&target_id)
            }
            _ => entity_ids.contains(&source_id) && entity_ids.contains(&target_id),
        };
        if !valid_endpoint_kinds {
            continue;
        }
        let confidence = u8::try_from(relation.confidence_percent)
            .map_err(|_| AnalysisError::Invalid("relation confidence was invalid".to_owned()))?;
        relations.push(
            SemanticRelation::new(
                source_id,
                target_id,
                relation_type,
                confidence,
                bounded_evidence(&relation.evidence_span_ids, spans, 8)?,
                document.source().digest(),
            )
            .map_err(|error| AnalysisError::Invalid(error.to_string()))?,
        );
    }
    let summary = intelligence_summary(summary, key_points, &summary_span_ids, spans)?;
    finish_intelligence(
        document,
        classification,
        endpoint,
        summary,
        claims,
        entities,
        relations,
    )
}

fn intelligence_summary(
    text: String,
    key_points: Vec<String>,
    span_ids: &[String],
    spans: &[SourceSpan],
) -> Result<IntelligenceSummary, AnalysisError> {
    Ok(IntelligenceSummary {
        text,
        key_points,
        evidence: bounded_evidence(span_ids, spans, 12)?,
    })
}

fn finish_intelligence(
    document: &ExtractedDocument,
    classification: &DocumentClassification,
    endpoint: &LocalModelEndpoint,
    summary: IntelligenceSummary,
    claims: Vec<IntelligenceClaim>,
    entities: Vec<IntelligenceEntity>,
    relations: Vec<SemanticRelation>,
) -> Result<DocumentIntelligence, AnalysisError> {
    DocumentIntelligence {
        source_digest: document.source().digest().clone(),
        classification: classification.clone(),
        summary,
        claims,
        entities,
        relations,
        analysis: AnalysisProvenance {
            function: "DocumentIntelligence".to_owned(),
            client: "LocalDistiller".to_owned(),
            model: endpoint.model().to_owned(),
            pipeline_version: "intelligence-v1".to_owned(),
            processed_at: Utc::now().to_rfc3339(),
        },
    }
    .validate_against(document.text())
    .map_err(|error| AnalysisError::Invalid(error.to_string()))
}

fn bounded_evidence(
    ids: &[String],
    spans: &[SourceSpan],
    maximum: usize,
) -> Result<Vec<EvidenceReference>, AnalysisError> {
    evidence_from_span_ids(
        &ids.iter().take(maximum).cloned().collect::<Vec<_>>(),
        spans,
    )
}

fn to_baml_classification(value: &DocumentClassification) -> BamlClassification {
    BamlClassification {
        title: value.title.clone(),
        authors: value.authors.clone(),
        source_type: baml_document_source_type(value.source_type),
        language_code: value.language_code.clone(),
        topics: value.topics.clone(),
    }
}

const fn baml_document_source_type(value: DocumentSourceType) -> BamlDocumentSourceType {
    match value {
        DocumentSourceType::ResearchPaper => BamlDocumentSourceType::ResearchPaper,
        DocumentSourceType::Report => BamlDocumentSourceType::Report,
        DocumentSourceType::Article => BamlDocumentSourceType::Article,
        DocumentSourceType::Documentation => BamlDocumentSourceType::Documentation,
        DocumentSourceType::Website => BamlDocumentSourceType::Website,
        DocumentSourceType::SourceCode => BamlDocumentSourceType::SourceCode,
        DocumentSourceType::Repository => BamlDocumentSourceType::Repository,
        DocumentSourceType::Specification => BamlDocumentSourceType::Specification,
        DocumentSourceType::Tutorial => BamlDocumentSourceType::Tutorial,
        DocumentSourceType::PersonalProfile => BamlDocumentSourceType::PersonalProfile,
        DocumentSourceType::Other => BamlDocumentSourceType::Other,
    }
}

const fn claim_kind(value: &BamlClaimKind) -> ClaimKind {
    match value {
        BamlClaimKind::Fact => ClaimKind::Fact,
        BamlClaimKind::Inference => ClaimKind::Inference,
        BamlClaimKind::Recommendation => ClaimKind::Recommendation,
        BamlClaimKind::Critique => ClaimKind::Critique,
    }
}

const fn entity_kind(value: &BamlEntityKind) -> EntityKind {
    match value {
        BamlEntityKind::Person => EntityKind::Person,
        BamlEntityKind::Organization => EntityKind::Organization,
        BamlEntityKind::Project => EntityKind::Project,
        BamlEntityKind::Technology => EntityKind::Technology,
        BamlEntityKind::Concept => EntityKind::Concept,
        BamlEntityKind::Method => EntityKind::Method,
        BamlEntityKind::Dataset => EntityKind::Dataset,
        BamlEntityKind::Benchmark => EntityKind::Benchmark,
        BamlEntityKind::Document => EntityKind::Document,
    }
}

const fn relation_type(value: &BamlRelationType) -> SemanticRelationType {
    match value {
        BamlRelationType::Supports => SemanticRelationType::Supports,
        BamlRelationType::Contradicts => SemanticRelationType::Contradicts,
        BamlRelationType::Implements => SemanticRelationType::Implements,
        BamlRelationType::Evaluates => SemanticRelationType::Evaluates,
        BamlRelationType::DependsOn => SemanticRelationType::DependsOn,
        BamlRelationType::Extends => SemanticRelationType::Extends,
        BamlRelationType::Uses => SemanticRelationType::Uses,
        BamlRelationType::Causes => SemanticRelationType::Causes,
        BamlRelationType::PartOf => SemanticRelationType::PartOf,
        BamlRelationType::EvolvesFrom => SemanticRelationType::EvolvesFrom,
    }
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
        .map_err(|error| {
            let message = error.to_string().to_ascii_lowercase();
            let category = if message.contains("context")
                || message.contains("token limit")
                || message.contains("too many tokens")
            {
                "classifier context overflow"
            } else if message.contains("connect")
                || message.contains("timeout")
                || message.contains("unavailable")
            {
                "classifier endpoint unavailable"
            } else {
                "classifier request failed"
            };
            AnalysisError::Model(category.to_owned())
        })
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
