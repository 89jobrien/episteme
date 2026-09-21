//! Domain types and invariants.

use std::collections::HashSet;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A validated lowercase BLAKE3 digest.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SourceDigest(String);

impl SourceDigest {
    /// Computes a digest from source bytes.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(blake3::hash(bytes).to_hex().to_string())
    }

    /// Parses a lowercase hexadecimal BLAKE3 digest.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidSourceDigest`] when the value is not exactly 64 lowercase
    /// hexadecimal characters.
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let valid = value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !valid {
            return Err(DomainError::InvalidSourceDigest);
        }
        Ok(Self(value))
    }

    /// Returns the hexadecimal digest.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SourceDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for SourceDigest {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<SourceDigest> for String {
    fn from(value: SourceDigest) -> Self {
        value.0
    }
}

/// A source filename safe to join beneath a trusted root.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SourceFileName(String);

impl SourceFileName {
    /// Validates a single filename without path traversal or control characters.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::UnsafeSourceFileName`] when the value is empty, contains path
    /// components, separators, or control characters.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let path = Path::new(&value);
        let one_normal_component = {
            let mut components = path.components();
            matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
        };
        let safe = !value.trim().is_empty()
            && value != "."
            && value != ".."
            && !value.contains(['/', '\\'])
            && !value.chars().any(char::is_control)
            && one_normal_component;
        if !safe {
            return Err(DomainError::UnsafeSourceFileName(value));
        }
        Ok(Self(value))
    }

    /// Returns the validated filename.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SourceFileName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for SourceFileName {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<SourceFileName> for String {
    fn from(value: SourceFileName) -> Self {
        value.0
    }
}

/// The durable progress point for one ingestion run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestionStage {
    /// The source has been accepted but not extracted.
    Discovered,
    /// Deterministic text extraction completed.
    Extracted,
    /// Local model analysis completed.
    Analyzed,
    /// A research draft was written to the vault.
    NoteWritten,
    /// The original source was archived.
    Archived,
    /// The vault index was refreshed.
    Indexed,
    /// A retryable failure interrupted processing.
    FailedRetryable,
    /// Human review is required before continuing.
    NeedsReview,
}

impl IngestionStage {
    /// Returns the stable database representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::Extracted => "extracted",
            Self::Analyzed => "analyzed",
            Self::NoteWritten => "note_written",
            Self::Archived => "archived",
            Self::Indexed => "indexed",
            Self::FailedRetryable => "failed_retryable",
            Self::NeedsReview => "needs_review",
        }
    }
}

impl FromStr for IngestionStage {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "discovered" => Ok(Self::Discovered),
            "extracted" => Ok(Self::Extracted),
            "analyzed" => Ok(Self::Analyzed),
            "note_written" => Ok(Self::NoteWritten),
            "archived" => Ok(Self::Archived),
            "indexed" => Ok(Self::Indexed),
            "failed_retryable" => Ok(Self::FailedRetryable),
            "needs_review" => Ok(Self::NeedsReview),
            _ => Err(DomainError::InvalidIngestionStage(value.to_owned())),
        }
    }
}

/// Rebuildable provenance for one source document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestionRun {
    /// Source content identity.
    pub digest: SourceDigest,
    /// Original source filename.
    pub source_name: SourceFileName,
    /// Current durable stage.
    pub stage: IngestionStage,
    /// Vault-relative generated note path, when available.
    pub note_path: Option<String>,
    /// Vault-relative archived source path, when available.
    pub archive_path: Option<String>,
    /// Sanitized failure summary, when available.
    pub error: Option<String>,
}

/// Supported source document formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    /// A Portable Document Format source.
    Pdf,
    /// An HTML source document.
    Html,
    /// A raster image requiring OCR.
    Image,
}

/// The deterministic method used to extract text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionMethod {
    /// Direct PDF text extraction.
    PdfText,
    /// PDF rendering followed by OCR.
    PdfOcr,
    /// HTML conversion through Pandoc.
    HtmlPandoc,
    /// Image OCR through Tesseract.
    ImageOcr,
}

