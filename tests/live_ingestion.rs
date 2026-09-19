use std::fs;

use assert_cmd::Command;
use episteme::config::validate_live_test_vault;

#[test]
#[ignore = "requires EPISTEME_RUN_LIVE=1 and configured local model endpoints"]
fn live_ingestion_uses_temporary_vault() -> Result<(), Box<dyn std::error::Error>> {
    run_live_ingestion(
        "<html><body><h1>Atomic Ingestion</h1><p>Author: Test Researcher.</p><p>Citation: Test Researcher, Atomic Ingestion, 2026.</p><h2>Summary</h2><p>This fixture evaluates local document ingestion.</p><h2>Key Idea</h2><p>Atomic ingestion preserves grounded source evidence.</p><h2>Implementation</h2><p>Write validated notes before archiving source copies.</p><h2>Critique</h2><p>The fixture is intentionally narrow.</p></body></html>",
        false,
    )
}

#[test]
#[ignore = "requires EPISTEME_RUN_LIVE=1 and configured local model endpoints"]
fn live_chunked_ingestion_uses_source_span_evidence() -> Result<(), Box<dyn std::error::Error>> {
    let repeated_section = "<h2>Grounded Processing</h2><p>Atomic ingestion preserves grounded source evidence while chunked distillation keeps large documents within local model context limits.</p><p>Implementation requires bounded chunk processing, exact evidence quotes, deterministic ordering, and validation before vault mutation.</p><p>Critique: aggregation can omit useful details, so every final claim must remain supported by verbatim source evidence.</p>";
    let document = format!(
        "<html><body><h1>Atomic Ingestion</h1><p>Author: Test Researcher.</p><p>Citation: Test Researcher, Atomic Ingestion, 2026.</p>{}</body></html>",
        repeated_section.repeat(40)
    );
    run_live_ingestion(&document, true)
}

fn run_live_ingestion(
    document: &str,
    expect_chunked: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("EPISTEME_RUN_LIVE").as_deref() != Ok("1") {
        return Err("set EPISTEME_RUN_LIVE=1 to run live ingestion".into());
    }
    let classifier_url = required_env("EPISTEME_CLASSIFY_BASE_URL")?;
    let classifier_model = required_env("EPISTEME_CLASSIFY_MODEL")?;
    let distiller_url = required_env("EPISTEME_DISTILL_BASE_URL")?;
    let distiller_model = required_env("EPISTEME_DISTILL_MODEL")?;
    let temporary_root = tempfile::tempdir()?;
    let vault = temporary_root.path().join("vault");
    fs::create_dir(&vault)?;
    validate_live_test_vault(&vault, temporary_root.path())?;
    let zk_status = std::process::Command::new("/opt/homebrew/bin/zk")
        .args([
            "init",
            vault.to_str().ok_or("vault path is not UTF-8")?,
            "--no-input",
        ])
        .status()?;
    assert!(zk_status.success());

    let config = temporary_root.path().join("episteme.toml");
    let database = temporary_root.path().join("state/episteme.duckdb");
    fs::write(
        &config,
        format!(
            r#"vault_root = {vault}
inbox_directory = "00_Inbox/_Incoming"
research_directory = "04_Research"
archive_directory = "09_Archive/Sources"
database_path = {database}
minimum_text_characters = 20
process_timeout_seconds = 300
maximum_output_bytes = 1048576
maximum_source_bytes = 1048576
watch_interval_seconds = 1

[tools]
pdftotext = "/opt/homebrew/bin/pdftotext"
pdftoppm = "/opt/homebrew/bin/pdftoppm"
tesseract = "/opt/homebrew/bin/tesseract"
pandoc = "/opt/homebrew/bin/pandoc"
zk = "/opt/homebrew/bin/zk"

[classifier]
base_url = {classifier_url}
model = {classifier_model}

[distiller]
base_url = {distiller_url}
model = {distiller_model}
"#,
            vault = json_string(&vault.display().to_string())?,
            database = json_string(&database.display().to_string())?,
            classifier_url = json_string(&classifier_url)?,
            classifier_model = json_string(&classifier_model)?,
            distiller_url = json_string(&distiller_url)?,
            distiller_model = json_string(&distiller_model)?,
        ),
    )?;

    Command::cargo_bin("episteme")?
        .args([
            "--config",
            config.to_str().ok_or("config path is not UTF-8")?,
            "init",
        ])
        .assert()
        .success();
    let source = vault.join("00_Inbox/_Incoming/fixture.html");
    fs::write(&source, document)?;
    Command::cargo_bin("episteme")?
        .args([
            "--config",
            config.to_str().ok_or("config path is not UTF-8")?,
            "ingest",
            source.to_str().ok_or("source path is not UTF-8")?,
        ])
        .assert()
        .success();

    assert_eq!(fs::read_dir(vault.join("04_Research"))?.count(), 1);
    assert_eq!(fs::read_dir(vault.join("09_Archive/Sources"))?.count(), 1);
    let note_path = fs::read_dir(vault.join("04_Research"))?
        .next()
        .ok_or("research note was not created")??
        .path();
    let note = fs::read_to_string(note_path)?;
    assert!(note.contains("baml_function: \"ResearchDistillationPipeline\""));
    assert!(note.contains(&format!(
        "baml_model: \"classifier={classifier_model};distiller={distiller_model}\""
    )));
    if expect_chunked {
        assert!(note.contains("characters "));
        assert!(note.contains("Atomic ingestion preserves grounded source evidence"));
    }
    assert!(source.exists());
    Ok(())
}

fn required_env(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    std::env::var(name).map_err(|_| format!("missing required environment variable {name}").into())
}

fn json_string(value: &str) -> Result<String, Box<dyn std::error::Error>> {
    serde_json::to_string(value).map_err(Into::into)
}
