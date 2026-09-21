//! Runtime configuration and safety validation.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;
use url::{Host, Url};

/// A configured local OpenAI-compatible model endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalModelEndpoint {
    base_url: Url,
    model: String,
}

/// One ordered local classifier route with an explicit input bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifierProfile {
    /// Stable operator-facing profile name.
    pub name: String,
    /// Validated loopback-only model endpoint.
    pub endpoint: LocalModelEndpoint,
    /// Maximum extracted Unicode scalar values accepted by this profile.
    pub maximum_input_characters: usize,
}

/// Explicit paths to required document workflow tools.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ToolPaths {
    /// Poppler PDF text extractor.
    pub pdftotext: PathBuf,
    /// Poppler PDF page renderer.
    pub pdftoppm: PathBuf,
    /// Tesseract OCR executable.
    pub tesseract: PathBuf,
    /// Pandoc document converter.
    pub pandoc: PathBuf,
    /// Zettelkasten indexer.
    pub zk: PathBuf,
}

/// Validated Episteme runtime settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Canonical private Obsidian vault root.
    pub vault_root: PathBuf,
    /// Vault-relative inbox directory.
    pub inbox_directory: PathBuf,
    /// Vault-relative research output directory.
    pub research_directory: PathBuf,
    /// Vault-relative source archive directory.
    pub archive_directory: PathBuf,
    /// Local derived-state database path.
    pub database_path: PathBuf,
    /// Required external executable paths.
    pub tools: ToolPaths,
    /// Local classifier endpoint.
    pub classifier: ClassifierProfile,
    /// Ordered local classifier fallback profiles.
    pub classifier_fallbacks: Vec<ClassifierProfile>,
    /// Local distiller endpoint.
    pub distiller: LocalModelEndpoint,
    /// Minimum direct PDF text characters before OCR fallback.
    pub minimum_text_characters: usize,
    /// External process timeout in seconds.
    pub process_timeout_seconds: u64,
    /// Maximum accepted process output bytes.
    pub maximum_output_bytes: usize,
    /// Maximum accepted source bytes.
    pub maximum_source_bytes: u64,
    /// Watcher polling interval in seconds.
    pub watch_interval_seconds: u64,
}

#[derive(Debug, Deserialize)]
struct RawSettings {
    vault_root: PathBuf,
    inbox_directory: PathBuf,
    research_directory: PathBuf,
    archive_directory: PathBuf,
    database_path: PathBuf,
    tools: ToolPaths,
    classifier: RawEndpoint,
    #[serde(default)]
    classifier_fallbacks: Vec<RawEndpoint>,
    distiller: RawEndpoint,
    minimum_text_characters: usize,
    process_timeout_seconds: u64,
    maximum_output_bytes: usize,
    maximum_source_bytes: u64,
    watch_interval_seconds: u64,
}

#[derive(Debug, Deserialize)]
struct RawEndpoint {
    #[serde(default)]
    name: Option<String>,
    base_url: String,
    model: String,
    #[serde(default)]
    maximum_input_characters: Option<usize>,
}

