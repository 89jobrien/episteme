//! Hexagonal ports for external document and storage integrations.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

use crate::config::ClassifierProfile;
use crate::domain::{
    ArchivedSource, ClassificationAttemptRecord, ClassificationBatchSummary,
    ClassificationCacheKey, ClassificationFailure, ClassificationOutcome, ClassificationRecord,
    DocumentClassification, DocumentIntelligence, ExtractedDocument, IngestionRun,
    IntelligenceCacheKey, ModelAvailability, ResearchDraft, ResearchNote, SourceDigest,
    StagedSource, StoredNote,
};

/// Primitive document conversion operations supplied by command-line tools.
#[async_trait]
pub trait DocumentTools: Send + Sync {
    /// Extracts embedded text from a PDF.
    async fn pdf_text(&self, source: &Path) -> Result<String, ExtractionError>;
    /// Renders PDF pages and performs OCR.
    async fn pdf_ocr(&self, source: &Path) -> Result<String, ExtractionError>;
    /// Performs OCR over one image.
    async fn image_text(&self, source: &Path) -> Result<String, ExtractionError>;
    /// Converts an HTML source into normalized text.
    async fn html_text(&self, source: &Path) -> Result<String, ExtractionError>;
}

/// Extracts usable text from a staged document.
#[async_trait]
pub trait DocumentExtractor: Send + Sync {
    /// Extracts text and records the deterministic method used.
    async fn extract(&self, source: &StagedSource) -> Result<ExtractedDocument, ExtractionError>;
}

/// Classifies extracted source text through local inference.
#[async_trait]
pub trait DocumentClassifier: Send + Sync {
    /// Returns ordered cache keys eligible for this extracted input size.
    fn cache_keys(&self, extracted_characters: usize) -> Vec<ClassificationCacheKey>;

    /// Produces typed metadata for an extracted document.
    async fn classify(
        &self,
        document: &ExtractedDocument,
    ) -> Result<ClassificationOutcome, ClassificationFailure>;
}

/// Calls one explicit local classifier profile.
#[async_trait]
pub trait ProfileClassifier: Send + Sync {
    /// Produces metadata through exactly one configured profile.
    async fn classify_with_profile(
        &self,
        document: &ExtractedDocument,
        profile: &ClassifierProfile,
    ) -> Result<DocumentClassification, ClassificationFailure>;
}

/// Checks local model endpoint readiness before inference.
#[async_trait]
pub trait ModelProbe: Send + Sync {
    /// Probes one validated loopback-only classifier profile.
    async fn probe(
        &self,
        profile: &ClassifierProfile,
    ) -> Result<ModelAvailability, ClassificationFailure>;
}

/// Produces a typed, source-grounded research draft through local inference.
#[async_trait]
pub trait ResearchAnalyzer: Send + Sync {
    /// Analyzes extracted source text.
    async fn analyze(&self, document: &ExtractedDocument) -> Result<ResearchDraft, AnalysisError>;
}

/// Produces an evidence-grounded summary, claim set, and semantic graph.
#[async_trait]
pub trait DocumentIntelligenceAnalyzer: Send + Sync {
    /// Analyzes one extracted document using its normalized classification.
    async fn analyze_intelligence(
        &self,
        document: &ExtractedDocument,
        classification: &DocumentClassification,
    ) -> Result<DocumentIntelligence, AnalysisError>;
}

/// Stores versioned rebuildable document intelligence graphs.
pub trait DocumentIntelligenceStore: Send + Sync {
    /// Loads one matching versioned intelligence graph.
    ///
    /// # Errors
    ///
    /// Returns [`IntelligenceStoreError`] when persistence fails.
    fn load_intelligence(
        &self,
        digest: &SourceDigest,
        key: &IntelligenceCacheKey,
    ) -> Result<Option<DocumentIntelligence>, IntelligenceStoreError>;

