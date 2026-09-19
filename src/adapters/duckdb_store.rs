//! DuckDB-backed rebuildable ingestion state.

use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;

use duckdb::{Connection, params};
use thiserror::Error;

use crate::domain::{IngestionRun, IngestionStage, SourceDigest, SourceFileName};
use crate::ports::{IngestionStore, IngestionStoreError};

const INITIAL_SCHEMA: &str = include_str!("../../migrations/001_ingestion.sql");

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
}

/// Persistence failures for rebuildable ingestion state.
#[derive(Debug, Error)]
pub enum StoreError {
    /// `DuckDB` rejected an operation.
    #[error("DuckDB operation failed: {0}")]
    Database(#[source] duckdb::Error),
    /// Persisted state violated current domain rules.
    #[error("invalid persisted ingestion record: {0}")]
    InvalidRecord(String),
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

impl StoreError {
    fn invalid_record(error: impl std::error::Error) -> Self {
        Self::InvalidRecord(error.to_string())
    }
}