impl ExtractionMethod {
    /// Returns the stable frontmatter representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PdfText => "pdf_text",
            Self::PdfOcr => "pdf_ocr",
            Self::HtmlPandoc => "html_pandoc",
            Self::ImageOcr => "image_ocr",
        }
    }
}

impl FromStr for ExtractionMethod {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pdf_text" => Ok(Self::PdfText),
            "pdf_ocr" => Ok(Self::PdfOcr),
            "html_pandoc" => Ok(Self::HtmlPandoc),
            "image_ocr" => Ok(Self::ImageOcr),
            _ => Err(DomainError::InvalidExtractionMethod(value.to_owned())),
        }
    }
}

/// A validated source staged for deterministic extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedSource {
    path: PathBuf,
    original_path: PathBuf,
    digest: SourceDigest,
    source_name: SourceFileName,
    kind: DocumentKind,
}

impl StagedSource {
    /// Creates staged source metadata.
    #[must_use]
    pub fn new(
        path: PathBuf,
        digest: SourceDigest,
        source_name: SourceFileName,
        kind: DocumentKind,
    ) -> Self {
        Self {
            original_path: path.clone(),
            path,
            digest,
            source_name,
            kind,
        }
    }

    /// Creates source metadata with separate immutable processing and original paths.
    #[must_use]
    pub fn with_original(
        path: PathBuf,
        original_path: PathBuf,
        digest: SourceDigest,
        source_name: SourceFileName,
        kind: DocumentKind,
    ) -> Self {
        Self {
            path,
            original_path,
            digest,
            source_name,
            kind,
        }
    }

    /// Returns the staged source path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the original inbox path.
    #[must_use]
    pub fn original_path(&self) -> &Path {
        &self.original_path
    }

    /// Returns the source digest.
    #[must_use]
    pub fn digest(&self) -> &SourceDigest {
        &self.digest
    }

    /// Returns the safe original filename.
    #[must_use]
    pub fn source_name(&self) -> &SourceFileName {
        &self.source_name
    }

    /// Returns the source format.
    #[must_use]
    pub const fn kind(&self) -> DocumentKind {
        self.kind
    }
}

/// Extracted text with deterministic provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedDocument {
    source: StagedSource,
    text: String,
    method: ExtractionMethod,
}

/// Typed metadata inferred from one source document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentClassification {
    /// Source-supported document title.
    pub title: String,
    /// Source-supported author names, when present.
    pub authors: Vec<String>,
    /// Broad source format or publication category.
    pub source_type: DocumentSourceType,
    /// Detected human-language ISO 639-1 code.
    pub language_code: String,
    /// Topic labels supported by the source.
    pub topics: Vec<String>,
}

/// Canonical document source category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentSourceType {
    /// Academic or scientific paper.
    ResearchPaper,
    /// Technical, operational, or analytical report.
    Report,
    /// Article or essay.
    Article,
    /// Reference or explanatory documentation.
    Documentation,
    /// Web page that does not fit a narrower category.
    Website,
    /// Source code or code listing.
    SourceCode,
    /// Software repository overview.
    Repository,
    /// Formal specification or request for comments.
    Specification,
    /// Tutorial or guided learning material.
    Tutorial,
    /// Professional profile or persona report.
    PersonalProfile,
    /// Source whose category cannot be established safely.
    Other,
}

impl DocumentSourceType {
    /// Returns the stable persisted representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ResearchPaper => "research_paper",
            Self::Report => "report",
            Self::Article => "article",
            Self::Documentation => "documentation",
            Self::Website => "website",
            Self::SourceCode => "source_code",
            Self::Repository => "repository",
            Self::Specification => "specification",
            Self::Tutorial => "tutorial",
            Self::PersonalProfile => "personal_profile",
            Self::Other => "other",
        }
    }
}

impl FromStr for DocumentSourceType {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "research_paper" => Ok(Self::ResearchPaper),
            "report" => Ok(Self::Report),
            "article" => Ok(Self::Article),
            "documentation" => Ok(Self::Documentation),
            "website" => Ok(Self::Website),
            "source_code" => Ok(Self::SourceCode),
            "repository" => Ok(Self::Repository),
            "specification" => Ok(Self::Specification),
            "tutorial" => Ok(Self::Tutorial),
            "personal_profile" => Ok(Self::PersonalProfile),
            "other" => Ok(Self::Other),
            _ => Err(DomainError::InvalidClassification),
        }
    }
}

