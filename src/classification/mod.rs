//! Idempotent document classification orchestration.

mod batch;

pub use batch::{BatchOptions, ClassificationBatch};

/// Version of classification prompts, normalization, routing, and cache semantics.
pub const CLASSIFICATION_POLICY_VERSION: &str = "classification-v2";

use chrono::Utc;
use thiserror::Error;

use crate::config::ClassifierProfile;
use crate::domain::{
    AnalysisProvenance, ClassificationCacheKey, ClassificationFailure, ClassificationFailureCode,
    ClassificationOutcome, ClassificationRecord, ExtractedDocument, StagedSource,
};
use crate::ports::{
    ClassificationStore, DocumentClassifier, DocumentExtractor, ModelProbe, ProfileClassifier,
};

/// Routes classification through ordered, bounded local model profiles.
#[derive(Debug)]
pub struct RoutedDocumentClassifier<C, P> {
    profiles: Vec<ClassifierProfile>,
    classifier: C,
    probe: P,
    policy_version: String,
}

impl<C, P> RoutedDocumentClassifier<C, P> {
    /// Creates a router from ordered profiles and external ports.
    #[must_use]
    pub fn new(
        profiles: Vec<ClassifierProfile>,
        classifier: C,
        probe: P,
        policy_version: impl Into<String>,
    ) -> Self {
        Self {
            profiles,
            classifier,
            probe,
            policy_version: policy_version.into(),
        }
    }
}

impl<C, P> RoutedDocumentClassifier<C, P>
where
    C: ProfileClassifier,
    P: ModelProbe,
{
    async fn classify_profile(
        &self,
        document: &ExtractedDocument,
        profile: &ClassifierProfile,
        extracted_characters: u64,
    ) -> Result<ClassificationOutcome, ClassificationFailure> {
        let availability = self.probe.probe(profile).await.map_err(|failure| {
            failure.with_context(profile.endpoint.model(), extracted_characters)
        })?;
        if !availability
            .advertised_models
            .iter()
            .any(|model| model == profile.endpoint.model())
        {
            return Err(ClassificationFailure::retryable(
                ClassificationFailureCode::ModelUnavailable,
                "configured classifier model was not advertised",
            )
            .with_context(profile.endpoint.model(), extracted_characters));
        }
        let classification = self
            .classifier
            .classify_with_profile(document, profile)
            .await
            .map_err(|failure| {
                failure.with_context(profile.endpoint.model(), extracted_characters)
            })?
            .normalize()
            .map_err(|_| {
                ClassificationFailure::terminal(
                    ClassificationFailureCode::InvalidOutput,
                    "generated classification violated metadata policy",
                )
                .with_context(profile.endpoint.model(), extracted_characters)
            })?;
        Ok(ClassificationOutcome {
            classification,
            analysis: AnalysisProvenance {
                function: "ClassifyDocument".to_owned(),
                client: profile.name.clone(),
                model: profile.endpoint.model().to_owned(),
                pipeline_version: self.policy_version.clone(),
                processed_at: Utc::now().to_rfc3339(),
            },
            extracted_characters,
        })
    }
}

