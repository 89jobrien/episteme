use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use clap::{Parser, Subcommand};
use episteme::adapters::{
    BamlDocumentClassifier, BamlResearchAnalyzer, DocumentCliExtractor, DuckDbIngestionStore,
    SystemDocumentTools, VaultFileStore, ZkIndexer, prepare_vault_directory,
};
use episteme::classification::ClassificationPipeline;
use episteme::config::Settings;
use episteme::doctor::Doctor;
use episteme::domain::AnalysisProvenance;
use episteme::ingest::Ingestor;
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
        /// Source path beneath the configured inbox.
        source: PathBuf,
    },
    /// Poll the configured inbox and ingest stable files.
    Watch,
}

#[tokio::main]
async fn main() -> Result<()> {
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
        Command::Classify { source } => {
            let record = classify_path(&settings, &source).await?;
            println!("{}", serde_json::to_string_pretty(&record)?);
            Ok(())
        }
        Command::Watch => watch(&settings).await,
    }
}

async fn classify_path(
    settings: &Settings,
    source_path: &Path,
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
    let classifier = BamlDocumentClassifier::new(settings.classifier.clone(), timeout);
    let store = DuckDbIngestionStore::open(&settings.database_path)?;
    ClassificationPipeline::new(extractor, classifier, store)
        .classify(
            &source,
            AnalysisProvenance {
                function: "ClassifyDocument".to_owned(),
                client: "LocalClassifier".to_owned(),
                model: settings.classifier.model().to_owned(),
                pipeline_version: env!("CARGO_PKG_VERSION").to_owned(),
                processed_at: Utc::now().to_rfc3339(),
            },
        )
        .await
        .map_err(Into::into)
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
        settings.classifier.clone(),
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
                function: "DistillResearch".to_owned(),
                client: "LocalDistiller".to_owned(),
                model: settings.distiller.model().to_owned(),
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