/// Persisted classification with source, extraction, and inference provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassificationRecord {
    /// Source content identity.
    pub source_digest: SourceDigest,
    /// Original source filename.
    pub source_name: SourceFileName,
    /// Deterministic extraction method.
    pub extraction_method: ExtractionMethod,
    /// Validated document metadata.
    pub classification: DocumentClassification,
    /// Local inference provenance.
    pub analysis: AnalysisProvenance,
}

/// Safe category for one classification failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationFailureCode {
    /// Source staging failed.
    Staging,
    /// Deterministic extraction failed.
    Extraction,
    /// A configured local endpoint was unavailable.
    EndpointUnavailable,
    /// The endpoint did not advertise the configured model.
    ModelUnavailable,
    /// No profile accepted the extracted input size.
    InputTooLarge,
    /// The model rejected input exceeding its context.
    ContextOverflow,
    /// A local model request failed for another reason.
    ModelRequest,
    /// Generated output violated classification policy.
    InvalidOutput,
    /// Derived-state persistence failed.
    Persistence,
}

impl ClassificationFailureCode {
    /// Returns the stable persisted representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Staging => "staging",
            Self::Extraction => "extraction",
            Self::EndpointUnavailable => "endpoint_unavailable",
            Self::ModelUnavailable => "model_unavailable",
            Self::InputTooLarge => "input_too_large",
            Self::ContextOverflow => "context_overflow",
            Self::ModelRequest => "model_request",
            Self::InvalidOutput => "invalid_output",
            Self::Persistence => "persistence",
        }
    }
}

impl FromStr for ClassificationFailureCode {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "staging" => Ok(Self::Staging),
            "extraction" => Ok(Self::Extraction),
            "endpoint_unavailable" => Ok(Self::EndpointUnavailable),
            "model_unavailable" => Ok(Self::ModelUnavailable),
            "input_too_large" => Ok(Self::InputTooLarge),
            "context_overflow" => Ok(Self::ContextOverflow),
            "model_request" => Ok(Self::ModelRequest),
            "invalid_output" => Ok(Self::InvalidOutput),
            "persistence" => Ok(Self::Persistence),
            _ => Err(DomainError::InvalidClassification),
        }
    }
}

/// Content-free failure returned by classification ports and services.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[error("{message}")]
pub struct ClassificationFailure {
    /// Stable machine-readable category.
    pub code: ClassificationFailureCode,
    /// Bounded message without source content, response bodies, paths, or backtraces.
    pub message: String,
    /// Whether retrying this operation may succeed.
    pub retryable: bool,
    /// Selected model when a model attempt occurred.
    pub model: Option<String>,
    /// Extracted Unicode scalar count when known.
    pub extracted_characters: Option<u64>,
}

impl ClassificationFailure {
    /// Creates a retryable content-free failure.
    #[must_use]
    pub fn retryable(code: ClassificationFailureCode, message: impl AsRef<str>) -> Self {
        Self::new(code, message.as_ref(), true)
    }

    /// Creates a terminal content-free failure.
    #[must_use]
    pub fn terminal(code: ClassificationFailureCode, message: impl AsRef<str>) -> Self {
        Self::new(code, message.as_ref(), false)
    }

    fn new(code: ClassificationFailureCode, message: &str, retryable: bool) -> Self {
        let message = message
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .take(512)
            .collect::<String>();
        Self {
            code,
            message,
            retryable,
            model: None,
            extracted_characters: None,
        }
    }

    /// Adds safe model and input-size context to this failure.
    #[must_use]
    pub fn with_context(mut self, model: impl Into<String>, extracted_characters: u64) -> Self {
        self.model = Some(model.into());
        self.extracted_characters = Some(extracted_characters);
        self
    }