impl Settings {
    /// Loads and validates settings from TOML.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError`] when the file cannot be read, parsed, or violates path and local
    /// endpoint policy.
    pub fn load(path: &Path) -> Result<Self, SettingsError> {
        let text = fs::read_to_string(path).map_err(SettingsError::Read)?;
        let raw: RawSettings = toml::from_str(&text)
            .map_err(|error| SettingsError::InvalidConfiguration(error.to_string()))?;
        for relative in [
            &raw.inbox_directory,
            &raw.research_directory,
            &raw.archive_directory,
        ] {
            validate_relative_path(relative)?;
        }
        for tool in [
            &raw.tools.pdftotext,
            &raw.tools.pdftoppm,
            &raw.tools.tesseract,
            &raw.tools.pandoc,
            &raw.tools.zk,
        ] {
            if !tool.is_absolute() {
                return Err(SettingsError::UnsafeToolPath(tool.display().to_string()));
            }
        }
        if raw.minimum_text_characters == 0
            || raw.process_timeout_seconds == 0
            || raw.maximum_output_bytes == 0
            || raw.maximum_source_bytes == 0
            || raw.watch_interval_seconds == 0
        {
            return Err(SettingsError::InvalidLimit);
        }
        let classifier = classifier_profile(raw.classifier, "primary", false)?;
        let classifier_fallbacks = raw
            .classifier_fallbacks
            .into_iter()
            .enumerate()
            .map(|(index, profile)| {
                classifier_profile(profile, &format!("fallback-{}", index + 1), true)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut names = HashSet::new();
        if !names.insert(classifier.name.clone())
            || classifier_fallbacks
                .iter()
                .any(|profile| !names.insert(profile.name.clone()))
        {
            return Err(SettingsError::DuplicateClassifierProfile);
        }
        Ok(Self {
            vault_root: raw.vault_root,
            inbox_directory: raw.inbox_directory,
            research_directory: raw.research_directory,
            archive_directory: raw.archive_directory,
            database_path: raw.database_path,
            tools: raw.tools,
            classifier,
            classifier_fallbacks,
            distiller: LocalModelEndpoint::new(&raw.distiller.base_url, raw.distiller.model)?,
            minimum_text_characters: raw.minimum_text_characters,
            process_timeout_seconds: raw.process_timeout_seconds,
            maximum_output_bytes: raw.maximum_output_bytes,
            maximum_source_bytes: raw.maximum_source_bytes,
            watch_interval_seconds: raw.watch_interval_seconds,
        })
    }

    /// Returns all required external tools for dependency diagnostics.
    #[must_use]
    pub fn required_tools(&self) -> Vec<crate::doctor::RequiredTool> {
        [
            ("pdftotext", &self.tools.pdftotext),
            ("pdftoppm", &self.tools.pdftoppm),
            ("tesseract", &self.tools.tesseract),
            ("pandoc", &self.tools.pandoc),
            ("zk", &self.tools.zk),
        ]
        .into_iter()
        .map(|(name, path)| crate::doctor::RequiredTool {
            name: name.to_owned(),
            path: path.clone(),
        })
        .collect()
    }
}

fn classifier_profile(
    raw: RawEndpoint,
    default_name: &str,
    require_limit: bool,
) -> Result<ClassifierProfile, SettingsError> {
    let name = match raw.name {
        Some(name) => name,
        None if require_limit => return Err(SettingsError::MissingClassifierProfileName),
        None => default_name.to_owned(),
    };
    if name.trim().is_empty() || name.chars().any(char::is_control) {
        return Err(SettingsError::InvalidClassifierProfileName);
    }
    let maximum_input_characters = match raw.maximum_input_characters {
        Some(0) => return Err(SettingsError::InvalidLimit),
        Some(limit) => limit,
        None if require_limit => return Err(SettingsError::MissingClassifierProfileLimit),
        None => usize::MAX,
    };
    Ok(ClassifierProfile {
        name,
        endpoint: LocalModelEndpoint::new(&raw.base_url, raw.model)?,
        maximum_input_characters,
    })
}

fn validate_relative_path(path: &Path) -> Result<(), SettingsError> {
    let valid = !path.as_os_str().is_empty()
        && path.is_relative()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if !valid {
        return Err(SettingsError::UnsafeRelativePath(
            path.display().to_string(),
        ));
    }
    Ok(())
}

impl LocalModelEndpoint {
    /// Creates an endpoint after verifying that it uses an explicit loopback IP address.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError`] when the URL, scheme, host, credentials, or model name is
    /// invalid.
    pub fn new(base_url: &str, model: impl Into<String>) -> Result<Self, SettingsError> {
        let base_url = Url::parse(base_url)
            .map_err(|error| SettingsError::InvalidModelEndpoint(error.to_string()))?;

        if !matches!(base_url.scheme(), "http" | "https") {
            return Err(SettingsError::UnsupportedModelEndpointScheme(
                base_url.scheme().to_owned(),
            ));
        }

        if !base_url.username().is_empty() || base_url.password().is_some() {
            return Err(SettingsError::ModelEndpointContainsCredentials);
        }

        let is_loopback = match base_url.host() {
            Some(Host::Ipv4(address)) => address.is_loopback(),
            Some(Host::Ipv6(address)) => address.is_loopback(),
            Some(Host::Domain(_)) | None => false,
        };
        if !is_loopback {
            return Err(SettingsError::ModelEndpointNotLoopback(
                base_url.to_string(),
            ));
        }

        if base_url.query().is_some() || base_url.fragment().is_some() {
            return Err(SettingsError::ModelEndpointContainsQueryOrFragment);
        }

        let model = model.into();
        if model.trim().is_empty() || model.chars().any(char::is_control) {
            return Err(SettingsError::EmptyModelName);
        }

        Ok(Self { base_url, model })
    }

    /// Returns the validated base URL.
    #[must_use]
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Returns the configured model identifier.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }
}

/// Configuration validation failures.
#[derive(Debug, Error)]
pub enum SettingsError {
    /// The configuration file could not be read.
    #[error("failed to read configuration: {0}")]
    Read(#[source] std::io::Error),
    /// TOML parsing failed.
    #[error("invalid configuration: {0}")]
    InvalidConfiguration(String),
    /// A vault-relative path could escape its root.
    #[error("unsafe vault-relative path: {0}")]
    UnsafeRelativePath(String),
    /// A configured limit was zero.
    #[error("configured limits and intervals must be greater than zero")]
    InvalidLimit,
    /// A fallback classifier omitted its required input limit.
    #[error("fallback classifier profiles require maximum_input_characters")]
    MissingClassifierProfileLimit,
    /// A fallback classifier omitted its required stable name.
    #[error("fallback classifier profiles require an explicit name")]
    MissingClassifierProfileName,
    /// A classifier profile name was empty or unsafe.
    #[error("classifier profile name must be non-empty and contain no control characters")]
    InvalidClassifierProfileName,
    /// Classifier profile names were not unique.
    #[error("classifier profile names must be unique")]
    DuplicateClassifierProfile,
    /// A tool path was not explicit and absolute.
    #[error("tool path must be absolute: {0}")]
    UnsafeToolPath(String),
    /// The endpoint is not a valid URL.
    #[error("invalid model endpoint: {0}")]
    InvalidModelEndpoint(String),
    /// The endpoint does not use HTTP or HTTPS.
    #[error("unsupported model endpoint scheme: {0}")]
    UnsupportedModelEndpointScheme(String),
    /// The endpoint is not bound to an explicit loopback IP address.
    #[error("model endpoint must use an explicit loopback IP address: {0}")]
    ModelEndpointNotLoopback(String),
    /// Credentials were embedded in the endpoint URL.
    #[error("model endpoint URL must not contain credentials")]
    ModelEndpointContainsCredentials,
    /// Query strings and fragments are forbidden on model endpoints.
    #[error("model endpoint URL must not contain a query or fragment")]
    ModelEndpointContainsQueryOrFragment,
    /// The configured model name is empty.
    #[error("model name must not be empty")]
    EmptyModelName,
    /// A live test attempted to target a non-temporary vault.
    #[error("live integration tests require a temporary vault: {0}")]
    UnsafeLiveTestVault(String),
}

/// Verifies that a live integration test cannot target a non-temporary vault.
///
/// # Errors
///
/// Returns [`SettingsError`] when either root cannot be canonicalized or the vault is outside the
/// supplied temporary root.
pub fn validate_live_test_vault(
    vault_root: &Path,
    temporary_root: &Path,
) -> Result<(), SettingsError> {
    let vault_root = vault_root.canonicalize().map_err(SettingsError::Read)?;
    let temporary_root = temporary_root.canonicalize().map_err(SettingsError::Read)?;
    if !vault_root.starts_with(temporary_root) {
        return Err(SettingsError::UnsafeLiveTestVault(
            vault_root.display().to_string(),
        ));
    }
    Ok(())
}
