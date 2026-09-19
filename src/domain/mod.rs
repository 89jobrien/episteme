//! Domain types and invariants.

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
    /// A generated research draft lacked required content or evidence.
    #[error("research draft must contain required prose and grounded evidence")]
    InvalidResearchDraft,
    /// Generated evidence did not occur in extracted source text.
    #[error("research draft contains evidence absent from the source")]
    UngroundedEvidence,
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