    /// Adds safe input-size context when no model was selected.
    #[must_use]
    pub const fn with_extracted_characters(mut self, extracted_characters: u64) -> Self {
        self.extracted_characters = Some(extracted_characters);
        self
    }
}

/// Successful classification plus selected-model provenance and metrics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassificationOutcome {
    /// Normalized classification metadata.
    pub classification: DocumentClassification,
    /// Selected local model provenance.
    pub analysis: AnalysisProvenance,
    /// Number of extracted Unicode scalar values sent for classification.
    pub extracted_characters: u64,
}

/// Identifies one reusable model/policy classification result.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClassificationCacheKey {
    /// Selected model identifier.
    pub model: String,
    /// Classification contract version.
    pub policy_version: String,
}

/// Durable status of one classification attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationAttemptStatus {
    /// Work is known but not started.
    Pending,
    /// Work is currently executing.
    Running,
    /// Inference succeeded and was persisted.
    Succeeded,
    /// Work failed and may succeed on retry.
    FailedRetryable,
    /// Work failed terminally.
    FailedTerminal,
    /// A matching versioned cache record was reused.
    Cached,
}

impl ClassificationAttemptStatus {
    /// Returns the stable persisted representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::FailedRetryable => "failed_retryable",
            Self::FailedTerminal => "failed_terminal",
            Self::Cached => "cached",
        }
    }
}

impl FromStr for ClassificationAttemptStatus {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed_retryable" => Ok(Self::FailedRetryable),
            "failed_terminal" => Ok(Self::FailedTerminal),
            "cached" => Ok(Self::Cached),
            _ => Err(DomainError::InvalidClassification),
        }
    }
}

/// Durable, content-free metrics for one batch attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassificationAttemptRecord {
    /// Batch identity.
    pub batch_id: String,
    /// Inbox-relative source path.
    pub source_path: String,
    /// Source content identity when staging succeeded.
    pub source_digest: Option<SourceDigest>,
    /// One-based attempt number for this source in this batch.
    pub attempt: u32,
    /// Durable attempt status.
    pub status: ClassificationAttemptStatus,
    /// Selected model when known.
    pub model: Option<String>,
    /// Extracted Unicode scalar count when known.
    pub extracted_characters: Option<u64>,
    /// Attempt wall-clock duration.
    pub duration_ms: u64,
    /// Safe machine-readable failure category.
    pub failure_code: Option<ClassificationFailureCode>,
    /// Bounded content-free failure message.
    pub failure_message: Option<String>,
    /// RFC 3339 update timestamp.
    pub updated_at: String,
}

/// Reconciled final status for one classification batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassificationBatchSummary {
    /// Batch identity.
    pub batch_id: String,
    /// Number of discovered sources.
    pub total: usize,
    /// Number newly classified successfully.
    pub succeeded: usize,
    /// Number served by matching versioned cache entries.
    pub cached: usize,
    /// Number with retryable final failures.
    pub failed_retryable: usize,
    /// Number with terminal final failures.
    pub failed_terminal: usize,
    /// Latest attempt per source, sorted by relative path.
    pub attempts: Vec<ClassificationAttemptRecord>,
}

/// One safely staged or rejected source discovered for batch classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassificationBatchSource {
    /// A source ready for extraction and inference.
    Ready {
        /// Inbox-relative source path.
        relative_path: String,
        /// Immutable staged source.
        source: StagedSource,
    },
    /// A source rejected during discovery or staging.
    Rejected {
        /// Inbox-relative source path.
        relative_path: String,
        /// Content-free staging failure.
        failure: ClassificationFailure,
    },
}

/// Semantic category of one extracted claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimKind {
    Fact,
    Inference,
    Recommendation,
    Critique,
}

impl ClaimKind {
    /// Returns the stable serialized claim-kind label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Inference => "inference",
            Self::Recommendation => "recommendation",
            Self::Critique => "critique",
        }
    }
}

/// Canonical graph entity category.
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

impl EntityKind {
    /// Returns the stable serialized entity-kind label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Organization => "organization",
            Self::Project => "project",
            Self::Technology => "technology",
            Self::Concept => "concept",
            Self::Method => "method",
            Self::Dataset => "dataset",
            Self::Benchmark => "benchmark",
            Self::Document => "document",
        }
    }
}