    /// Saves one complete versioned intelligence graph.
    ///
    /// # Errors
    ///
    /// Returns [`IntelligenceStoreError`] when persistence fails.
    fn save_intelligence(
        &self,
        key: &IntelligenceCacheKey,
        intelligence: &DocumentIntelligence,
    ) -> Result<(), IntelligenceStoreError>;
}

/// Persists source-backed notes without overwriting vault content.
pub trait VaultStore: Send + Sync {
    /// Finds an existing note carrying the source digest.
    ///
    /// # Errors
    ///
    /// Returns [`VaultError`] when the vault cannot be searched safely.
    fn find_by_digest(&self, digest: &SourceDigest) -> Result<Option<StoredNote>, VaultError>;

    /// Creates a note atomically using no-clobber semantics.
    ///
    /// # Errors
    ///
    /// Returns [`VaultError`] when rendering or filesystem persistence fails.
    fn create_note(&self, note: &ResearchNote) -> Result<StoredNote, VaultError>;

    /// Archives the original source without overwriting existing content.
    ///
    /// # Errors
    ///
    /// Returns [`VaultError`] when digest verification or filesystem persistence fails.
    fn archive_source(&self, source: &StagedSource) -> Result<ArchivedSource, VaultError>;
}

/// Stores rebuildable ingestion progress.
pub trait IngestionStore: Send + Sync {
    /// Loads the latest progress for a digest.
    ///
    /// # Errors
    ///
    /// Returns [`IngestionStoreError`] when persistence fails.
    fn load(&self, digest: &SourceDigest) -> Result<Option<IngestionRun>, IngestionStoreError>;

    /// Saves the latest durable progress.
    ///
    /// # Errors
    ///
    /// Returns [`IngestionStoreError`] when persistence fails.
    fn save(&self, run: &IngestionRun) -> Result<(), IngestionStoreError>;
}

/// Stores rebuildable document classifications.
pub trait ClassificationStore: Send + Sync {
    /// Loads a classification by content digest.
    ///
    /// # Errors
    ///
    /// Returns [`ClassificationStoreError`] when persistence fails.
    fn load(
        &self,
        digest: &SourceDigest,
        key: &ClassificationCacheKey,
    ) -> Result<Option<ClassificationRecord>, ClassificationStoreError>;

    /// Inserts or replaces one classification record.
    ///
    /// # Errors
    ///
    /// Returns [`ClassificationStoreError`] when persistence fails.
    fn save(
        &self,
        key: &ClassificationCacheKey,
        record: &ClassificationRecord,
    ) -> Result<(), ClassificationStoreError>;
}

/// Stores resumable classification batch state and attempts.
pub trait ClassificationRunStore: Send + Sync {
    /// Starts or resumes one batch.
    ///
    /// # Errors
    ///
    /// Returns [`ClassificationStoreError`] when persistence fails.
    fn begin_batch(&self, batch_id: &str, started_at: &str)
    -> Result<(), ClassificationStoreError>;

    /// Saves one durable attempt checkpoint.
    ///
    /// # Errors
    ///
    /// Returns [`ClassificationStoreError`] when persistence fails.
    fn save_attempt(
        &self,
        attempt: &ClassificationAttemptRecord,
    ) -> Result<(), ClassificationStoreError>;

    /// Returns the latest attempt per source path.
    ///
    /// # Errors
    ///
    /// Returns [`ClassificationStoreError`] when persistence fails.
    fn latest_attempts(
        &self,
        batch_id: &str,
    ) -> Result<Vec<ClassificationAttemptRecord>, ClassificationStoreError>;

    /// Persists final counters and completion time.
    ///
    /// # Errors
    ///
    /// Returns [`ClassificationStoreError`] when persistence fails.
    fn finish_batch(
        &self,
        summary: &ClassificationBatchSummary,
    ) -> Result<(), ClassificationStoreError>;
}

