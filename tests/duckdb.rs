//! Tests `DuckDB` persistence, versioned classifications, and batch reconciliation.

use episteme::adapters::DuckDbIngestionStore;
use episteme::domain::{
    AnalysisProvenance, ClassificationAttemptRecord, ClassificationAttemptStatus,
    ClassificationBatchSummary, ClassificationCacheKey, ClassificationFailureCode,
    ClassificationRecord, DocumentClassification, DocumentSourceType, ExtractionMethod,
    IngestionRun, IngestionStage, SourceDigest, SourceFileName,
};
use episteme::ports::{ClassificationRunStore, ClassificationStore};

#[test]
fn duckdb_round_trips_ingestion_provenance() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let database = directory.path().join("episteme.duckdb");
    let store = DuckDbIngestionStore::open(&database)?;
    let run = IngestionRun {
        digest: SourceDigest::from_bytes(b"source"),
        source_name: SourceFileName::new("paper.pdf")?,
        stage: IngestionStage::NoteWritten,
        note_path: Some("04_Research/Paper.md".to_owned()),
        archive_path: None,
        error: None,
    };

    store.save(&run)?;

    assert_eq!(store.load(&run.digest)?, Some(run));
    Ok(())
}

#[test]
fn duckdb_round_trips_and_upserts_classifications() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let store = DuckDbIngestionStore::open(&directory.path().join("episteme.duckdb"))?;
    let mut record = classification_record()?;
    let key = ClassificationCacheKey {
        model: record.analysis.model.clone(),
        policy_version: record.analysis.pipeline_version.clone(),
    };

    ClassificationStore::save(&store, &key, &record)?;
    assert_eq!(
        ClassificationStore::load(&store, &record.source_digest, &key)?,
        Some(record.clone())
    );

    record.classification.title = "Updated Agent Systems".to_owned();
    ClassificationStore::save(&store, &key, &record)?;
    assert_eq!(
        ClassificationStore::load(&store, &record.source_digest, &key)?,
        Some(record.clone())
    );

    let mut invalid = record.clone();
    invalid.classification.title = String::new();
    assert!(ClassificationStore::save(&store, &key, &invalid).is_err());
    assert_eq!(
        ClassificationStore::load(&store, &record.source_digest, &key)?,
        Some(record)
    );
    Ok(())
}

#[test]
fn duckdb_versions_classifications_and_reconciles_latest_attempts()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let database = directory.path().join("episteme.duckdb");
    let store = DuckDbIngestionStore::open(&database)?;
    let mut first = classification_record()?;
    first.analysis.model = "small-model".to_owned();
    first.analysis.pipeline_version = "classification-v2".to_owned();
    let first_key = ClassificationCacheKey {
        model: "small-model".to_owned(),
        policy_version: "classification-v2".to_owned(),
    };
    let mut second = first.clone();
    second.analysis.model = "large-model".to_owned();
    second.classification.title = "Improved Agent Systems".to_owned();
    let second_key = ClassificationCacheKey {
        model: "large-model".to_owned(),
        policy_version: "classification-v2".to_owned(),
    };

    ClassificationStore::save(&store, &first_key, &first)?;
    ClassificationStore::save(&store, &second_key, &second)?;
    let mismatched_key = ClassificationCacheKey {
        model: "wrong-model".to_owned(),
        policy_version: "classification-v2".to_owned(),
    };
    assert!(ClassificationStore::save(&store, &mismatched_key, &first).is_err());

    assert_eq!(
        ClassificationStore::load(&store, &first.source_digest, &first_key)?,
        Some(first.clone())
    );
    assert_eq!(
        ClassificationStore::load(&store, &second.source_digest, &second_key)?,
        Some(second)
    );

    ClassificationRunStore::begin_batch(&store, "batch-1", "2026-09-19T00:00:00Z")?;
    let failed = attempt(
        1,
        ClassificationAttemptStatus::FailedRetryable,
        Some(ClassificationFailureCode::ContextOverflow),
    );
    let succeeded = attempt(2, ClassificationAttemptStatus::Succeeded, None);
    ClassificationRunStore::save_attempt(&store, &failed)?;
    ClassificationRunStore::save_attempt(&store, &succeeded)?;
    assert_eq!(
        ClassificationRunStore::latest_attempts(&store, "batch-1")?,
        vec![succeeded.clone()]
    );
    ClassificationRunStore::finish_batch(
        &store,
        &ClassificationBatchSummary {
            batch_id: "batch-1".to_owned(),
            total: 1,
            succeeded: 1,
            cached: 0,
            failed_retryable: 0,
            failed_terminal: 0,
            attempts: vec![succeeded],
        },
    )?;
    ClassificationRunStore::begin_batch(&store, "batch-1", "2026-09-19T00:01:00Z")?;
    let connection = duckdb::Connection::open(&database)?;
    let completed_at: Option<String> = connection.query_row(
        "SELECT completed_at FROM classification_batches WHERE batch_id = ?",
        duckdb::params!["batch-1"],
        |row| row.get(0),
    )?;
    assert_eq!(completed_at, None);
    Ok(())
}

fn attempt(
    attempt: u32,
    status: ClassificationAttemptStatus,
    failure_code: Option<ClassificationFailureCode>,
) -> ClassificationAttemptRecord {
    ClassificationAttemptRecord {
        batch_id: "batch-1".to_owned(),
        source_path: "report.pdf".to_owned(),
        source_digest: Some(SourceDigest::from_bytes(b"classified source")),
        attempt,
        status,
        model: Some("large-model".to_owned()),
        extracted_characters: Some(1_000),
        duration_ms: 25,
        failure_code,
        failure_message: failure_code.map(|_| "classifier context window was exceeded".to_owned()),
        updated_at: "2026-09-19T00:00:01Z".to_owned(),
    }
}

fn classification_record() -> Result<ClassificationRecord, Box<dyn std::error::Error>> {
    Ok(ClassificationRecord {
        source_digest: SourceDigest::from_bytes(b"classified source"),
        source_name: SourceFileName::new("paper.pdf")?,
        extraction_method: ExtractionMethod::PdfText,
        classification: DocumentClassification {
            title: "Agent Systems".to_owned(),
            authors: vec!["Researcher".to_owned()],
            source_type: DocumentSourceType::ResearchPaper,
            language_code: "en".to_owned(),
            topics: vec!["agents".to_owned(), "systems".to_owned()],
        },
        analysis: AnalysisProvenance {
            function: "ClassifyDocument".to_owned(),
            client: "LocalClassifier".to_owned(),
            model: "local-model".to_owned(),
            pipeline_version: "0.1.0".to_owned(),
            processed_at: "2026-09-19T00:00:00Z".to_owned(),
        },
    })
}
