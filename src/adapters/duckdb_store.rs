//! DuckDB-backed rebuildable ingestion state.

use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;

use chrono::Utc;
use duckdb::{Connection, params};
use thiserror::Error;

use crate::domain::{
    AnalysisProvenance, ClassificationAttemptRecord, ClassificationAttemptStatus,
    ClassificationBatchSummary, ClassificationCacheKey, ClassificationFailureCode,
    ClassificationRecord, DocumentClassification, DocumentIntelligence, DocumentSourceType,
    ExtractionMethod, IngestionRun, IngestionStage, IntelligenceCacheKey, SourceDigest,
    SourceFileName,
};
use crate::ports::{
    ClassificationRunStore, ClassificationStore, ClassificationStoreError,
    DocumentIntelligenceStore, IngestionStore, IngestionStoreError, IntelligenceStoreError,
};

const INITIAL_SCHEMA: &str = include_str!("../../migrations/001_ingestion.sql");
const CLASSIFICATION_SCHEMA: &str = include_str!("../../migrations/002_classification.sql");
const CLASSIFICATION_BATCH_SCHEMA: &str =
    include_str!("../../migrations/003_classification_batches.sql");
const INTELLIGENCE_SCHEMA: &str = include_str!("../../migrations/004_document_intelligence.sql");

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
        connection
            .execute_batch(CLASSIFICATION_BATCH_SCHEMA)
            .map_err(StoreError::Database)?;
        connection
            .execute_batch(INTELLIGENCE_SCHEMA)
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

    fn save_classification(
        &self,
        key: &ClassificationCacheKey,
        record: &ClassificationRecord,
    ) -> Result<(), StoreError> {
        if key.model != record.analysis.model
            || key.policy_version != record.analysis.pipeline_version
        {
            return Err(StoreError::InvalidRecord(
                "classification cache key did not match record provenance".to_owned(),
            ));
        }
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
                "INSERT INTO document_classification_versions (
                    source_digest, analysis_model, policy_version, source_name,
                    extraction_method, title, authors_json, source_type, language_code,
                    topics_json, analysis_function, analysis_client, processed_at
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (source_digest, analysis_model, policy_version) DO UPDATE SET
                    source_name = excluded.source_name,
                    extraction_method = excluded.extraction_method,
                    title = excluded.title,
                    authors_json = excluded.authors_json,
                    source_type = excluded.source_type,
                    language_code = excluded.language_code,
                    topics_json = excluded.topics_json,
                    analysis_function = excluded.analysis_function,
                    analysis_client = excluded.analysis_client,
                    processed_at = excluded.processed_at",
                params![
                    record.source_digest.as_str(),
                    key.model.as_str(),
                    key.policy_version.as_str(),
                    record.source_name.as_str(),
                    record.extraction_method.as_str(),
                    record.classification.title.as_str(),
                    authors,
                    record.classification.source_type.as_str(),
                    record.classification.language_code.as_str(),
                    topics,
                    record.analysis.function.as_str(),
                    record.analysis.client.as_str(),
                    record.analysis.processed_at.as_str(),
                ],
            )
            .map_err(StoreError::Database)?;
        Ok(())
    }

    fn load_classification(
        &self,
        digest: &SourceDigest,
        key: &ClassificationCacheKey,
    ) -> Result<Option<ClassificationRecord>, StoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StoreError::LockPoisoned)?;
        let mut statement = connection
            .prepare(
                "SELECT source_name, extraction_method, title, authors_json, source_type,
                        language_code, topics_json, analysis_function, analysis_client, processed_at
                 FROM document_classification_versions
                 WHERE source_digest = ? AND analysis_model = ? AND policy_version = ?",
            )
            .map_err(StoreError::Database)?;
        let mut rows = statement
            .query(params![
                digest.as_str(),
                key.model.as_str(),
                key.policy_version.as_str()
            ])
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
            source_type: DocumentSourceType::from_str(
                &row.get::<_, String>(4).map_err(StoreError::Database)?,
            )
            .map_err(StoreError::invalid_record)?,
            language_code: row.get(5).map_err(StoreError::Database)?,
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
                model: key.model.clone(),
                pipeline_version: key.policy_version.clone(),
                processed_at: row.get(9).map_err(StoreError::Database)?,
            },
        }))
    }

    fn load_latest_attempts(
        &self,
        batch_id: &str,
    ) -> Result<Vec<ClassificationAttemptRecord>, StoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StoreError::LockPoisoned)?;
        let mut statement = connection
            .prepare(
                "SELECT source_path, source_digest, attempt, status, model,
                        extracted_characters, duration_ms, failure_code, failure_message,
                        updated_at
                 FROM (
                    SELECT *, row_number() OVER (
                        PARTITION BY source_path ORDER BY attempt DESC
                    ) AS rank
                    FROM classification_attempts WHERE batch_id = ?
                 ) latest
                 WHERE rank = 1 ORDER BY source_path",
            )
            .map_err(StoreError::Database)?;
        let mut rows = statement
            .query(params![batch_id])
            .map_err(StoreError::Database)?;
        let mut attempts = Vec::new();
        while let Some(row) = rows.next().map_err(StoreError::Database)? {
            let digest: Option<String> = row.get(1).map_err(StoreError::Database)?;
            let status: String = row.get(3).map_err(StoreError::Database)?;
            let failure_code: Option<String> = row.get(7).map_err(StoreError::Database)?;
            attempts.push(ClassificationAttemptRecord {
                batch_id: batch_id.to_owned(),
                source_path: row.get(0).map_err(StoreError::Database)?,
                source_digest: digest
                    .map(SourceDigest::parse)
                    .transpose()
                    .map_err(StoreError::invalid_record)?,
                attempt: row.get(2).map_err(StoreError::Database)?,
                status: ClassificationAttemptStatus::from_str(&status)
                    .map_err(StoreError::invalid_record)?,
                model: row.get(4).map_err(StoreError::Database)?,
                extracted_characters: row.get(5).map_err(StoreError::Database)?,
                duration_ms: row.get(6).map_err(StoreError::Database)?,
                failure_code: failure_code
                    .map(|code| ClassificationFailureCode::from_str(&code))
                    .transpose()
                    .map_err(StoreError::invalid_record)?,
                failure_message: row.get(8).map_err(StoreError::Database)?,
                updated_at: row.get(9).map_err(StoreError::Database)?,
            });
        }
        Ok(attempts)
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
        key: &ClassificationCacheKey,
    ) -> Result<Option<ClassificationRecord>, ClassificationStoreError> {
        self.load_classification(digest, key)
            .map_err(|error| ClassificationStoreError(error.to_string()))
    }

    fn save(
        &self,
        key: &ClassificationCacheKey,
        record: &ClassificationRecord,
    ) -> Result<(), ClassificationStoreError> {
        self.save_classification(key, record)
            .map_err(|error| ClassificationStoreError(error.to_string()))
    }
}

