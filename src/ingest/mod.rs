//! Recoverable document ingestion orchestration.

use thiserror::Error;

use crate::domain::{
    AnalysisProvenance, IngestionRun, IngestionStage, ResearchNote, StagedSource, StoredNote,
};
use crate::ports::{DocumentExtractor, IngestionStore, ResearchAnalyzer, VaultIndexer, VaultStore};

/// Coordinates one source through deterministic extraction, local analysis, and persistence.
#[derive(Debug)]
pub struct Ingestor<E, A, V, S, I> {
    extractor: E,
    analyzer: A,
    vault: V,
    store: S,
    indexer: I,
}

impl<E, A, V, S, I> Ingestor<E, A, V, S, I> {
    /// Creates an ingestion service from its external ports.
    #[must_use]
    pub const fn new(extractor: E, analyzer: A, vault: V, store: S, indexer: I) -> Self {
        Self {
            extractor,
            analyzer,
            vault,
            store,
            indexer,
        }
    }
}

impl<E, A, V, S, I> Ingestor<E, A, V, S, I>
where
    E: DocumentExtractor,
    A: ResearchAnalyzer,
    V: VaultStore,
    S: IngestionStore,
    I: VaultIndexer,
{
    /// Ingests or safely resumes one staged source.
    ///
    /// # Errors
    ///
    /// Returns [`IngestError`] when any required stage fails. Durable completed stages remain
    /// recorded so a later call can resume.
    pub async fn ingest(
        &self,
        source: &StagedSource,
        provenance: AnalysisProvenance,
    ) -> Result<IngestionRun, IngestError> {
        if let Some(run) = self
            .store
            .load(source.digest())
            .map_err(IngestError::store)?
            && run.stage == IngestionStage::Indexed
        {
            return Ok(run);
        }

        let mut run = IngestionRun {
            digest: source.digest().clone(),
            source_name: source.source_name().clone(),
            stage: IngestionStage::Discovered,
            note_path: None,
            archive_path: None,
            error: None,
        };

        let stored_note = if let Some(existing) = self
            .vault
            .find_by_digest(source.digest())
            .map_err(IngestError::vault)?
        {
            run.stage = IngestionStage::NoteWritten;
            run.note_path = Some(existing.relative_path.display().to_string());
            self.store.save(&run).map_err(IngestError::store)?;
            existing
        } else {
            let extracted = self
                .extractor
                .extract(source)
                .await
                .map_err(IngestError::extract)?;
            run.stage = IngestionStage::Extracted;
            self.store.save(&run).map_err(IngestError::store)?;

            let draft = self
                .analyzer
                .analyze(&extracted)
                .await
                .map_err(IngestError::analyze)?;
            run.stage = IngestionStage::Analyzed;
            self.store.save(&run).map_err(IngestError::store)?;

            let note = ResearchNote {
                draft,
                source_digest: source.digest().clone(),
                source_name: source.source_name().clone(),
                extraction_method: extracted.method(),
                analysis: provenance,
            };
            let stored = self.vault.create_note(&note).map_err(IngestError::vault)?;
            run.stage = IngestionStage::NoteWritten;
            run.note_path = Some(stored.relative_path.display().to_string());
            self.store.save(&run).map_err(IngestError::store)?;
            stored
        };

        self.finish(source, stored_note, run).await
    }

    async fn finish(
        &self,
        source: &StagedSource,
        _stored_note: StoredNote,
        mut run: IngestionRun,
    ) -> Result<IngestionRun, IngestError> {
        self.indexer.index().await.map_err(IngestError::index)?;
        let archived = self
            .vault
            .archive_source(source)
            .map_err(IngestError::vault)?;
        run.archive_path = Some(archived.relative_path.display().to_string());
        run.stage = IngestionStage::Indexed;
        self.store.save(&run).map_err(IngestError::store)?;
        Ok(run)
    }
}

/// Ingestion orchestration failures with no private document content.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IngestError {
    /// Deterministic extraction failed.
    #[error("extraction failed: {0}")]
    Extraction(String),
    /// Local analysis failed.
    #[error("analysis failed: {0}")]
    Analysis(String),
    /// Vault persistence failed.
    #[error("vault persistence failed: {0}")]
    Vault(String),
    /// Progress persistence failed.
    #[error("progress persistence failed: {0}")]
    Store(String),
    /// Vault indexing failed.
    #[error("vault indexing failed: {0}")]
    Index(String),
}

impl IngestError {
    fn extract(error: impl std::error::Error) -> Self {
        Self::Extraction(error.to_string())
    }

    fn analyze(error: impl std::error::Error) -> Self {
        Self::Analysis(error.to_string())
    }

    fn vault(error: impl std::error::Error) -> Self {
        Self::Vault(error.to_string())
    }

    fn store(error: impl std::error::Error) -> Self {
        Self::Store(error.to_string())
    }

    fn index(error: impl std::error::Error) -> Self {
        Self::Index(error.to_string())
    }
}
