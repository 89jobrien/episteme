use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use episteme::classification::ClassificationPipeline;
use episteme::domain::{
    AnalysisProvenance, ClassificationRecord, DocumentClassification, DocumentKind, DomainError,
    ExtractedDocument, ExtractionMethod, SourceDigest, SourceFileName, StagedSource,
};
use episteme::ports::{
    AnalysisError, ClassificationStore, ClassificationStoreError, DocumentClassifier,
    DocumentExtractor, ExtractionError,
};

#[test]
fn classification_rejects_missing_required_fields() {
    let invalid = [
        DocumentClassification {
            title: " ".to_owned(),
            authors: vec!["Researcher".to_owned()],
            source_type: "paper".to_owned(),
            language: "en".to_owned(),
            topics: vec!["agents".to_owned()],
        },
        DocumentClassification {
            title: "Agent Systems".to_owned(),
            authors: vec!["Researcher".to_owned()],
            source_type: String::new(),
            language: "en".to_owned(),
            topics: vec!["agents".to_owned()],
        },
        DocumentClassification {
            title: "Agent Systems".to_owned(),
            authors: vec!["Researcher".to_owned()],
            source_type: "paper".to_owned(),
            language: String::new(),
            topics: vec!["agents".to_owned()],
        },
        DocumentClassification {
            title: "Agent Systems".to_owned(),
            authors: vec!["Researcher".to_owned()],
            source_type: "paper".to_owned(),
            language: "en".to_owned(),
            topics: Vec::new(),
        },
        DocumentClassification {
            title: "Agent\nSystems".to_owned(),
            authors: vec!["Researcher".to_owned()],
            source_type: "paper".to_owned(),
            language: "en".to_owned(),
            topics: vec!["agents".to_owned()],
        },
        DocumentClassification {
            title: "Agent Systems".to_owned(),
            authors: vec!["Researcher".to_owned()],
            source_type: "paper".to_owned(),
            language: "en".to_owned(),
            topics: vec!["x".repeat(129)],
        },
    ];

    for classification in invalid {
        assert_eq!(
            classification.validate(),
            Err(DomainError::InvalidClassification)
        );
    }
}

#[derive(Debug)]
struct FakeExtractor {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl DocumentExtractor for FakeExtractor {
    async fn extract(&self, source: &StagedSource) -> Result<ExtractedDocument, ExtractionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ExtractedDocument::new(
            source.clone(),
            "Agent systems source text",
            ExtractionMethod::HtmlPandoc,
        )
        .map_err(|error| ExtractionError::Invalid(error.to_string()))
    }
}

#[derive(Debug)]
struct FakeClassifier {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl DocumentClassifier for FakeClassifier {
    async fn classify(
        &self,
        _document: &ExtractedDocument,
    ) -> Result<DocumentClassification, AnalysisError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(valid_classification())
    }
}

#[derive(Debug, Default)]
struct FakeClassificationStore {
    record: Mutex<Option<ClassificationRecord>>,
}

impl ClassificationStore for FakeClassificationStore {
    fn load(
        &self,
        _digest: &SourceDigest,
    ) -> Result<Option<ClassificationRecord>, ClassificationStoreError> {
        self.record
            .lock()
            .map(|record| record.clone())
            .map_err(|_| ClassificationStoreError("fake store lock was poisoned".to_owned()))
    }

    fn save(&self, record: &ClassificationRecord) -> Result<(), ClassificationStoreError> {
        *self
            .record
            .lock()
            .map_err(|_| ClassificationStoreError("fake store lock was poisoned".to_owned()))? =
            Some(record.clone());
        Ok(())
    }
}

#[tokio::test]
async fn classification_pipeline_persists_and_reuses_digest_records()
-> Result<(), Box<dyn std::error::Error>> {
    let extractor_calls = Arc::new(AtomicUsize::new(0));
    let classifier_calls = Arc::new(AtomicUsize::new(0));
    let pipeline = ClassificationPipeline::new(
        FakeExtractor {
            calls: Arc::clone(&extractor_calls),
        },
        FakeClassifier {
            calls: Arc::clone(&classifier_calls),
        },
        FakeClassificationStore::default(),
    );
    let source = staged_source()?;
    let provenance = AnalysisProvenance {
        function: "ClassifyDocument".to_owned(),
        client: "LocalClassifier".to_owned(),
        model: "local-model".to_owned(),
        pipeline_version: "0.1.0".to_owned(),
        processed_at: "2026-09-19T00:00:00Z".to_owned(),
    };

    let first = pipeline.classify(&source, provenance.clone()).await?;
    let second = pipeline.classify(&source, provenance).await?;

    assert_eq!(first, second);
    assert_eq!(first.classification, valid_classification());
    assert_eq!(extractor_calls.load(Ordering::SeqCst), 1);
    assert_eq!(classifier_calls.load(Ordering::SeqCst), 1);
    Ok(())
}

fn valid_classification() -> DocumentClassification {
    DocumentClassification {
        title: "Agent Systems".to_owned(),
        authors: vec!["Researcher".to_owned()],
        source_type: "paper".to_owned(),
        language: "en".to_owned(),
        topics: vec!["agents".to_owned()],
    }
}

fn staged_source() -> Result<StagedSource, DomainError> {
    Ok(StagedSource::new(
        PathBuf::from("paper.html"),
        SourceDigest::from_bytes(b"source"),
        SourceFileName::new("paper.html")?,
        DocumentKind::Html,
    ))
}
