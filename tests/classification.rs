//! Tests classification validation, routing, fallback, and cache behavior.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use episteme::classification::{CachePolicy, ClassificationPipeline, RoutedDocumentClassifier};
use episteme::config::{ClassifierProfile, LocalModelEndpoint};
use episteme::domain::{
    AnalysisProvenance, ClassificationCacheKey, ClassificationFailure, ClassificationFailureCode,
    ClassificationOutcome, ClassificationRecord, DocumentClassification, DocumentKind,
    DocumentSourceType, DomainError, ExtractedDocument, ExtractionMethod, ModelAvailability,
    SourceDigest, SourceFileName, StagedSource,
};
use episteme::ports::{
    ClassificationStore, ClassificationStoreError, DocumentClassifier, DocumentExtractor,
    ExtractionError, ModelProbe, ProfileClassifier,
};

#[test]
fn classification_rejects_missing_required_fields() {
    let invalid = [
        DocumentClassification {
            title: " ".to_owned(),
            authors: vec!["Researcher".to_owned()],
            source_type: DocumentSourceType::ResearchPaper,
            language_code: "en".to_owned(),
            topics: vec!["agents".to_owned()],
        },
        DocumentClassification {
            title: "Agent Systems".to_owned(),
            authors: vec!["Researcher".to_owned()],
            source_type: DocumentSourceType::ResearchPaper,
            language_code: String::new(),
            topics: vec!["agents".to_owned()],
        },
        DocumentClassification {
            title: "Agent Systems".to_owned(),
            authors: vec!["Researcher".to_owned()],
            source_type: DocumentSourceType::ResearchPaper,
            language_code: "en".to_owned(),
            topics: Vec::new(),
        },
        DocumentClassification {
            title: "Agent\nSystems".to_owned(),
            authors: vec!["Researcher".to_owned()],
            source_type: DocumentSourceType::ResearchPaper,
            language_code: "en".to_owned(),
            topics: vec!["agents".to_owned()],
        },
        DocumentClassification {
            title: "Agent Systems".to_owned(),
            authors: vec!["Researcher".to_owned()],
            source_type: DocumentSourceType::ResearchPaper,
            language_code: "en".to_owned(),
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

#[test]
fn classification_normalizes_taxonomy_and_rejects_placeholders() {
    let normalized = DocumentClassification {
        title: " Agent Systems ".to_owned(),
        authors: vec![" Researcher ".to_owned(), "researcher".to_owned()],
        source_type: DocumentSourceType::Documentation,
        language_code: "EN".to_owned(),
        topics: vec![" Agent Systems ".to_owned(), "agent systems".to_owned()],
    }
    .normalize()
    .expect("valid metadata should normalize");

    assert_eq!(normalized.title, "Agent Systems");
    assert_eq!(normalized.authors, ["Researcher"]);
    assert_eq!(normalized.language_code, "en");
    assert_eq!(normalized.topics, ["agent systems"]);

    for invalid in [
        DocumentClassification {
            title: "string".to_owned(),
            authors: Vec::new(),
            source_type: DocumentSourceType::Other,
            language_code: "en".to_owned(),
            topics: vec!["agents".to_owned()],
        },
        DocumentClassification {
            title: "Agent Systems".to_owned(),
            authors: Vec::new(),
            source_type: DocumentSourceType::Other,
            language_code: "text".to_owned(),
            topics: vec!["agents".to_owned()],
        },
        DocumentClassification {
            title: "Agent Systems".to_owned(),
            authors: vec!["<source_document>".to_owned()],
            source_type: DocumentSourceType::Other,
            language_code: "en".to_owned(),
            topics: vec!["agents".to_owned()],
        },
        DocumentClassification {
            title: "Agent Systems".to_owned(),
            authors: vec!["string".to_owned()],
            source_type: DocumentSourceType::Other,
            language_code: "en".to_owned(),
            topics: vec!["agents".to_owned()],
        },
        DocumentClassification {
            title: "Agent Systems".to_owned(),
            authors: Vec::new(),
            source_type: DocumentSourceType::Other,
            language_code: "en".to_owned(),
            topics: vec!["string[]".to_owned()],
        },
    ] {
        assert_eq!(invalid.normalize(), Err(DomainError::InvalidClassification));
    }
}

#[derive(Debug)]
struct AlwaysOverflowClassifier;

#[async_trait]
impl ProfileClassifier for AlwaysOverflowClassifier {
    async fn classify_with_profile(
        &self,
        _document: &ExtractedDocument,
        _profile: &ClassifierProfile,
    ) -> Result<DocumentClassification, ClassificationFailure> {
        Err(ClassificationFailure::retryable(
            ClassificationFailureCode::ContextOverflow,
            "configured model context was exceeded",
        ))
    }
}

#[tokio::test]
async fn router_marks_final_context_overflow_terminal() -> Result<(), Box<dyn std::error::Error>> {
    let router = RoutedDocumentClassifier::new(
        vec![profile("small", 100)?],
        AlwaysOverflowClassifier,
        FakeProbe,
        "classification-v2",
    );
    let document = ExtractedDocument::new(
        staged_source()?,
        "private source content",
        ExtractionMethod::HtmlPandoc,
    )?;

    let failure = router
        .classify(&document)
        .await
        .expect_err("final context overflow must fail");

    assert_eq!(failure.code, ClassificationFailureCode::ContextOverflow);
    assert!(!failure.retryable);
    assert_eq!(failure.model.as_deref(), Some("small-model"));
    assert_eq!(failure.extracted_characters, Some(22));
    Ok(())
}

#[derive(Debug)]
struct FailingExtractor;

#[async_trait]
impl DocumentExtractor for FailingExtractor {
    async fn extract(&self, _source: &StagedSource) -> Result<ExtractedDocument, ExtractionError> {
        Err(ExtractionError::Tool(
            "/Users/private/bin/pandoc failed on secret source".to_owned(),
        ))
    }
}

#[tokio::test]
async fn classification_processor_redacts_extraction_paths()
-> Result<(), Box<dyn std::error::Error>> {
    use episteme::classification::ClassificationProcessor;

    let pipeline = ClassificationPipeline::new(
        FailingExtractor,
        FakeClassifier {
            calls: Arc::new(AtomicUsize::new(0)),
        },
        FakeClassificationStore::default(),
    );

    let failure = pipeline
        .process(&staged_source()?, CachePolicy::Use)
        .await
        .expect_err("extraction must fail");

    assert_eq!(failure.code, ClassificationFailureCode::Extraction);
    assert!(!failure.message.contains("/Users/private"));
    assert!(!failure.message.contains("secret source"));
    Ok(())
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
    fn cache_keys(&self, _extracted_characters: usize) -> Vec<ClassificationCacheKey> {
        vec![ClassificationCacheKey {
            model: "local-model".to_owned(),
            policy_version: "classification-v2".to_owned(),
        }]
    }

    async fn classify(
        &self,
        _document: &ExtractedDocument,
    ) -> Result<ClassificationOutcome, ClassificationFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ClassificationOutcome {
            classification: valid_classification(),
            analysis: AnalysisProvenance {
                function: "ClassifyDocument".to_owned(),
                client: "fake".to_owned(),
                model: "local-model".to_owned(),
                pipeline_version: "classification-v2".to_owned(),
                processed_at: "2026-09-19T00:00:00Z".to_owned(),
            },
            extracted_characters: 25,
        })
    }
}

#[derive(Debug, Default)]
struct FakeClassificationStore {
    record: Mutex<Option<(ClassificationCacheKey, ClassificationRecord)>>,
}

impl ClassificationStore for FakeClassificationStore {
    fn load(
        &self,
        _digest: &SourceDigest,
        key: &ClassificationCacheKey,
    ) -> Result<Option<ClassificationRecord>, ClassificationStoreError> {
        self.record
            .lock()
            .map(|record| {
                record
                    .as_ref()
                    .and_then(|(stored_key, record)| (stored_key == key).then(|| record.clone()))
            })
            .map_err(|_| ClassificationStoreError("fake store lock was poisoned".to_owned()))
    }

    fn save(
        &self,
        key: &ClassificationCacheKey,
        record: &ClassificationRecord,
    ) -> Result<(), ClassificationStoreError> {
        *self
            .record
            .lock()
            .map_err(|_| ClassificationStoreError("fake store lock was poisoned".to_owned()))? =
            Some((key.clone(), record.clone()));
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

    let first = pipeline.classify(&source, CachePolicy::Use).await?;
    let second = pipeline.classify(&source, CachePolicy::Use).await?;

    assert_eq!(first, second);
    assert_eq!(first.classification, valid_classification());
    assert_eq!(extractor_calls.load(Ordering::SeqCst), 2);
    assert_eq!(classifier_calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn classification_cache_requires_matching_key_and_force_refreshes()
-> Result<(), Box<dyn std::error::Error>> {
    let extractor_calls = Arc::new(AtomicUsize::new(0));
    let classifier_calls = Arc::new(AtomicUsize::new(0));
    let mut stale = ClassificationRecord {
        source_digest: SourceDigest::from_bytes(b"source"),
        source_name: SourceFileName::new("paper.html")?,
        extraction_method: ExtractionMethod::HtmlPandoc,
        classification: valid_classification(),
        analysis: AnalysisProvenance {
            function: "ClassifyDocument".to_owned(),
            client: "old".to_owned(),
            model: "old-model".to_owned(),
            pipeline_version: "classification-v2".to_owned(),
            processed_at: "2026-09-18T00:00:00Z".to_owned(),
        },
    };
    stale.classification.title = "Stale Title".to_owned();
    let pipeline = ClassificationPipeline::new(
        FakeExtractor {
            calls: Arc::clone(&extractor_calls),
        },
        FakeClassifier {
            calls: Arc::clone(&classifier_calls),
        },
        FakeClassificationStore {
            record: Mutex::new(Some((
                ClassificationCacheKey {
                    model: "old-model".to_owned(),
                    policy_version: "classification-v2".to_owned(),
                },
                stale,
            ))),
        },
    );
    let source = staged_source()?;

    let refreshed_mismatch = pipeline.classify(&source, CachePolicy::Use).await?;
    let forced = pipeline.classify(&source, CachePolicy::Refresh).await?;

    assert_eq!(refreshed_mismatch.classification.title, "Agent Systems");
    assert_eq!(forced.classification.title, "Agent Systems");
    assert_eq!(classifier_calls.load(Ordering::SeqCst), 2);
    assert_eq!(extractor_calls.load(Ordering::SeqCst), 2);
    Ok(())
}

fn valid_classification() -> DocumentClassification {
    DocumentClassification {
        title: "Agent Systems".to_owned(),
        authors: vec!["Researcher".to_owned()],
        source_type: DocumentSourceType::ResearchPaper,
        language_code: "en".to_owned(),
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

#[derive(Debug)]
struct FakeProbe;

#[async_trait]
impl ModelProbe for FakeProbe {
    async fn probe(
        &self,
        profile: &ClassifierProfile,
    ) -> Result<ModelAvailability, ClassificationFailure> {
        Ok(ModelAvailability {
            advertised_models: vec![profile.endpoint.model().to_owned()],
        })
    }
}

#[derive(Debug, Default)]
struct FakeProfileClassifier {
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl ProfileClassifier for FakeProfileClassifier {
    async fn classify_with_profile(
        &self,
        _document: &ExtractedDocument,
        profile: &ClassifierProfile,
    ) -> Result<DocumentClassification, ClassificationFailure> {
        self.calls
            .lock()
            .map_err(|_| {
                ClassificationFailure::terminal(
                    ClassificationFailureCode::ModelRequest,
                    "fake classifier lock failed",
                )
            })?
            .push(profile.name.clone());
        if profile.name == "small" {
            return Err(ClassificationFailure::retryable(
                ClassificationFailureCode::ContextOverflow,
                "configured model context was exceeded",
            ));
        }
        Ok(valid_classification())
    }
}

#[tokio::test]
async fn router_falls_back_after_context_overflow_and_preserves_safe_failure()
-> Result<(), Box<dyn std::error::Error>> {
    let classifier = FakeProfileClassifier::default();
    let router = RoutedDocumentClassifier::new(
        vec![profile("small", 100)?, profile("large", 10_000)?],
        classifier,
        FakeProbe,
        "classification-v2",
    );
    let document = ExtractedDocument::new(
        staged_source()?,
        "private source content",
        ExtractionMethod::HtmlPandoc,
    )?;

    let outcome: ClassificationOutcome = router.classify(&document).await?;

    assert_eq!(outcome.analysis.model, "large-model");
    assert_eq!(outcome.extracted_characters, 22);
    assert_eq!(outcome.classification, valid_classification());
    Ok(())
}

fn profile(name: &str, maximum_input_characters: usize) -> Result<ClassifierProfile, DomainError> {
    let endpoint = LocalModelEndpoint::new(
        if name == "small" {
            "http://127.0.0.1:18181/v1"
        } else {
            "http://127.0.0.1:18182/v1"
        },
        format!("{name}-model"),
    )
    .map_err(|_| DomainError::InvalidClassification)?;
    Ok(ClassifierProfile {
        name: name.to_owned(),
        endpoint,
        maximum_input_characters,
    })
}
