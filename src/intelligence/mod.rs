//! Evidence-grounded document intelligence orchestration.

use thiserror::Error;

use crate::classification::CachePolicy;
use crate::domain::{DocumentIntelligence, IntelligenceCacheKey, StagedSource};
use crate::ports::{
    DocumentClassifier, DocumentExtractor, DocumentIntelligenceAnalyzer, DocumentIntelligenceStore,
};

pub const INTELLIGENCE_POLICY_VERSION: &str = "intelligence-v1";

#[derive(Debug)]
pub struct DocumentIntelligencePipeline<E, C, A, S> {
    extractor: E,
    classifier: C,
    analyzer: A,
    store: S,
    model: String,
}

impl<E, C, A, S> DocumentIntelligencePipeline<E, C, A, S> {
    #[must_use]
    pub fn new(
        extractor: E,
        classifier: C,
        analyzer: A,
        store: S,
        model: impl Into<String>,
    ) -> Self {
        Self {
            extractor,
            classifier,
            analyzer,
            store,
            model: model.into(),
        }
    }
}

impl<E, C, A, S> DocumentIntelligencePipeline<E, C, A, S>
where
    E: DocumentExtractor,
    C: DocumentClassifier,
    A: DocumentIntelligenceAnalyzer,
    S: DocumentIntelligenceStore,
{
    /// Produces or reuses one evidence-grounded intelligence graph.
    ///
    /// # Errors
    ///
    /// Returns [`IntelligenceError`] when extraction, classification, analysis, or persistence
    /// fails.
    pub async fn analyze(
        &self,
        source: &StagedSource,
        cache_policy: CachePolicy,
    ) -> Result<DocumentIntelligence, IntelligenceError> {
        let key = IntelligenceCacheKey {
            model: self.model.clone(),
            policy_version: INTELLIGENCE_POLICY_VERSION.to_owned(),
        };
        if cache_policy == CachePolicy::Use
            && let Some(value) = self
                .store
                .load_intelligence(source.digest(), &key)
                .map_err(|_| IntelligenceError::Store)?
        {
            return Ok(value);
        }
        let document = self
            .extractor
            .extract(source)
            .await
            .map_err(|_| IntelligenceError::Extraction)?;
        let classification = self
            .classifier
            .classify(&document)
            .await
            .map_err(|_| IntelligenceError::Classification)?;
        let intelligence = self
            .analyzer
            .analyze_intelligence(&document, &classification.classification)
            .await
            .map_err(|_| IntelligenceError::Analysis)?;
        self.store
            .save_intelligence(&key, &intelligence)
            .map_err(|_| IntelligenceError::Store)?;
        Ok(intelligence)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IntelligenceError {
    #[error("document extraction failed")]
    Extraction,
    #[error("document classification failed")]
    Classification,
    #[error("document intelligence analysis failed")]
    Analysis,
    #[error("document intelligence persistence failed")]
    Store,
}
