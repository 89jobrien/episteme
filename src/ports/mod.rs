//! Hexagonal ports for external document and storage integrations.

use std::path::Path;

use async_trait::async_trait;
use thiserror::Error;

use crate::domain::{
    ArchivedSource, ClassificationRecord, DocumentClassification, ExtractedDocument, IngestionRun,
    ResearchDraft, ResearchNote, SourceDigest, StagedSource, StoredNote,
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
    /// Produces typed metadata for an extracted document.
    async fn classify(
        &self,
        document: &ExtractedDocument,
    ) -> Result<DocumentClassification, AnalysisError>;
}

/// Produces a typed, source-grounded research draft through local inference.
#[async_trait]
pub trait ResearchAnalyzer: Send + Sync {
    /// Analyzes extracted source text.
    async fn analyze(&self, document: &ExtractedDocument) -> Result<ResearchDraft, AnalysisError>;
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
    ) -> Result<Option<ClassificationRecord>, ClassificationStoreError>;

    /// Inserts or replaces one classification record.
    ///
    /// # Errors
    ///
    /// Returns [`ClassificationStoreError`] when persistence fails.
    fn save(&self, record: &ClassificationRecord) -> Result<(), ClassificationStoreError>;
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