impl ClassificationRunStore for DuckDbIngestionStore {
    fn begin_batch(
        &self,
        batch_id: &str,
        started_at: &str,
    ) -> Result<(), ClassificationStoreError> {
        self.connection
            .lock()
            .map_err(|_| StoreError::LockPoisoned)
            .and_then(|connection| {
                connection
                    .execute(
                        "INSERT INTO classification_batches (batch_id, started_at)
                         VALUES (?, ?) ON CONFLICT (batch_id) DO UPDATE SET
                            started_at = excluded.started_at,
                            completed_at = NULL,
                            total = NULL,
                            succeeded = NULL,
                            cached = NULL,
                            failed_retryable = NULL,
                            failed_terminal = NULL",
                        params![batch_id, started_at],
                    )
                    .map(|_| ())
                    .map_err(StoreError::Database)
            })
            .map_err(|error| ClassificationStoreError(error.to_string()))
    }

    fn save_attempt(
        &self,
        attempt: &ClassificationAttemptRecord,
    ) -> Result<(), ClassificationStoreError> {
        self.connection
            .lock()
            .map_err(|_| StoreError::LockPoisoned)
            .and_then(|connection| {
                connection
                    .execute(
                        "INSERT INTO classification_attempts (
                            batch_id, source_path, attempt, source_digest, status, model,
                            extracted_characters, duration_ms, failure_code, failure_message,
                            updated_at
                         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                         ON CONFLICT (batch_id, source_path, attempt) DO UPDATE SET
                            source_digest = excluded.source_digest,
                            status = excluded.status,
                            model = excluded.model,
                            extracted_characters = excluded.extracted_characters,
                            duration_ms = excluded.duration_ms,
                            failure_code = excluded.failure_code,
                            failure_message = excluded.failure_message,
                            updated_at = excluded.updated_at",
                        params![
                            attempt.batch_id.as_str(),
                            attempt.source_path.as_str(),
                            attempt.attempt,
                            attempt.source_digest.as_ref().map(SourceDigest::as_str),
                            attempt.status.as_str(),
                            attempt.model.as_deref(),
                            attempt.extracted_characters,
                            attempt.duration_ms,
                            attempt.failure_code.map(ClassificationFailureCode::as_str),
                            attempt.failure_message.as_deref(),
                            attempt.updated_at.as_str(),
                        ],
                    )
                    .map(|_| ())
                    .map_err(StoreError::Database)
            })
            .map_err(|error| ClassificationStoreError(error.to_string()))
    }