/// Canonical semantic edge category.
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

impl SemanticRelationType {
    /// Returns the stable serialized relation-type label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supports => "supports",
            Self::Contradicts => "contradicts",
            Self::Implements => "implements",
            Self::Evaluates => "evaluates",
            Self::DependsOn => "depends_on",
            Self::Extends => "extends",
            Self::Uses => "uses",
            Self::Causes => "causes",
            Self::PartOf => "part_of",
            Self::EvolvesFrom => "evolves_from",
        }
    }
}

/// Grounded document summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelligenceSummary {
    pub text: String,
    pub key_points: Vec<String>,
    pub evidence: Vec<EvidenceReference>,
}

/// One evidence-grounded claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelligenceClaim {
    pub id: String,
    pub text: String,
    pub kind: ClaimKind,
    pub confidence_percent: u8,
    pub evidence: Vec<EvidenceReference>,
}

impl IntelligenceClaim {
    /// Creates a normalized grounded claim.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidIntelligence`] for invalid confidence, text, or evidence.
    pub fn new(
        text: impl AsRef<str>,
        kind: ClaimKind,
        confidence_percent: u8,
        evidence: Vec<EvidenceReference>,
    ) -> Result<Self, DomainError> {
        let text = normalize_whitespace(text.as_ref());
        validate_intelligence_field(&text, 2_000, confidence_percent, &evidence)?;
        Ok(Self {
            id: Self::stable_id(&text),
            text,
            kind,
            confidence_percent,
            evidence,
        })
    }

    /// Derives an identifier from normalized, case-folded claim text.
    #[must_use]
    pub fn stable_id(text: &str) -> String {
        stable_graph_id("claim", &normalize_whitespace(text).to_ascii_lowercase())
    }
}

/// One canonical evidence-grounded entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelligenceEntity {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub kind: EntityKind,
    pub description: String,
    pub evidence: Vec<EvidenceReference>,
}

impl IntelligenceEntity {
    /// Creates a normalized grounded entity.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidIntelligence`] for invalid names, descriptions, or evidence.
    pub fn new(
        name: impl AsRef<str>,
        kind: EntityKind,
        description: impl AsRef<str>,
        evidence: Vec<EvidenceReference>,
    ) -> Result<Self, DomainError> {
        let name = normalize_whitespace(name.as_ref());
        let description = normalize_whitespace(description.as_ref());
        validate_intelligence_field(&name, 512, 100, &evidence)?;
        if !valid_metadata(&description, 2_000) {
            return Err(DomainError::InvalidIntelligence);
        }
        Ok(Self {
            id: Self::stable_id(kind, &name),
            name,
            aliases: Vec::new(),
            kind,
            description,
            evidence,
        })
    }

    /// Derives an identifier from the entity kind and normalized, case-folded name.
    #[must_use]
    pub fn stable_id(kind: EntityKind, name: &str) -> String {
        stable_graph_id(
            kind.as_str(),
            &normalize_whitespace(name).to_ascii_lowercase(),
        )
    }
}

/// One evidence-grounded semantic graph edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticRelation {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation_type: SemanticRelationType,
    pub confidence_percent: u8,
    pub evidence: Vec<EvidenceReference>,
}

impl SemanticRelation {
    /// Creates a normalized grounded semantic relation.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidIntelligence`] for invalid endpoints, confidence, or evidence.
    pub fn new(
        source_id: String,
        target_id: String,
        relation_type: SemanticRelationType,
        confidence_percent: u8,
        evidence: Vec<EvidenceReference>,
        source_digest: &SourceDigest,
    ) -> Result<Self, DomainError> {
        if source_id == target_id || source_id.is_empty() || target_id.is_empty() {
            return Err(DomainError::InvalidIntelligence);
        }
        validate_intelligence_field("relation", 32, confidence_percent, &evidence)?;
        let id = stable_graph_id(
            "relation",
            &format!(
                "{}:{}:{}:{}",
                source_id,
                relation_type.as_str(),
                target_id,
                source_digest.as_str()
            ),
        );
        Ok(Self {
            id,
            source_id,
            target_id,
            relation_type,
            confidence_percent,
            evidence,
        })
    }
}

