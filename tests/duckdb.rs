use episteme::adapters::DuckDbIngestionStore;
use episteme::domain::{
    AnalysisProvenance, ClassificationRecord, DocumentClassification, ExtractionMethod,
    IngestionRun, IngestionStage, SourceDigest, SourceFileName,
};
use episteme::ports::ClassificationStore;

#[test]
fn duckdb_round_trips_ingestion_provenance() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let store = DuckDbIngestionStore::open(&directory.path().join("episteme.duckdb"))?;
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

    ClassificationStore::save(&store, &record)?;
    assert_eq!(
        ClassificationStore::load(&store, &record.source_digest)?,
        Some(record.clone())
    );

    record.classification.title = "Updated Agent Systems".to_owned();
    ClassificationStore::save(&store, &record)?;
    assert_eq!(
        ClassificationStore::load(&store, &record.source_digest)?,
        Some(record.clone())
    );

    let mut invalid = record.clone();
    invalid.classification.title = String::new();
    assert!(ClassificationStore::save(&store, &invalid).is_err());
    assert_eq!(
        ClassificationStore::load(&store, &record.source_digest)?,
        Some(record)
    );
    Ok(())
}

fn classification_record() -> Result<ClassificationRecord, Box<dyn std::error::Error>> {
    Ok(ClassificationRecord {
        source_digest: SourceDigest::from_bytes(b"classified source"),
        source_name: SourceFileName::new("paper.pdf")?,
        extraction_method: ExtractionMethod::PdfText,
        classification: DocumentClassification {
            title: "Agent Systems".to_owned(),
            authors: vec!["Researcher".to_owned()],
            source_type: "paper".to_owned(),
            language: "en".to_owned(),
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