#[async_trait::async_trait]
impl<C, P> DocumentClassifier for RoutedDocumentClassifier<C, P>
where
    C: ProfileClassifier,
    P: ModelProbe,
{
    fn cache_keys(&self, extracted_characters: usize) -> Vec<ClassificationCacheKey> {
        let eligible = self
            .profiles
            .iter()
            .filter(|profile| extracted_characters <= profile.maximum_input_characters)
            .map(|profile| ClassificationCacheKey {
                model: profile.endpoint.model().to_owned(),
                policy_version: self.policy_version.clone(),
            })
            .collect::<Vec<_>>();
        if eligible.is_empty() {
            self.profiles
                .iter()
                .max_by_key(|profile| profile.maximum_input_characters)
                .map(|profile| {
                    vec![ClassificationCacheKey {
                        model: profile.endpoint.model().to_owned(),
                        policy_version: self.policy_version.clone(),
                    }]
                })
                .unwrap_or_default()
        } else {
            eligible
        }
    }

    async fn classify(
        &self,
        document: &ExtractedDocument,
    ) -> Result<ClassificationOutcome, ClassificationFailure> {
        let extracted_characters = document.text().chars().count();
        let extracted_characters_u64 = u64::try_from(extracted_characters).unwrap_or(u64::MAX);
        let mut eligible = false;
        let mut last_failure = None;
        for profile in &self.profiles {
            if extracted_characters > profile.maximum_input_characters {
                continue;
            }
            eligible = true;
            match self
                .classify_profile(document, profile, extracted_characters_u64)
                .await
            {
                Ok(outcome) => return Ok(outcome),
                Err(failure)
                    if failure.retryable
                        || failure.code == ClassificationFailureCode::InvalidOutput =>
                {
                    last_failure = Some(failure);
                }
                Err(failure) => return Err(failure),
            }
        }
        if !eligible {
            if let Some(profile) = self
                .profiles
                .iter()
                .max_by_key(|profile| profile.maximum_input_characters)
            {
                return self
                    .classify_profile(document, profile, extracted_characters_u64)
                    .await
                    .map_err(|failure| {
                        if failure.code == ClassificationFailureCode::ContextOverflow {
                            ClassificationFailure::terminal(failure.code, failure.message)
                                .with_context(profile.endpoint.model(), extracted_characters_u64)
                        } else {
                            failure
                        }
                    });
            }
            return Err(ClassificationFailure::terminal(
                ClassificationFailureCode::InputTooLarge,
                "no classifier profiles were configured",
            )
            .with_extracted_characters(extracted_characters_u64));
        }
        let failure = last_failure.unwrap_or_else(|| {
            ClassificationFailure::terminal(
                ClassificationFailureCode::ModelRequest,
                "all classifier profiles failed",
            )
        });
        if failure.code == ClassificationFailureCode::ContextOverflow {
            return Err(
                ClassificationFailure::terminal(failure.code, failure.message).with_context(
                    failure.model.unwrap_or_else(|| "unknown".to_owned()),
                    failure
                        .extracted_characters
                        .unwrap_or(extracted_characters_u64),
                ),
            );
        }
        Err(failure)
    }
}

/// Coordinates extraction, local classification, and derived-state persistence.
#[derive(Debug)]
pub struct ClassificationPipeline<E, C, S> {
    extractor: E,
    classifier: C,
    store: S,
}

/// Controls whether a matching versioned classification may be reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePolicy {
    /// Return the first matching cache entry in profile order.
    Use,
    /// Always invoke classification and persist a fresh version.
    Refresh,
}

/// Result details required by durable batch accounting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassificationExecution {
    /// Classification record returned or persisted.
    pub record: ClassificationRecord,
    /// Whether a matching versioned cache entry was reused.
    pub cache_hit: bool,
    /// Extracted Unicode scalar count.
    pub extracted_characters: u64,
}

/// Classifies one staged source for batch orchestration.
#[async_trait::async_trait]
pub trait ClassificationProcessor: Send + Sync {
    /// Processes one source with the requested cache policy.
    async fn process(
        &self,
        source: &StagedSource,
        cache_policy: CachePolicy,
    ) -> Result<ClassificationExecution, ClassificationFailure>;
}

impl<E, C, S> ClassificationPipeline<E, C, S> {
    /// Creates a classification pipeline from its external ports.
    #[must_use]
    pub const fn new(extractor: E, classifier: C, store: S) -> Self {
        Self {
            extractor,
            classifier,
            store,
        }
    }
}