/// Complete versioned intelligence graph for one document.
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

/// Identifies one reusable intelligence model/policy result.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IntelligenceCacheKey {
    pub model: String,
    pub policy_version: String,
}

impl DocumentIntelligence {
    /// Validates graph integrity and exact source grounding.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidIntelligence`] for unknown nodes or ungrounded evidence.
    pub fn validate_against(self, source: &str) -> Result<Self, DomainError> {
        self.classification.clone().normalize()?;
        if !valid_metadata(&self.summary.text, 8_000)
            || self.summary.key_points.is_empty()
            || self.summary.evidence.is_empty()
        {
            return Err(DomainError::InvalidIntelligence);
        }
        let mut nodes = HashSet::new();
        if self
            .claims
            .iter()
            .any(|claim| !nodes.insert(claim.id.clone()))
            || self
                .entities
                .iter()
                .any(|entity| !nodes.insert(entity.id.clone()))
        {
            return Err(DomainError::InvalidIntelligence);
        }
        if self.relations.iter().any(|relation| {
            !nodes.contains(&relation.source_id) || !nodes.contains(&relation.target_id)
        }) {
            return Err(DomainError::InvalidIntelligence);
        }
        let evidence = self
            .summary
            .evidence
            .iter()
            .chain(self.claims.iter().flat_map(|claim| &claim.evidence))
            .chain(self.entities.iter().flat_map(|entity| &entity.evidence))
            .chain(
                self.relations
                    .iter()
                    .flat_map(|relation| &relation.evidence),
            );
        if evidence.into_iter().any(|item| {
            item.quote.trim().is_empty()
                || item.location.trim().is_empty()
                || !source.contains(&item.quote)
        }) {
            return Err(DomainError::InvalidIntelligence);
        }
        Ok(self)
    }
}

fn validate_intelligence_field(
    text: &str,
    maximum_characters: usize,
    confidence_percent: u8,
    evidence: &[EvidenceReference],
) -> Result<(), DomainError> {
    if !valid_metadata(text, maximum_characters)
        || !(60..=100).contains(&confidence_percent)
        || evidence.is_empty()
    {
        return Err(DomainError::InvalidIntelligence);
    }
    Ok(())
}

fn stable_graph_id(namespace: &str, value: &str) -> String {
    blake3::hash(format!("{namespace}:{value}").as_bytes())
        .to_hex()
        .to_string()
}

/// Models advertised by one local endpoint probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelAvailability {
    /// Model identifiers returned by the endpoint, or empty when not exposed.
    pub advertised_models: Vec<String>,
}

/// A source location and excerpt supporting generated research content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceReference {
    /// A verbatim excerpt from extracted source text.
    pub quote: String,
    /// A page, section, or deterministic text location.
    pub location: String,
}

/// A validated research note draft produced by local analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchDraft {
    /// Proposed note title.
    pub title: String,
    /// Source citation text.
    pub citation: String,
    /// Topic labels.
    pub topics: Vec<String>,
    /// Concise source summary.
    pub summary: String,
    /// Important source-backed ideas.
    pub key_ideas: Vec<String>,
    /// Potential implementation relevance.
    pub implementation_notes: Vec<String>,
    /// Critical assessment of the source.
    pub critique: String,
    /// Evidence supporting generated content.
    pub evidence: Vec<EvidenceReference>,
}

/// Reproducibility metadata for local BAML analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisProvenance {
    /// BAML function responsible for the final draft.
    pub function: String,
    /// Configured local BAML client.
    pub client: String,
    /// Configured local model identifier.
    pub model: String,
    /// Episteme pipeline version.
    pub pipeline_version: String,
    /// RFC 3339 processing timestamp supplied by the application boundary.
    pub processed_at: String,
}

