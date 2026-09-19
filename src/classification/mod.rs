//! Idempotent document classification orchestration.

use thiserror::Error;

use crate::domain::{AnalysisProvenance, ClassificationRecord, StagedSource};
use crate::ports::{ClassificationStore, DocumentClassifier, DocumentExtractor};

/// Coordinates extraction, local classification, and derived-state persistence.
#[derive(Debug)]
pub struct ClassificationPipeline<E, C, S> {
    extractor: E,
    classifier: C,
    store: S,
}

impl<E, C, S> ClassificationPipeline<E, C, S> {
    /// Creates a classification pipeline from its external ports.
    #[must_use]
    pub const fn new(extractor: E, classifier: C, store: S) -> Self {
        Self {
            extractor,
            classifier,
            store,
        }
    }
}

impl<E, C, S> ClassificationPipeline<E, C, S>
where
    E: DocumentExtractor,
    C: DocumentClassifier,
    S: ClassificationStore,
{
    /// Classifies a staged source or returns its existing digest-matched record.
    ///
    /// # Errors
    ///
    /// Returns [`ClassificationError`] when extraction, inference, validation, or persistence
    /// fails.
    pub async fn classify(
        &self,
        source: &StagedSource,
        provenance: AnalysisProvenance,
    ) -> Result<ClassificationRecord, ClassificationError> {
        if let Some(record) = self
            .store
            .load(source.digest())
            .map_err(ClassificationError::store)?
        {
            return Ok(record);
        }

        let extracted = self
            .extractor
            .extract(source)
            .await
            .map_err(ClassificationError::extract)?;
        let classification = self
            .classifier
            .classify(&extracted)
            .await
            .map_err(ClassificationError::analyze)?
            .validate()
            .map_err(ClassificationError::analyze)?;
        let record = ClassificationRecord {
            source_digest: source.digest().clone(),
            source_name: source.source_name().clone(),
            extraction_method: extracted.method(),
            classification,
            analysis: provenance,
        };
        self.store
            .save(&record)
            .map_err(ClassificationError::store)?;
        Ok(record)
    }
}

/// Standalone classification failures without private document content.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ClassificationError {
    /// Deterministic extraction failed.
    #[error("extraction failed: {0}")]
    Extraction(String),
    /// Local inference or generated metadata validation failed.
    #[error("classification failed: {0}")]
    Analysis(String),
    /// Derived-state persistence failed.
    #[error("classification persistence failed: {0}")]
    Store(String),
}

impl ClassificationError {
    fn extract(error: impl std::error::Error) -> Self {
        Self::Extraction(error.to_string())
    }

    fn analyze(error: impl std::error::Error) -> Self {
        Self::Analysis(error.to_string())
    }

    fn store(error: impl std::error::Error) -> Self {
        Self::Store(error.to_string())
    }
}