    fn latest_attempts(
        &self,
        batch_id: &str,
    ) -> Result<Vec<ClassificationAttemptRecord>, ClassificationStoreError> {
        self.load_latest_attempts(batch_id)
            .map_err(|error| ClassificationStoreError(error.to_string()))
    }

    fn finish_batch(
        &self,
        summary: &ClassificationBatchSummary,
    ) -> Result<(), ClassificationStoreError> {
        let counts = [
            summary.total,
            summary.succeeded,
            summary.cached,
            summary.failed_retryable,
            summary.failed_terminal,
        ]
        .map(|value| {
            u64::try_from(value)
                .map_err(|_| StoreError::InvalidRecord("batch count overflow".to_owned()))
        });
        let [total, succeeded, cached, failed_retryable, failed_terminal] = counts;
        let (total, succeeded, cached, failed_retryable, failed_terminal) = (
            total.map_err(|error| ClassificationStoreError(error.to_string()))?,
            succeeded.map_err(|error| ClassificationStoreError(error.to_string()))?,
            cached.map_err(|error| ClassificationStoreError(error.to_string()))?,
            failed_retryable.map_err(|error| ClassificationStoreError(error.to_string()))?,
            failed_terminal.map_err(|error| ClassificationStoreError(error.to_string()))?,
        );
        self.connection
            .lock()
            .map_err(|_| StoreError::LockPoisoned)
            .and_then(|connection| {
                connection
                    .execute(
                        "UPDATE classification_batches SET completed_at = ?, total = ?,
                         succeeded = ?, cached = ?, failed_retryable = ?, failed_terminal = ?
                         WHERE batch_id = ?",
                        params![
                            Utc::now().to_rfc3339(),
                            total,
                            succeeded,
                            cached,
                            failed_retryable,
                            failed_terminal,
                            summary.batch_id.as_str(),
                        ],
                    )
                    .map(|_| ())
                    .map_err(StoreError::Database)
            })
            .map_err(|error| ClassificationStoreError(error.to_string()))
    }
}