/// Complete input for rendering a source-backed research note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchNote {
    /// Validated generated content.
    pub draft: ResearchDraft,
    /// Source content digest.
    pub source_digest: SourceDigest,
    /// Original safe filename.
    pub source_name: SourceFileName,
    /// Deterministic extraction method.
    pub extraction_method: ExtractionMethod,
    /// Local generation provenance.
    pub analysis: AnalysisProvenance,
}

/// A newly persisted vault note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredNote {
    /// Vault-relative note path.
    pub relative_path: PathBuf,
}

/// A source moved into the immutable archive area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedSource {
    /// Vault-relative archived source path.
    pub relative_path: PathBuf,
}

impl ResearchDraft {
    /// Validates that a generated draft is usable and grounded in source evidence.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidResearchDraft`] when required prose or evidence is absent.
    pub fn validate(self) -> Result<Self, DomainError> {
        let required_text = [
            self.title.as_str(),
            self.citation.as_str(),
            self.summary.as_str(),
            self.critique.as_str(),
        ];
        let evidence_valid = !self.evidence.is_empty()
            && self.evidence.iter().all(|reference| {
                !reference.quote.trim().is_empty() && !reference.location.trim().is_empty()
            });
        if required_text.iter().any(|value| value.trim().is_empty()) || !evidence_valid {
            return Err(DomainError::InvalidResearchDraft);
        }
        Ok(self)
    }

    /// Validates the draft and verifies every quoted excerpt against extracted source text.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::UngroundedEvidence`] when a normalized quote is absent from the
    /// normalized extracted document.
    pub fn validate_against(self, source_text: &str) -> Result<Self, DomainError> {
        let draft = self.validate()?;
        let normalized_source = normalize_whitespace(source_text);
        if draft.evidence.iter().any(|reference| {
            let quote = normalize_whitespace(&reference.quote);
            quote.is_empty() || !normalized_source.contains(&quote)
        }) {
            return Err(DomainError::UngroundedEvidence);
        }
        Ok(draft)
    }
}

impl DocumentClassification {
    /// Normalizes bounded metadata and rejects placeholders or prompt leakage.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidClassification`] when metadata is unsafe, structurally
    /// invalid, or contains known schema placeholders.
    pub fn normalize(mut self) -> Result<Self, DomainError> {
        let raw_values = std::iter::once(self.title.as_str())
            .chain(std::iter::once(self.language_code.as_str()))
            .chain(self.authors.iter().map(String::as_str))
            .chain(self.topics.iter().map(String::as_str));
        if raw_values.into_iter().any(|value| {
            value.chars().any(char::is_control)
                || value.chars().count() > 512
                || value.trim().is_empty()
        }) {
            return Err(DomainError::InvalidClassification);
        }

        self.title = normalize_whitespace(&self.title);
        self.language_code = normalize_whitespace(&self.language_code).to_ascii_lowercase();
        self.authors = normalize_unique(self.authors, 64, 256)?;
        self.topics = normalize_unique(self.topics, 64, 128)?
            .into_iter()
            .map(|topic| topic.to_ascii_lowercase())
            .collect();

        let invalid_title = matches!(
            self.title.to_ascii_lowercase().as_str(),
            "string" | "document" | "text" | "title" | "unknown" | "untitled"
        ) || contains_prompt_leakage(&self.title);
        let invalid_language = !is_iso_639_1(&self.language_code);
        let invalid_author = self.authors.iter().any(|author| {
            contains_prompt_leakage(author)
                || matches!(
                    author.to_ascii_lowercase().as_str(),
                    "string" | "author" | "authors" | "unknown"
                )
        });
        let invalid_topic = self.topics.iter().any(|topic| {
            matches!(
                topic.as_str(),
                "string" | "string[]" | "topic" | "topics" | "untrusted source data"
            ) || contains_prompt_leakage(topic)
        });
        if invalid_title
            || invalid_language
            || invalid_author
            || invalid_topic
            || self.topics.is_empty()
        {
            return Err(DomainError::InvalidClassification);
        }
        Ok(self)
    }

    /// Validates required classification fields.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidClassification`] when required scalar fields or topics are
    /// empty, contain control characters, exceed metadata limits, or contain invalid list items.
    pub fn validate(self) -> Result<Self, DomainError> {
        self.normalize()
    }
}

