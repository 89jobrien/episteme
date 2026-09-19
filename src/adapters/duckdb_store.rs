//! DuckDB-backed rebuildable ingestion state.

use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;

use duckdb::{Connection, params};
use thiserror::Error;

use crate::domain::{
    AnalysisProvenance, ClassificationRecord, DocumentClassification, ExtractionMethod,
    IngestionRun, IngestionStage, SourceDigest, SourceFileName,
};
use crate::ports::{
    ClassificationStore, ClassificationStoreError, IngestionStore, IngestionStoreError,
};

const INITIAL_SCHEMA: &str = include_str!("../../migrations/001_ingestion.sql");
const CLASSIFICATION_SCHEMA: &str = include_str!("../../migrations/002_classification.sql");

/// DuckDB-backed ingestion provenance storage.
#[derive(Debug)]
pub struct DuckDbIngestionStore {
    connection: Mutex<Connection>,
}

impl DuckDbIngestionStore {
    /// Opens a database and applies the initial idempotent schema.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the database cannot be opened or initialized.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let connection = Connection::open(path).map_err(StoreError::Database)?;
        connection
            .execute_batch(INITIAL_SCHEMA)
            .map_err(StoreError::Database)?;
        connection
            .execute_batch(CLASSIFICATION_SCHEMA)
            .map_err(StoreError::Database)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Inserts or replaces the derived record for a source digest.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when `DuckDB` rejects the operation.
    pub fn save(&self, run: &IngestionRun) -> Result<(), StoreError> {
        self.connection
            .lock()
            .map_err(|_| StoreError::LockPoisoned)?
            .execute(
                "INSERT INTO ingestion_runs (
                    source_digest, source_name, stage, note_path, archive_path, error
                 ) VALUES (?, ?, ?, ?, ?, ?)
                 ON CONFLICT (source_digest) DO UPDATE SET
                    source_name = excluded.source_name,
                    stage = excluded.stage,
                    note_path = excluded.note_path,
                    archive_path = excluded.archive_path,
                    error = excluded.error",
                params![
                    run.digest.as_str(),
                    run.source_name.as_str(),
                    run.stage.as_str(),
                    run.note_path.as_deref(),
                    run.archive_path.as_deref(),
                    run.error.as_deref(),
                ],
            )
            .map_err(StoreError::Database)?;
        Ok(())
    }

    /// Loads a record by content digest.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the query fails or persisted values violate domain rules.
    pub fn load(&self, digest: &SourceDigest) -> Result<Option<IngestionRun>, StoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StoreError::LockPoisoned)?;
        let mut statement = connection
            .prepare(
                "SELECT source_name, stage, note_path, archive_path, error
                 FROM ingestion_runs WHERE source_digest = ?",
            )
            .map_err(StoreError::Database)?;
        let mut rows = statement
            .query(params![digest.as_str()])
            .map_err(StoreError::Database)?;
        let Some(row) = rows.next().map_err(StoreError::Database)? else {
            return Ok(None);
        };

        let source_name: String = row.get(0).map_err(StoreError::Database)?;
        let stage: String = row.get(1).map_err(StoreError::Database)?;
        Ok(Some(IngestionRun {
            digest: digest.clone(),
            source_name: SourceFileName::new(source_name).map_err(StoreError::invalid_record)?,
            stage: IngestionStage::from_str(&stage).map_err(StoreError::invalid_record)?,
            note_path: row.get(2).map_err(StoreError::Database)?,
            archive_path: row.get(3).map_err(StoreError::Database)?,
            error: row.get(4).map_err(StoreError::Database)?,
        }))
    }

    fn save_classification(&self, record: &ClassificationRecord) -> Result<(), StoreError> {
        record
            .classification
            .clone()
            .validate()
            .map_err(StoreError::invalid_record)?;
        let authors = serde_json::to_string(&record.classification.authors)
            .map_err(StoreError::Serialization)?;
        let topics = serde_json::to_string(&record.classification.topics)
            .map_err(StoreError::Serialization)?;
        self.connection
            .lock()
            .map_err(|_| StoreError::LockPoisoned)?
            .execute(
                "INSERT INTO document_classifications (
                    source_digest, source_name, extraction_method, title, authors_json,
                    source_type, language, topics_json, analysis_function, analysis_client,
                    analysis_model, pipeline_version, processed_at
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (source_digest) DO UPDATE SET
                    source_name = excluded.source_name,
                    extraction_method = excluded.extraction_method,
                    title = excluded.title,
                    authors_json = excluded.authors_json,
                    source_type = excluded.source_type,
                    language = excluded.language,
                    topics_json = excluded.topics_json,
                    analysis_function = excluded.analysis_function,
                    analysis_client = excluded.analysis_client,
                    analysis_model = excluded.analysis_model,
                    pipeline_version = excluded.pipeline_version,
                    processed_at = excluded.processed_at",
                params![
                    record.source_digest.as_str(),
                    record.source_name.as_str(),
                    record.extraction_method.as_str(),
                    record.classification.title.as_str(),
                    authors,
                    record.classification.source_type.as_str(),
                    record.classification.language.as_str(),
                    topics,
                    record.analysis.function.as_str(),
                    record.analysis.client.as_str(),
                    record.analysis.model.as_str(),
                    record.analysis.pipeline_version.as_str(),
                    record.analysis.processed_at.as_str(),
                ],
            )
            .map_err(StoreError::Database)?;
        Ok(())
    }

    fn load_classification(
        &self,
        digest: &SourceDigest,
    ) -> Result<Option<ClassificationRecord>, StoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StoreError::LockPoisoned)?;
        let mut statement = connection
            .prepare(
                "SELECT source_name, extraction_method, title, authors_json, source_type,
                        language, topics_json, analysis_function, analysis_client, analysis_model,
                        pipeline_version, processed_at
                 FROM document_classifications WHERE source_digest = ?",
            )
            .map_err(StoreError::Database)?;
        let mut rows = statement
            .query(params![digest.as_str()])
            .map_err(StoreError::Database)?;
        let Some(row) = rows.next().map_err(StoreError::Database)? else {
            return Ok(None);
        };

        let source_name: String = row.get(0).map_err(StoreError::Database)?;
        let extraction_method: String = row.get(1).map_err(StoreError::Database)?;
        let authors: String = row.get(3).map_err(StoreError::Database)?;
        let topics: String = row.get(6).map_err(StoreError::Database)?;
        let classification = DocumentClassification {
            title: row.get(2).map_err(StoreError::Database)?,
            authors: serde_json::from_str(&authors).map_err(StoreError::Serialization)?,
            source_type: row.get(4).map_err(StoreError::Database)?,
            language: row.get(5).map_err(StoreError::Database)?,
            topics: serde_json::from_str(&topics).map_err(StoreError::Serialization)?,
        }
        .validate()
        .map_err(StoreError::invalid_record)?;
        Ok(Some(ClassificationRecord {
            source_digest: digest.clone(),
            source_name: SourceFileName::new(source_name).map_err(StoreError::invalid_record)?,
            extraction_method: ExtractionMethod::from_str(&extraction_method)
                .map_err(StoreError::invalid_record)?,
            classification,
            analysis: AnalysisProvenance {
                function: row.get(7).map_err(StoreError::Database)?,
                client: row.get(8).map_err(StoreError::Database)?,
                model: row.get(9).map_err(StoreError::Database)?,
                pipeline_version: row.get(10).map_err(StoreError::Database)?,
                processed_at: row.get(11).map_err(StoreError::Database)?,
            },
        }))
    }
}

