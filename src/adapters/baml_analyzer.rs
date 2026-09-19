//! Typed BAML adapter for local document classification and distillation.

use std::time::Duration;

use async_trait::async_trait;

use crate::baml_client::B;
use crate::config::LocalModelEndpoint;
use crate::domain::{EvidenceReference, ExtractedDocument, ResearchDraft};
use crate::ports::{AnalysisError, ResearchAnalyzer};

/// Runs Episteme's typed BAML functions against configured local clients.
#[derive(Debug, Clone)]
pub struct BamlResearchAnalyzer {
    classifier: LocalModelEndpoint,
    distiller: LocalModelEndpoint,
    timeout: Duration,
}

impl BamlResearchAnalyzer {
    /// Creates an analyzer using validated local endpoints.
    #[must_use]
    pub const fn new(
        classifier: LocalModelEndpoint,
        distiller: LocalModelEndpoint,
        timeout: Duration,
    ) -> Self {
        Self {
            classifier,
            distiller,
            timeout,
        }
    }
}

#[async_trait]
impl ResearchAnalyzer for BamlResearchAnalyzer {
    async fn analyze(&self, document: &ExtractedDocument) -> Result<ResearchDraft, AnalysisError> {
        let classification = B
            .ClassifyDocument
            .with_env_var(
                "EPISTEME_CLASSIFY_BASE_URL",
                self.classifier.base_url().as_str(),
            )
            .with_env_var("EPISTEME_CLASSIFY_MODEL", self.classifier.model())
            .with_cancellation_token(Some(baml::CancellationToken::new_with_timeout(
                self.timeout,
            )))
            .call(document.text())
            .await
            .map_err(|_| AnalysisError::Model("classifier request failed".to_owned()))?;
        let output = B
            .DistillResearch
            .with_env_var(
                "EPISTEME_DISTILL_BASE_URL",
                self.distiller.base_url().as_str(),
            )
            .with_env_var("EPISTEME_DISTILL_MODEL", self.distiller.model())
            .with_cancellation_token(Some(baml::CancellationToken::new_with_timeout(
                self.timeout,
            )))
            .call(document.text(), &classification)
            .await
            .map_err(|_| AnalysisError::Model("distiller request failed".to_owned()))?;
        ResearchDraft {
            title: output.title,
            citation: output.citation,
            topics: output.topics,
            summary: output.summary,
            key_ideas: output.key_ideas,
            implementation_notes: output.implementation_notes,
            critique: output.critique,
            evidence: output
                .evidence
                .into_iter()
                .map(|reference| EvidenceReference {
                    quote: reference.quote,
                    location: reference.location,
                })
                .collect(),
        }
        .validate_against(document.text())
        .map_err(|error| AnalysisError::Invalid(error.to_string()))
    }
}
