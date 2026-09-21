//! Tests resumable ingestion stages and guarded vault side effects.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use episteme::domain::{
    AnalysisProvenance, ArchivedSource, DocumentKind, EvidenceReference, ExtractedDocument,
    ExtractionMethod, IngestionRun, IngestionStage, ResearchDraft, ResearchNote, SourceDigest,
    SourceFileName, StagedSource, StoredNote,
};
use episteme::ingest::Ingestor;
use episteme::ports::{
    AnalysisError, DocumentExtractor, ExtractionError, IndexError, IngestionStore,
    IngestionStoreError, ResearchAnalyzer, VaultError, VaultIndexer, VaultStore,
};

#[derive(Debug, Default)]
struct FakeStore(Mutex<Option<IngestionRun>>);

impl IngestionStore for FakeStore {
    fn load(&self, _digest: &SourceDigest) -> Result<Option<IngestionRun>, IngestionStoreError> {
        self.0
            .lock()
            .map(|run| run.clone())
            .map_err(|_| IngestionStoreError("lock poisoned".to_owned()))
    }

    fn save(&self, run: &IngestionRun) -> Result<(), IngestionStoreError> {
        *self
            .0
            .lock()
            .map_err(|_| IngestionStoreError("lock poisoned".to_owned()))? = Some(run.clone());
        Ok(())
    }
}

#[derive(Debug)]
struct FakeExtractor(Arc<AtomicUsize>);

#[async_trait]
impl DocumentExtractor for FakeExtractor {
    async fn extract(&self, source: &StagedSource) -> Result<ExtractedDocument, ExtractionError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        ExtractedDocument::new(source.clone(), "source evidence", ExtractionMethod::PdfText)
            .map_err(|error| ExtractionError::Invalid(error.to_string()))
    }
}

#[derive(Debug)]
struct FakeAnalyzer(Arc<AtomicUsize>);

#[async_trait]
impl ResearchAnalyzer for FakeAnalyzer {
    async fn analyze(&self, _document: &ExtractedDocument) -> Result<ResearchDraft, AnalysisError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(valid_draft())
    }
}

#[derive(Debug, Default)]
struct FakeVault {
    note: Mutex<Option<StoredNote>>,
    create_count: Arc<AtomicUsize>,
    archive_attempts: Mutex<usize>,
}

impl VaultStore for FakeVault {
    fn find_by_digest(&self, _digest: &SourceDigest) -> Result<Option<StoredNote>, VaultError> {
        self.note
            .lock()
            .map(|note| note.clone())
            .map_err(|_| VaultError::Rendering("lock poisoned".to_owned()))
    }

    fn create_note(&self, _note: &ResearchNote) -> Result<StoredNote, VaultError> {
        self.create_count.fetch_add(1, Ordering::SeqCst);
        let stored = StoredNote {
            relative_path: "04_Research/note.md".into(),
        };
        *self
            .note
            .lock()
            .map_err(|_| VaultError::Rendering("lock poisoned".to_owned()))? = Some(stored.clone());
        Ok(stored)
    }

    fn archive_source(&self, _source: &StagedSource) -> Result<ArchivedSource, VaultError> {
        let mut attempts = self
            .archive_attempts
            .lock()
            .map_err(|_| VaultError::Rendering("lock poisoned".to_owned()))?;
        *attempts += 1;
        if *attempts == 1 {
            return Err(VaultError::Rendering(
                "simulated archive failure".to_owned(),
            ));
        }
        Ok(ArchivedSource {
            relative_path: "09_Archive/Sources/source.pdf".into(),
        })
    }
}

#[derive(Debug, Default)]
struct FakeIndexer;

#[async_trait]
impl VaultIndexer for FakeIndexer {
    async fn index(&self) -> Result<(), IndexError> {
        Ok(())
    }
}

#[tokio::test]
async fn ingestion_retry_resumes_without_duplicate_output() -> Result<(), Box<dyn std::error::Error>>
{
    let extract_count = Arc::new(AtomicUsize::new(0));
    let analyze_count = Arc::new(AtomicUsize::new(0));
    let create_count = Arc::new(AtomicUsize::new(0));
    let ingestor = Ingestor::new(
        FakeExtractor(extract_count.clone()),
        FakeAnalyzer(analyze_count.clone()),
        FakeVault {
            create_count: create_count.clone(),
            ..FakeVault::default()
        },
        FakeStore::default(),
        FakeIndexer,
    );
    let source = StagedSource::new(
        "paper.pdf".into(),
        SourceDigest::from_bytes(b"paper"),
        SourceFileName::new("paper.pdf")?,
        DocumentKind::Pdf,
    );
    let provenance = AnalysisProvenance {
        function: "DistillResearch".to_owned(),
        client: "LocalDistiller".to_owned(),
        model: "local".to_owned(),
        pipeline_version: "0.1.0".to_owned(),
        processed_at: "2026-09-18T12:00:00Z".to_owned(),
    };

    assert!(ingestor.ingest(&source, provenance.clone()).await.is_err());
    let completed = ingestor.ingest(&source, provenance).await?;

    assert_eq!(completed.stage, IngestionStage::Indexed);
    assert_eq!(extract_count.load(Ordering::SeqCst), 1);
    assert_eq!(analyze_count.load(Ordering::SeqCst), 1);
    assert_eq!(create_count.load(Ordering::SeqCst), 1);
    Ok(())
}

fn valid_draft() -> ResearchDraft {
    ResearchDraft {
        title: "Recovered research".to_owned(),
        citation: "fixture".to_owned(),
        topics: vec!["recovery".to_owned()],
        summary: "summary".to_owned(),
        key_ideas: vec!["resume durable work".to_owned()],
        implementation_notes: Vec::new(),
        critique: "fixture only".to_owned(),
        evidence: vec![EvidenceReference {
            quote: "source evidence".to_owned(),
            location: "paragraph 1".to_owned(),
        }],
    }
}