/// Persistence failures for rebuildable ingestion state.
#[derive(Debug, Error)]
pub enum StoreError {
    /// `DuckDB` rejected an operation.
    #[error("DuckDB operation failed: {0}")]
    Database(#[source] duckdb::Error),
    /// Persisted state violated current domain rules.
    #[error("invalid persisted record: {0}")]
    InvalidRecord(String),
    /// JSON-encoded collection data could not be serialized or parsed.
    #[error("classification serialization failed: {0}")]
    Serialization(#[source] serde_json::Error),
    /// Another thread panicked while holding the database lock.
    #[error("DuckDB connection lock was poisoned")]
    LockPoisoned,
}

impl IngestionStore for DuckDbIngestionStore {
    fn load(&self, digest: &SourceDigest) -> Result<Option<IngestionRun>, IngestionStoreError> {
        Self::load(self, digest).map_err(|error| IngestionStoreError(error.to_string()))
    }

    fn save(&self, run: &IngestionRun) -> Result<(), IngestionStoreError> {
        Self::save(self, run).map_err(|error| IngestionStoreError(error.to_string()))
    }
}

impl ClassificationStore for DuckDbIngestionStore {
    fn load(
        &self,
        digest: &SourceDigest,
    ) -> Result<Option<ClassificationRecord>, ClassificationStoreError> {
        self.load_classification(digest)
            .map_err(|error| ClassificationStoreError(error.to_string()))
    }

    fn save(&self, record: &ClassificationRecord) -> Result<(), ClassificationStoreError> {
        self.save_classification(record)
            .map_err(|error| ClassificationStoreError(error.to_string()))
    }
}

impl StoreError {
    fn invalid_record(error: impl std::error::Error) -> Self {
        Self::InvalidRecord(error.to_string())
    }
}
