//! Redirect-free readiness probes for validated local model endpoints.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::redirect::Policy;
use serde::Deserialize;

use crate::config::ClassifierProfile;
use crate::domain::{ClassificationFailure, ClassificationFailureCode, ModelAvailability};
use crate::ports::ModelProbe;

const MAX_MODELS_RESPONSE_BYTES: u64 = 1_048_576;

/// Probes OpenAI-compatible loopback model endpoints.
#[derive(Debug, Clone)]
pub struct HttpModelProbe {
    client: reqwest::Client,
}

impl HttpModelProbe {
    /// Creates a probe with redirects disabled and a bounded request timeout.
    ///
    /// # Errors
    ///
    /// Returns a Reqwest error when the client cannot be built.
    pub fn new(timeout: Duration) -> Result<Self, reqwest::Error> {
        reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(timeout)
            .build()
            .map(|client| Self { client })
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
    #[serde(default)]
    models: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

impl ModelEntry {
    fn identifier(self) -> Option<String> {
        self.id.or(self.name).or(self.model)
    }
}

#[async_trait]
impl ModelProbe for HttpModelProbe {
    async fn probe(
        &self,
        profile: &ClassifierProfile,
    ) -> Result<ModelAvailability, ClassificationFailure> {
        let endpoint = format!(
            "{}/models",
            profile.endpoint.base_url().as_str().trim_end_matches('/')
        );
        let mut response = self.client.get(endpoint).send().await.map_err(|_| {
            ClassificationFailure::retryable(
                ClassificationFailureCode::EndpointUnavailable,
                "local classifier endpoint probe failed",
            )
        })?;
        if response.status().is_redirection() {
            return Err(ClassificationFailure::terminal(
                ClassificationFailureCode::EndpointUnavailable,
                "local classifier endpoint attempted a redirect",
            ));
        }
        if response.status().is_server_error() {
            return Err(ClassificationFailure::retryable(
                ClassificationFailureCode::EndpointUnavailable,
                "local classifier endpoint returned a server error",
            ));
        }
        if !response.status().is_success() {
            return Err(ClassificationFailure::retryable(
                ClassificationFailureCode::ModelUnavailable,
                "local classifier model list was unavailable",
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_MODELS_RESPONSE_BYTES)
        {
            return Err(ClassificationFailure::terminal(
                ClassificationFailureCode::ModelUnavailable,
                "local classifier model list exceeded the response limit",
            ));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            ClassificationFailure::retryable(
                ClassificationFailureCode::ModelUnavailable,
                "local classifier model list could not be read",
            )
        })? {
            if body.len().saturating_add(chunk.len())
                > usize::try_from(MAX_MODELS_RESPONSE_BYTES).unwrap_or(usize::MAX)
            {
                return Err(ClassificationFailure::terminal(
                    ClassificationFailureCode::ModelUnavailable,
                    "local classifier model list exceeded the response limit",
                ));
            }
            body.extend_from_slice(&chunk);
        }
        let models = serde_json::from_slice::<ModelsResponse>(&body).map_err(|_| {
            ClassificationFailure::retryable(
                ClassificationFailureCode::ModelUnavailable,
                "local classifier model list was invalid",
            )
        })?;
        let advertised_models = models
            .data
            .into_iter()
            .chain(models.models)
            .filter_map(ModelEntry::identifier)
            .take(1_000)
            .collect();
        Ok(ModelAvailability { advertised_models })
    }
}
