//! Tests loopback endpoint validation and ordered classifier profile parsing.

use std::fs;

use episteme::config::{LocalModelEndpoint, Settings, SettingsError};

#[test]
fn settings_reject_non_loopback_model_endpoint() {
    let error = LocalModelEndpoint::new("https://api.openai.com/v1", "remote")
        .expect_err("a remote endpoint must be rejected");

    assert!(matches!(error, SettingsError::ModelEndpointNotLoopback(_)));
    assert!(LocalModelEndpoint::new("http://127.0.0.1:11434/v1", "local").is_ok());
}

#[test]
fn settings_parse_ordered_classifier_profiles_with_limits() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let config = directory.path().join("episteme.toml");
    fs::write(
        &config,
        format!(
            r#"
vault_root = "{}"
inbox_directory = "00_Inbox/_Incoming"
research_directory = "04_Research"
archive_directory = "09_Archive/Sources"
database_path = "{}"
minimum_text_characters = 200
process_timeout_seconds = 300
maximum_output_bytes = 10485760
maximum_source_bytes = 104857600
watch_interval_seconds = 5

[tools]
pdftotext = "/bin/true"
pdftoppm = "/bin/true"
tesseract = "/bin/true"
pandoc = "/bin/true"
zk = "/bin/true"

[classifier]
name = "fast"
base_url = "http://127.0.0.1:18181/v1"
model = "small-local"
maximum_input_characters = 12000

[[classifier_fallbacks]]
name = "large"
base_url = "http://127.0.0.1:11434/v1"
model = "large-local"
maximum_input_characters = 120000

[distiller]
base_url = "http://127.0.0.1:18181/v1"
model = "local-distiller"
"#,
            directory.path().display(),
            directory.path().join("episteme.duckdb").display()
        ),
    )?;

    let settings = Settings::load(&config)?;

    assert_eq!(settings.classifier.name, "fast");
    assert_eq!(settings.classifier.maximum_input_characters, 12_000);
    assert_eq!(settings.classifier.endpoint.model(), "small-local");
    assert_eq!(settings.classifier_fallbacks.len(), 1);
    assert_eq!(settings.classifier_fallbacks[0].name, "large");
    assert_eq!(
        settings.classifier_fallbacks[0].maximum_input_characters,
        120_000
    );
    assert_eq!(
        settings.classifier_fallbacks[0].endpoint.model(),
        "large-local"
    );
    Ok(())
}
