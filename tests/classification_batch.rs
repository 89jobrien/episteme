use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use episteme::adapters::DuckDbIngestionStore;
use episteme::classification::{
    BatchOptions, CachePolicy, ClassificationBatch, ClassificationExecution,
    ClassificationProcessor,
};
use episteme::domain::{
    AnalysisProvenance, ClassificationBatchSource, ClassificationFailure,
    ClassificationFailureCode, ClassificationRecord, DocumentClassification, DocumentKind,
    DocumentSourceType, ExtractionMethod, SourceDigest, SourceFileName, StagedSource,
};

#[derive(Debug, Default)]
struct FakeProcessor {
    calls: Mutex<HashMap<String, usize>>,
}

#[async_trait]
impl ClassificationProcessor for FakeProcessor {
    async fn process(
        &self,
        source: &StagedSource,
        _cache_policy: CachePolicy,
    ) -> Result<ClassificationExecution, ClassificationFailure> {
        let name = source.source_name().as_str().to_owned();
        let mut calls = self.calls.lock().map_err(|_| {
            ClassificationFailure::terminal(
                ClassificationFailureCode::Persistence,
                "fake processor lock failed",
            )
        })?;
        let call = calls.entry(name.clone()).or_default();
        *call += 1;
        if name == "retry.html" && *call == 1 {
            return Err(ClassificationFailure::retryable(
                ClassificationFailureCode::EndpointUnavailable,
                "local classifier endpoint was unavailable",
            ));
        }
        if name == "terminal.html" {
            return Err(ClassificationFailure::terminal(
                ClassificationFailureCode::InvalidOutput,
                "generated classification violated metadata policy",
            ));
        }
        Ok(ClassificationExecution {
            record: record(source),
            cache_hit: false,
            extracted_characters: 20,
        })
    }
}

#[tokio::test]
async fn batch_resumes_retries_and_reconciles_final_status()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let store = Arc::new(DuckDbIngestionStore::open(
        &directory.path().join("episteme.duckdb"),
    )?);
    let processor = Arc::new(FakeProcessor::default());
    let batch = ClassificationBatch::new(Arc::clone(&processor), Arc::clone(&store), 2)?;
    let sources = vec![
        source("success.html")?,
        source("retry.html")?,
        source("terminal.html")?,
    ];

    let first = batch
        .run(
            "batch-1",
            sources.clone(),
            BatchOptions {
                cache_policy: CachePolicy::Use,
                retry_failed: false,
            },
        )
        .await?;
    assert_eq!(first.succeeded, 1);
    assert_eq!(first.failed_retryable, 1);
    assert_eq!(first.failed_terminal, 1);

    let resumed = batch
        .run(
            "batch-1",
            sources,
            BatchOptions {
                cache_policy: CachePolicy::Use,
                retry_failed: true,
            },
        )
        .await?;

    assert_eq!(resumed.succeeded, 2);
    assert_eq!(resumed.failed_retryable, 0);
    assert_eq!(resumed.failed_terminal, 1);
    assert_eq!(
        resumed
            .attempts
            .iter()
            .map(|attempt| attempt.source_path.as_str())
            .collect::<Vec<_>>(),
        ["retry.html", "success.html", "terminal.html"]
    );
    assert_eq!(
        processor.calls.lock().map_err(|_| "call lock failed")?["success.html"],
        1
    );
    assert_eq!(
        processor.calls.lock().map_err(|_| "call lock failed")?["retry.html"],
        2
    );
    assert_eq!(
        processor.calls.lock().map_err(|_| "call lock failed")?["terminal.html"],
        1
    );

    let refreshed = batch
        .run(
            "batch-1",
            vec![source_with_content("success.html", b"changed source")?],
            BatchOptions {
                cache_policy: CachePolicy::Refresh,
                retry_failed: false,
            },
        )
        .await?;
    assert_eq!(refreshed.total, 1);
    assert_eq!(refreshed.succeeded, 1);
    assert_eq!(refreshed.attempts[0].source_path, "success.html");
    assert_eq!(
        processor.calls.lock().map_err(|_| "call lock failed")?["success.html"],
        2
    );

    let duplicate = batch
        .run(
            "batch-duplicates",
            vec![source("same.html")?, source("same.html")?],
            BatchOptions {
                cache_policy: CachePolicy::Use,
                retry_failed: false,
            },
        )
        .await;
    assert!(duplicate.is_err());
    Ok(())
}

fn source(name: &str) -> Result<ClassificationBatchSource, Box<dyn std::error::Error>> {
    source_with_content(name, name.as_bytes())
}

fn source_with_content(
    name: &str,
    content: &[u8],
) -> Result<ClassificationBatchSource, Box<dyn std::error::Error>> {
    Ok(ClassificationBatchSource::Ready {
        relative_path: name.to_owned(),
        source: StagedSource::new(
            PathBuf::from(name),
            SourceDigest::from_bytes(content),
            SourceFileName::new(name)?,
            DocumentKind::Html,
        ),
    })
}

fn record(source: &StagedSource) -> ClassificationRecord {
    ClassificationRecord {
        source_digest: source.digest().clone(),
        source_name: source.source_name().clone(),
        extraction_method: ExtractionMethod::HtmlPandoc,
        classification: DocumentClassification {
            title: "Agent Systems".to_owned(),
            authors: Vec::new(),
            source_type: DocumentSourceType::Documentation,
            language_code: "en".to_owned(),
            topics: vec!["agent systems".to_owned()],
        },
        analysis: AnalysisProvenance {
            function: "ClassifyDocument".to_owned(),
            client: "large".to_owned(),
            model: "large-model".to_owned(),
            pipeline_version: "classification-v2".to_owned(),
            processed_at: "2026-09-19T00:00:00Z".to_owned(),
        },
    }
}