fn normalize_unique(
    values: Vec<String>,
    maximum_items: usize,
    maximum_characters: usize,
) -> Result<Vec<String>, DomainError> {
    if values.len() > maximum_items {
        return Err(DomainError::InvalidClassification);
    }
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for value in values {
        let value = normalize_whitespace(&value);
        if !valid_metadata(&value, maximum_characters) {
            return Err(DomainError::InvalidClassification);
        }
        if seen.insert(value.to_ascii_lowercase()) {
            normalized.push(value);
        }
    }
    Ok(normalized)
}

fn contains_prompt_leakage(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    ['<', '>'].iter().any(|marker| value.contains(*marker))
        || [
            "source_document",
            "source_spans",
            "output_format",
            "untrusted source",
            "schema placeholder",
        ]
        .iter()
        .any(|marker| value.contains(marker))
}

fn is_iso_639_1(value: &str) -> bool {
    const CODES: &str = "aa ab ae af ak am an ar as av ay az ba be bg bh bi bm bn bo br bs ca ce ch co cr cs cu cv cy da de dv dz ee el en eo es et eu fa ff fi fj fo fr fy ga gd gl gn gu gv ha he hi ho hr ht hu hy hz ia id ie ig ii ik io is it iu ja jv ka kg ki kj kk kl km kn ko kr ks ku kv kw ky la lb lg li ln lo lt lu lv mg mh mi mk ml mn mr ms mt my na nb nd ne ng nl nn no nr nv ny oc oj om or os pa pi pl ps pt qu rm rn ro ru rw sa sc sd se sg si sk sl sm sn so sq sr ss st su sv sw ta te tg th ti tk tl tn to tr ts tt tw ty ug uk ur uz ve vi vo wa wo xh yi yo za zh zu";
    value.len() == 2 && CODES.split_ascii_whitespace().any(|code| code == value)
}

impl ExtractedDocument {
    /// Creates extracted content after rejecting empty text.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::EmptyExtraction`] when no non-whitespace text was extracted.
    pub fn new(
        source: StagedSource,
        text: impl Into<String>,
        method: ExtractionMethod,
    ) -> Result<Self, DomainError> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(DomainError::EmptyExtraction);
        }
        Ok(Self {
            source,
            text,
            method,
        })
    }

    /// Returns the staged source metadata.
    #[must_use]
    pub fn source(&self) -> &StagedSource {
        &self.source
    }

    /// Returns extracted text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the extraction method.
    #[must_use]
    pub const fn method(&self) -> ExtractionMethod {
        self.method
    }
}

/// Domain validation failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    /// A digest did not match the BLAKE3 hexadecimal representation.
    #[error("source digest must be 64 lowercase hexadecimal characters")]
    InvalidSourceDigest,
    /// A source filename could escape or corrupt a trusted root.
    #[error("unsafe source filename: {0}")]
    UnsafeSourceFileName(String),
    /// A persisted ingestion stage was not recognized.
    #[error("invalid ingestion stage: {0}")]
    InvalidIngestionStage(String),
    /// Deterministic extraction produced no usable text.
    #[error("document extraction produced no text")]
    EmptyExtraction,
    /// A persisted extraction method was not recognized.
    #[error("invalid extraction method: {0}")]
    InvalidExtractionMethod(String),
    /// A generated research draft lacked required content or evidence.
    #[error("research draft must contain required prose and grounded evidence")]
    InvalidResearchDraft,
    /// A generated classification lacked required metadata.
    #[error("document classification contains invalid or excessive metadata")]
    InvalidClassification,
    /// Generated intelligence was ungrounded or structurally invalid.
    #[error("document intelligence must be grounded and referentially valid")]
    InvalidIntelligence,
    /// Generated evidence did not occur in extracted source text.
    #[error("research draft contains evidence absent from the source")]
    UngroundedEvidence,
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn valid_metadata(value: &str, maximum_characters: usize) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= maximum_characters
        && !value.chars().any(char::is_control)
}