impl DocumentIntelligenceStore for DuckDbIngestionStore {
    fn load_intelligence(
        &self,
        digest: &SourceDigest,
        key: &IntelligenceCacheKey,
    ) -> Result<Option<DocumentIntelligence>, IntelligenceStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| IntelligenceStoreError(StoreError::LockPoisoned.to_string()))?;
        let mut statement = connection
            .prepare(
                "SELECT intelligence_json FROM document_intelligence_versions
                 WHERE source_digest = ? AND analysis_model = ? AND policy_version = ?",
            )
            .map_err(|error| IntelligenceStoreError(error.to_string()))?;
        let mut rows = statement
            .query(params![
                digest.as_str(),
                key.model.as_str(),
                key.policy_version.as_str()
            ])
            .map_err(|error| IntelligenceStoreError(error.to_string()))?;
        let Some(row) = rows
            .next()
            .map_err(|error| IntelligenceStoreError(error.to_string()))?
        else {
            return Ok(None);
        };
        let json: String = row
            .get(0)
            .map_err(|error| IntelligenceStoreError(error.to_string()))?;
        serde_json::from_str(&json)
            .map(Some)
            .map_err(|error| IntelligenceStoreError(error.to_string()))
    }

    fn save_intelligence(
        &self,
        key: &IntelligenceCacheKey,
        intelligence: &DocumentIntelligence,
    ) -> Result<(), IntelligenceStoreError> {
        if key.model != intelligence.analysis.model
            || key.policy_version != intelligence.analysis.pipeline_version
        {
            return Err(IntelligenceStoreError(
                "intelligence cache key did not match provenance".to_owned(),
            ));
        }
        let json = serde_json::to_string(intelligence)
            .map_err(|error| IntelligenceStoreError(error.to_string()))?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| IntelligenceStoreError(StoreError::LockPoisoned.to_string()))?;
        let transaction = connection
            .transaction()
            .map_err(|error| IntelligenceStoreError(error.to_string()))?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO document_intelligence_versions VALUES (?, ?, ?, ?, ?)",
                params![
                    intelligence.source_digest.as_str(),
                    key.model.as_str(),
                    key.policy_version.as_str(),
                    json,
                    intelligence.analysis.processed_at.as_str()
                ],
            )
            .map_err(|error| IntelligenceStoreError(error.to_string()))?;
        for claim in &intelligence.claims {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO intelligence_claims VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        intelligence.source_digest.as_str(),
                        key.model.as_str(),
                        key.policy_version.as_str(),
                        claim.id.as_str(),
                        claim.text.as_str(),
                        claim.kind.as_str(),
                        claim.confidence_percent,
                        serde_json::to_string(&claim.evidence)
                            .map_err(|error| IntelligenceStoreError(error.to_string()))?
                    ],
                )
                .map_err(|error| IntelligenceStoreError(error.to_string()))?;
        }
        for entity in &intelligence.entities {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO intelligence_entities VALUES (?, ?, ?, ?, ?)",
                    params![
                        entity.id.as_str(),
                        entity.kind.as_str(),
                        entity.name.as_str(),
                        serde_json::to_string(&entity.aliases)
                            .map_err(|error| IntelligenceStoreError(error.to_string()))?,
                        entity.description.as_str()
                    ],
                )
                .map_err(|error| IntelligenceStoreError(error.to_string()))?;
            transaction
                .execute(
                    "INSERT OR REPLACE INTO document_intelligence_entities VALUES (?, ?, ?, ?, ?)",
                    params![
                        intelligence.source_digest.as_str(),
                        key.model.as_str(),
                        key.policy_version.as_str(),
                        entity.id.as_str(),
                        serde_json::to_string(&entity.evidence)
                            .map_err(|error| IntelligenceStoreError(error.to_string()))?
                    ],
                )
                .map_err(|error| IntelligenceStoreError(error.to_string()))?;
        }
        for relation in &intelligence.relations {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO semantic_relations VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        intelligence.source_digest.as_str(),
                        key.model.as_str(),
                        key.policy_version.as_str(),
                        relation.id.as_str(),
                        relation.source_id.as_str(),
                        relation.target_id.as_str(),
                        relation.relation_type.as_str(),
                        relation.confidence_percent,
                        serde_json::to_string(&relation.evidence)
                            .map_err(|error| IntelligenceStoreError(error.to_string()))?
                    ],
                )
                .map_err(|error| IntelligenceStoreError(error.to_string()))?;
        }
        transaction
            .commit()
            .map_err(|error| IntelligenceStoreError(error.to_string()))
    }
}

impl StoreError {
    fn invalid_record(error: impl std::error::Error) -> Self {
        Self::InvalidRecord(error.to_string())
    }
}