impl<E, C, S> ClassificationPipeline<E, C, S>
where
    E: DocumentExtractor,
    C: DocumentClassifier,
    S: ClassificationStore,
{
    /// Classifies a staged source or returns its existing digest-matched record.
    ///
    /// # Errors
    ///
    /// Returns [`ClassificationError`] when extraction, inference, validation, or persistence
    /// fails.
    pub async fn classify(
        &self,
        source: &StagedSource,
        cache_policy: CachePolicy,
    ) -> Result<ClassificationRecord, ClassificationError> {
        self.classify_detailed(source, cache_policy)
            .await
            .map(|execution| execution.record)
    }

    /// Classifies one source and returns cache and extraction metrics.
    ///
    /// # Errors
    ///
    /// Returns [`ClassificationError`] when extraction, inference, validation, or persistence
    /// fails.
    pub async fn classify_detailed(
        &self,
        source: &StagedSource,
        cache_policy: CachePolicy,
    ) -> Result<ClassificationExecution, ClassificationError> {
        let extracted = self
            .extractor
            .extract(source)
            .await
            .map_err(ClassificationError::extract)?;
        if cache_policy == CachePolicy::Use {
            for key in self.classifier.cache_keys(extracted.text().chars().count()) {
                if let Some(record) = self
                    .store
                    .load(source.digest(), &key)
                    .map_err(ClassificationError::store)?
                {
                    return Ok(ClassificationExecution {
                        record,
                        cache_hit: true,
                        extracted_characters: u64::try_from(extracted.text().chars().count())
                            .unwrap_or(u64::MAX),
                    });
                }
            }
        }
        let outcome = self
            .classifier
            .classify(&extracted)
            .await
            .map_err(ClassificationError::Analysis)?;
        let record = ClassificationRecord {
            source_digest: source.digest().clone(),
            source_name: source.source_name().clone(),
            extraction_method: extracted.method(),
            classification: outcome.classification,
            analysis: outcome.analysis,
        };
        let key = ClassificationCacheKey {
            model: record.analysis.model.clone(),
            policy_version: record.analysis.pipeline_version.clone(),
        };
        self.store
            .save(&key, &record)
            .map_err(ClassificationError::store)?;
        Ok(ClassificationExecution {
            extracted_characters: outcome.extracted_characters,
            record,
            cache_hit: false,
        })
    }
}

#[async_trait::async_trait]
impl<E, C, S> ClassificationProcessor for ClassificationPipeline<E, C, S>
where
    E: DocumentExtractor,
    C: DocumentClassifier,
    S: ClassificationStore,
{
    async fn process(
        &self,
        source: &StagedSource,
        cache_policy: CachePolicy,
    ) -> Result<ClassificationExecution, ClassificationFailure> {
        self.classify_detailed(source, cache_policy)
            .await
            .map_err(Into::into)
    }
}

/// Standalone classification failures without private document content.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ClassificationError {
    /// Deterministic extraction failed.
    #[error("extraction failed: {0}")]
    Extraction(String),
    /// Local inference or generated metadata validation failed.
    #[error("classification failed: {0}")]
    Analysis(#[source] ClassificationFailure),
    /// Derived-state persistence failed.
    #[error("classification persistence failed: {0}")]
    Store(String),
    /// Requested concurrency was outside the supported range.
    #[error("classification concurrency must be between 1 and 5")]
    InvalidConcurrency,
}

impl ClassificationError {
    fn extract(_error: impl std::error::Error) -> Self {
        Self::Extraction("document extraction failed".to_owned())
    }

    fn store(_error: impl std::error::Error) -> Self {
        Self::Store("classification persistence failed".to_owned())
    }
}

impl From<ClassificationError> for ClassificationFailure {
    fn from(error: ClassificationError) -> Self {
        match error {
            ClassificationError::Extraction(message) => {
                Self::terminal(ClassificationFailureCode::Extraction, message)
            }
            ClassificationError::Analysis(failure) => failure,
            ClassificationError::Store(message) => {
                Self::terminal(ClassificationFailureCode::Persistence, message)
            }
            ClassificationError::InvalidConcurrency => Self::terminal(
                ClassificationFailureCode::Persistence,
                "classification concurrency was invalid",
            ),
        }
    }
}
