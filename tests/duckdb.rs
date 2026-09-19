use episteme::adapters::DuckDbIngestionStore;
use episteme::domain::{IngestionRun, IngestionStage, SourceDigest, SourceFileName};

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