impl<T> ClassificationStore for Arc<T>
where
    T: ClassificationStore + ?Sized,
{
    fn load(
        &self,
        digest: &SourceDigest,
        key: &ClassificationCacheKey,
    ) -> Result<Option<ClassificationRecord>, ClassificationStoreError> {
        (**self).load(digest, key)
    }

    fn save(
        &self,
        key: &ClassificationCacheKey,
        record: &ClassificationRecord,
    ) -> Result<(), ClassificationStoreError> {
        (**self).save(key, record)
    }
}

impl<T> ClassificationRunStore for Arc<T>
where
    T: ClassificationRunStore + ?Sized,
{
    fn begin_batch(
        &self,
        batch_id: &str,
        started_at: &str,
    ) -> Result<(), ClassificationStoreError> {
        (**self).begin_batch(batch_id, started_at)
    }

    fn save_attempt(
        &self,
        attempt: &ClassificationAttemptRecord,
    ) -> Result<(), ClassificationStoreError> {
        (**self).save_attempt(attempt)
    }

    fn latest_attempts(
        &self,
        batch_id: &str,
    ) -> Result<Vec<ClassificationAttemptRecord>, ClassificationStoreError> {
        (**self).latest_attempts(batch_id)
    }

    fn finish_batch(
        &self,
        summary: &ClassificationBatchSummary,
    ) -> Result<(), ClassificationStoreError> {
        (**self).finish_batch(summary)
    }
}

impl<T> DocumentIntelligenceStore for Arc<T>
where
    T: DocumentIntelligenceStore + ?Sized,
{
    fn load_intelligence(
        &self,
        digest: &SourceDigest,
        key: &IntelligenceCacheKey,
    ) -> Result<Option<DocumentIntelligence>, IntelligenceStoreError> {
        (**self).load_intelligence(digest, key)
    }

    fn save_intelligence(
        &self,
        key: &IntelligenceCacheKey,
        intelligence: &DocumentIntelligence,
    ) -> Result<(), IntelligenceStoreError> {
        (**self).save_intelligence(key, intelligence)
    }
}

/// Refreshes local vault search indexes.
#[async_trait]
pub trait VaultIndexer: Send + Sync {
    /// Refreshes the configured index.
    async fn index(&self) -> Result<(), IndexError>;
}

/// Ingestion progress persistence failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("ingestion store failed: {0}")]
pub struct IngestionStoreError(pub String);

/// Classification persistence failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("classification store failed: {0}")]
pub struct ClassificationStoreError(pub String);

/// Document intelligence persistence failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("document intelligence store failed: {0}")]
pub struct IntelligenceStoreError(pub String);

/// Vault indexing failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("vault indexing failed: {0}")]
pub struct IndexError(pub String);

/// Vault persistence failures.
#[derive(Debug, Error)]
pub enum VaultError {
    /// The generated destination already exists.
    #[error("vault note already exists: {0}")]
    AlreadyExists(String),
    /// A filesystem operation failed.
    #[error("vault filesystem operation failed: {0}")]
    Filesystem(#[source] std::io::Error),
    /// Note metadata could not be rendered safely.
    #[error("vault note rendering failed: {0}")]
    Rendering(String),
    /// Source content changed between discovery and archival.
    #[error("source digest changed before archival")]
    DigestMismatch,
    /// A configured path escaped the vault or traversed a symlink.
    #[error("unsafe vault path: {0}")]
    UnsafePath(String),
}

/// Local research analysis failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AnalysisError {
    /// The configured local BAML call failed.
    #[error("local BAML analysis failed: {0}")]
    Model(String),
    /// Generated output failed domain validation.
    #[error("invalid generated analysis output: {0}")]
    Invalid(String),
}

/// Deterministic document extraction failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExtractionError {
    /// An external conversion tool failed.
    #[error("document tool failed: {0}")]
    Tool(String),
    /// All extraction routes produced empty text.
    #[error("document contained no usable text")]
    Empty,
    /// Extracted content violated domain rules.
    #[error("invalid extracted document: {0}")]
    Invalid(String),
    /// The immutable staged source changed during extraction.
    #[error("staged source changed during extraction")]
    SourceChanged,
}
