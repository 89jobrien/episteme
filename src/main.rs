//! Parses CLI commands and coordinates local document-processing workflows.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use clap::{Parser, Subcommand};
use episteme::adapters::{
    BamlDocumentClassifier, BamlDocumentIntelligenceAnalyzer, BamlResearchAnalyzer,
    DocumentCliExtractor, DuckDbIngestionStore, HttpModelProbe, SystemDocumentTools,
    VaultFileStore, ZkIndexer, prepare_vault_directory,
};
use episteme::classification::{
    BatchOptions, CLASSIFICATION_POLICY_VERSION, CachePolicy, ClassificationBatch,
    ClassificationPipeline, RoutedDocumentClassifier,
};
use episteme::config::Settings;
use episteme::doctor::Doctor;
use episteme::domain::{
    AnalysisProvenance, ClassificationBatchSource, ClassificationFailure, ClassificationFailureCode,
};
use episteme::ingest::Ingestor;
use episteme::intelligence::DocumentIntelligencePipeline;
use episteme::stage::stage_source;
use episteme::watch::{StableFileTracker, WatchDecision};

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[arg(long, default_value = "episteme.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check configuration and required executables.
    Doctor,
    /// Create configured runtime directories.
    Init,
    /// Ingest one stable source from the configured inbox.
    Ingest {
        /// Source path beneath the configured inbox.
        source: PathBuf,
    },
    /// Classify one stable source from the configured inbox.
    Classify {
        /// Ignore matching versioned cache entries.
        #[arg(long)]
        force: bool,
        /// Source path beneath the configured inbox.
        source: PathBuf,
    },
    /// Classify every supported source beneath the configured inbox.
    ClassifyBatch {
        /// Ignore matching versioned cache entries.
        #[arg(long)]
        force: bool,
        /// Retry sources whose latest attempt is retryable.
        #[arg(long)]
        retry_failed: bool,
        /// Maximum concurrent local model requests.
        #[arg(long, default_value_t = 2)]
        concurrency: usize,
        /// Durable batch identity used for resume and reconciliation.
        #[arg(long, default_value = "inbox")]
        batch_id: String,
        /// Optional path for one atomically reconciled JSON summary.
        #[arg(long)]
        summary: Option<PathBuf>,
    },
    /// Extract evidence-grounded summary, claims, entities, and relations.
    Analyze {
        #[arg(long)]
        force: bool,
        source: PathBuf,
    },
    /// Poll the configured inbox and ingest stable files.
    Watch,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let settings = Settings::load(&cli.config)
        .with_context(|| format!("failed to load {}", cli.config.display()))?;
    match cli.command {
        Command::Doctor => run_doctor(&settings),
        Command::Init => initialize_directories(&settings),
        Command::Ingest { source } => {
            let run = ingest_path(&settings, &source).await?;
            println!("{}: {}", run.digest, run.stage.as_str());
            Ok(())
        }
        Command::Classify { force, source } => {
            let record = classify_path(
                &settings,
                &source,
                if force {
                    CachePolicy::Refresh
                } else {
                    CachePolicy::Use
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&record)?);
            Ok(())
        }
        Command::ClassifyBatch {
            force,
            retry_failed,
            concurrency,
            batch_id,
            summary,
        } => {
            let result = classify_batch(
                &settings,
                &batch_id,
                BatchOptions {
                    cache_policy: if force {
                        CachePolicy::Refresh
                    } else {
                        CachePolicy::Use
                    },
                    retry_failed,
                },
                concurrency,
            )
            .await?;
            if let Some(path) = summary {
                write_summary(&path, &result)?;
            }
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Command::Analyze { force, source } => {
            let result = analyze_path(
                &settings,
                &source,
                if force {
                    CachePolicy::Refresh
                } else {
                    CachePolicy::Use
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Command::Watch => watch(&settings).await,
    }
}

async fn analyze_path(
    settings: &Settings,
    source_path: &Path,
    cache_policy: CachePolicy,
) -> Result<episteme::domain::DocumentIntelligence> {
    let inbox = settings.vault_root.join(&settings.inbox_directory);
    let state_root = settings
        .database_path
        .parent()
        .context("database path must have a parent directory")?;
    let source = stage_source(
        &inbox,
        source_path,
        &state_root.join("staging"),
        settings.maximum_source_bytes,
    )?;
    let timeout = Duration::from_secs(settings.process_timeout_seconds);
    let tools = SystemDocumentTools::new(
        settings.tools.clone(),
        timeout,
        settings.maximum_output_bytes,
    );
    let extractor = DocumentCliExtractor::new(tools, settings.minimum_text_characters);
    let profiles = std::iter::once(settings.classifier.clone())
        .chain(settings.classifier_fallbacks.iter().cloned())
        .collect();
    let classifier = RoutedDocumentClassifier::new(
        profiles,
        BamlDocumentClassifier::new(timeout),
        HttpModelProbe::new(Duration::from_secs(
            settings.process_timeout_seconds.min(10),
        ))?,
        CLASSIFICATION_POLICY_VERSION,
    );
    let analyzer = BamlDocumentIntelligenceAnalyzer::new(settings.distiller.clone(), timeout);
    let store = DuckDbIngestionStore::open(&settings.database_path)?;
    DocumentIntelligencePipeline::new(
        extractor,
        classifier,
        analyzer,
        store,
        settings.distiller.model(),
    )
    .analyze(&source, cache_policy)
    .await
    .map_err(Into::into)
}

async fn classify_path(
    settings: &Settings,
    source_path: &Path,
    cache_policy: CachePolicy,
) -> Result<episteme::domain::ClassificationRecord> {
    let inbox = settings.vault_root.join(&settings.inbox_directory);
    let state_root = settings
        .database_path
        .parent()
        .context("database path must have a parent directory")?;
    let source = stage_source(
        &inbox,
        source_path,
        &state_root.join("staging"),
        settings.maximum_source_bytes,
    )?;
    let timeout = Duration::from_secs(settings.process_timeout_seconds);
    let tools = SystemDocumentTools::new(
        settings.tools.clone(),
        timeout,
        settings.maximum_output_bytes,
    );
    let extractor = DocumentCliExtractor::new(tools, settings.minimum_text_characters);
    let profiles = std::iter::once(settings.classifier.clone())
        .chain(settings.classifier_fallbacks.iter().cloned())
        .collect();
    let profile_classifier = BamlDocumentClassifier::new(timeout);
    let probe = HttpModelProbe::new(Duration::from_secs(
        settings.process_timeout_seconds.min(10),
    ))?;
    let classifier = RoutedDocumentClassifier::new(
        profiles,
        profile_classifier,
        probe,
        CLASSIFICATION_POLICY_VERSION,
    );
    let store = DuckDbIngestionStore::open(&settings.database_path)?;
    ClassificationPipeline::new(extractor, classifier, store)
        .classify(&source, cache_policy)
        .await
        .map_err(Into::into)
}

async fn classify_batch(
    settings: &Settings,
    batch_id: &str,
    options: BatchOptions,
    concurrency: usize,
) -> Result<episteme::domain::ClassificationBatchSummary> {
    let inbox = settings.vault_root.join(&settings.inbox_directory);
    let state_root = settings
        .database_path
        .parent()
        .context("database path must have a parent directory")?;
    let sources = discover_batch_sources(settings, &inbox, &state_root.join("staging"))?;
    let timeout = Duration::from_secs(settings.process_timeout_seconds);
    let tools = SystemDocumentTools::new(
        settings.tools.clone(),
        timeout,
        settings.maximum_output_bytes,
    );
    let extractor = DocumentCliExtractor::new(tools, settings.minimum_text_characters);
    let profiles = std::iter::once(settings.classifier.clone())
        .chain(settings.classifier_fallbacks.iter().cloned())
        .collect();
    let profile_classifier = BamlDocumentClassifier::new(timeout);
    let probe = HttpModelProbe::new(Duration::from_secs(
        settings.process_timeout_seconds.min(10),
    ))?;
    let classifier = RoutedDocumentClassifier::new(
        profiles,
        profile_classifier,
        probe,
        CLASSIFICATION_POLICY_VERSION,
    );
    let store = Arc::new(DuckDbIngestionStore::open(&settings.database_path)?);
    let processor = Arc::new(ClassificationPipeline::new(
        extractor,
        classifier,
        Arc::clone(&store),
    ));
    ClassificationBatch::new(processor, store, concurrency)?
        .run(batch_id, sources, options)
        .await
        .map_err(Into::into)
}

fn discover_batch_sources(
    settings: &Settings,
    inbox: &Path,
    staging: &Path,
) -> Result<Vec<ClassificationBatchSource>> {
    let canonical_inbox = inbox
        .canonicalize()
        .context("failed to resolve configured inbox")?;
    let mut directories = vec![canonical_inbox.clone()];
    let mut sources = Vec::new();
    while let Some(directory) = directories.pop() {
        let canonical_directory = directory
            .canonicalize()
            .context("failed to resolve inbox directory")?;
        if !canonical_directory.starts_with(&canonical_inbox) {
            bail!("inbox directory escaped configured root");
        }
        for entry in fs::read_dir(&canonical_directory)
            .context("failed to read configured inbox directory")?
        {
            let path = entry?.path();
            let relative = path
                .strip_prefix(&canonical_inbox)
                .context("discovered source escaped inbox")?
                .to_string_lossy()
                .into_owned();
            let metadata = fs::symlink_metadata(&path).context("failed to inspect inbox entry")?;
            if metadata.file_type().is_symlink() {
                sources.push(ClassificationBatchSource::Rejected {
                    relative_path: relative,
                    failure: ClassificationFailure::terminal(
                        ClassificationFailureCode::Staging,
                        "batch source traversed a symlink",
                    ),
                });
            } else if metadata.is_dir() {
                directories.push(path);
            } else if metadata.is_file() && supported_batch_source(&path) {
                match stage_source(
                    &canonical_inbox,
                    &path,
                    staging,
                    settings.maximum_source_bytes,
                ) {
                    Ok(source) => sources.push(ClassificationBatchSource::Ready {
                        relative_path: relative,
                        source,
                    }),
                    Err(_) => sources.push(ClassificationBatchSource::Rejected {
                        relative_path: relative,
                        failure: ClassificationFailure::terminal(
                            ClassificationFailureCode::Staging,
                            "batch source staging failed",
                        ),
                    }),
                }
            }
        }
    }
    Ok(sources)
}

fn supported_batch_source(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "pdf" | "html" | "htm" | "png" | "jpg" | "jpeg" | "tif" | "tiff" | "webp"
            )
        })
}

fn write_summary(
    path: &Path,
    summary: &episteme::domain::ClassificationBatchSummary,
) -> Result<()> {
    let parent = path
        .parent()
        .context("classification summary path must have a parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, summary)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to persist {}", path.display()))?;
    Ok(())
}

fn run_doctor(settings: &Settings) -> Result<()> {
    let report = Doctor::check_tools(&settings.required_tools());
    for check in &report.checks {
        let status = if check.available { "ok" } else { "missing" };
        println!("{status}: {} ({})", check.name, check.path.display());
    }
    if !report.is_healthy() {
        bail!("one or more required tools are unavailable");
    }
    Ok(())
}

fn initialize_directories(settings: &Settings) -> Result<()> {
    prepare_vault_directory(&settings.vault_root, &settings.inbox_directory)?;
    VaultFileStore::new(
        settings.vault_root.clone(),
        settings.research_directory.clone(),
        settings.archive_directory.clone(),
    )
    .prepare_directories()?;
    if let Some(parent) = settings.database_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let _store = DuckDbIngestionStore::open(&settings.database_path)?;
    Ok(())
}

async fn ingest_path(
    settings: &Settings,
    source_path: &Path,
) -> Result<episteme::domain::IngestionRun> {
    let inbox = settings.vault_root.join(&settings.inbox_directory);
    let state_root = settings
        .database_path
        .parent()
        .context("database path must have a parent directory")?;
    let source = stage_source(
        &inbox,
        source_path,
        &state_root.join("staging"),
        settings.maximum_source_bytes,
    )?;
    let timeout = Duration::from_secs(settings.process_timeout_seconds);
    let tools = SystemDocumentTools::new(
        settings.tools.clone(),
        timeout,
        settings.maximum_output_bytes,
    );
    let extractor = DocumentCliExtractor::new(tools, settings.minimum_text_characters);
    let analyzer = BamlResearchAnalyzer::new(
        settings.classifier.endpoint.clone(),
        settings.distiller.clone(),
        timeout,
    );
    let vault = VaultFileStore::new(
        settings.vault_root.clone(),
        settings.research_directory.clone(),
        settings.archive_directory.clone(),
    );
    let store = DuckDbIngestionStore::open(&settings.database_path)?;
    let indexer = ZkIndexer::new(
        settings.tools.zk.clone(),
        settings.vault_root.clone(),
        timeout,
        settings.maximum_output_bytes,
    );
    let ingestor = Ingestor::new(extractor, analyzer, vault, store, indexer);
    ingestor
        .ingest(
            &source,
            AnalysisProvenance {
                function: "ResearchDistillationPipeline".to_owned(),
                client: "LocalClassifier+LocalDistiller".to_owned(),
                model: format!(
                    "classifier={};distiller={}",
                    settings.classifier.endpoint.model(),
                    settings.distiller.model()
                ),
                pipeline_version: env!("CARGO_PKG_VERSION").to_owned(),
                processed_at: Utc::now().to_rfc3339(),
            },
        )
        .await
        .map_err(Into::into)
}

async fn watch(settings: &Settings) -> Result<()> {
    let inbox = settings.vault_root.join(&settings.inbox_directory);
    let mut tracker = StableFileTracker::new(&inbox)?;
    let mut interval = tokio::time::interval(Duration::from_secs(settings.watch_interval_seconds));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                for entry in fs::read_dir(&inbox)
                    .with_context(|| format!("failed to read {}", inbox.display()))?
                {
                    let path = entry?.path();
                    if let WatchDecision::Ready(path) = tracker.observe(&path)
                        && let Err(error) = ingest_path(settings, &path).await
                    {
                        eprintln!("ingestion failed for {}: {error:#}", path.display());
                    }
                }
            }
            signal = tokio::signal::ctrl_c() => {
                signal.context("failed to listen for shutdown signal")?;
                return Ok(());
            }
        }
    }
}
