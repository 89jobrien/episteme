//! Resumable bounded batch classification orchestration.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Component, Path};
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use tokio::task::JoinSet;

use super::{CachePolicy, ClassificationError, ClassificationExecution, ClassificationProcessor};
use crate::domain::{
    ClassificationAttemptRecord, ClassificationAttemptStatus, ClassificationBatchSource,
    ClassificationBatchSummary, ClassificationFailure, ClassificationFailureCode, StagedSource,
};
use crate::ports::ClassificationRunStore;

/// Batch controls for cache and retry behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchOptions {
    /// Whether matching versioned records may be reused.
    pub cache_policy: CachePolicy,
    /// Whether sources whose latest attempt is retryable should run again.
    pub retry_failed: bool,
}

/// Runs source classifications concurrently while checkpointing every attempt.
#[derive(Debug)]
pub struct ClassificationBatch<P, S> {
    processor: Arc<P>,
    store: Arc<S>,
    maximum_concurrency: usize,
}

#[derive(Debug)]
struct PendingSource {
    relative_path: String,
    source: StagedSource,
    attempt: u32,
}

impl<P, S> ClassificationBatch<P, S>
where
    P: ClassificationProcessor + 'static,
    S: ClassificationRunStore + 'static,
{
    /// Creates a batch runner with concurrency bounded to one through five workers.
    ///
    /// # Errors
    ///
    /// Returns [`ClassificationError::InvalidConcurrency`] outside the supported range.
    pub fn new(
        processor: Arc<P>,
        store: Arc<S>,
        maximum_concurrency: usize,
    ) -> Result<Self, ClassificationError> {
        if !(1..=5).contains(&maximum_concurrency) {
            return Err(ClassificationError::InvalidConcurrency);
        }
        Ok(Self {
            processor,
            store,
            maximum_concurrency,
        })
    }

    /// Runs or resumes one durable batch and returns latest status per source.
    ///
    /// # Errors
    ///
    /// Returns [`ClassificationError`] when batch persistence or worker coordination fails.
    pub async fn run(
        &self,
        batch_id: &str,
        mut sources: Vec<ClassificationBatchSource>,
        options: BatchOptions,
    ) -> Result<ClassificationBatchSummary, ClassificationError> {
        if batch_id.trim().is_empty() || batch_id.chars().any(char::is_control) {
            return Err(ClassificationError::Store(
                "classification batch id was invalid".to_owned(),
            ));
        }
        sources.sort_by(|left, right| source_path(left).cmp(source_path(right)));
        let mut current_paths = HashSet::new();
        for source in &sources {
            let relative_path = source_path(source);
            validate_relative_path(relative_path)?;
            if !current_paths.insert(relative_path.to_owned()) {
                return Err(ClassificationError::Store(
                    "classification batch contained duplicate source paths".to_owned(),
                ));
            }
        }
        self.store
            .begin_batch(batch_id, &Utc::now().to_rfc3339())
            .map_err(|error| ClassificationError::Store(error.to_string()))?;
        let latest = self
            .store
            .latest_attempts(batch_id)
            .map_err(|error| ClassificationError::Store(error.to_string()))?;
        let previous = latest
            .into_iter()
            .map(|attempt| (attempt.source_path.clone(), attempt))
            .collect::<HashMap<_, _>>();
        let mut pending = VecDeque::new();
        for source in sources {
            let relative_path = source_path(&source).to_owned();
            let prior = previous.get(&relative_path);
            if !should_process(prior, &source, options) {
                continue;
            }
            let attempt = prior.map_or(1, |record| record.attempt.saturating_add(1));
            match source {
                ClassificationBatchSource::Ready { source, .. } => {
                    pending.push_back(PendingSource {
                        relative_path,
                        source,
                        attempt,
                    });
                }
                ClassificationBatchSource::Rejected { failure, .. } => {
                    self.store_attempt(&failure_attempt(
                        batch_id,
                        relative_path,
                        attempt,
                        failure,
                    ))?;
                }
            }
        }

        let mut workers = JoinSet::new();
        for _ in 0..self.maximum_concurrency {
            if let Some(item) = pending.pop_front() {
                self.spawn(batch_id, item, options.cache_policy, &mut workers)?;
            }
        }
        while let Some(result) = workers.join_next().await {
            let attempt = result.map_err(|_| {
                ClassificationError::Analysis(ClassificationFailure::terminal(
                    ClassificationFailureCode::ModelRequest,
                    "classification worker task failed",
                ))
            })?;
            self.store_attempt(&attempt)?;
            if let Some(item) = pending.pop_front() {
                self.spawn(batch_id, item, options.cache_policy, &mut workers)?;
            }
        }

        let attempts = self
            .store
            .latest_attempts(batch_id)
            .map_err(|error| ClassificationError::Store(error.to_string()))?
            .into_iter()
            .filter(|attempt| current_paths.contains(&attempt.source_path))
            .collect();
        let summary = summarize(batch_id, attempts);
        self.store
            .finish_batch(&summary)
            .map_err(|error| ClassificationError::Store(error.to_string()))?;
        Ok(summary)
    }

    fn spawn(
        &self,
        batch_id: &str,
        item: PendingSource,
        cache_policy: CachePolicy,
        workers: &mut JoinSet<ClassificationAttemptRecord>,
    ) -> Result<(), ClassificationError> {
        self.store_attempt(&ClassificationAttemptRecord {
            batch_id: batch_id.to_owned(),
            source_path: item.relative_path.clone(),
            source_digest: Some(item.source.digest().clone()),
            attempt: item.attempt,
            status: ClassificationAttemptStatus::Running,
            model: None,
            extracted_characters: None,
            duration_ms: 0,
            failure_code: None,
            failure_message: None,
            updated_at: Utc::now().to_rfc3339(),
        })?;
        let processor = Arc::clone(&self.processor);
        let batch_id = batch_id.to_owned();
        workers.spawn(async move {
            let started = Instant::now();
            let result = processor.process(&item.source, cache_policy).await;
            result_attempt(batch_id, item, result, started)
        });
        Ok(())
    }

    fn store_attempt(
        &self,
        attempt: &ClassificationAttemptRecord,
    ) -> Result<(), ClassificationError> {
        self.store
            .save_attempt(attempt)
            .map_err(|error| ClassificationError::Store(error.to_string()))
    }
}

fn source_path(source: &ClassificationBatchSource) -> &str {
    match source {
        ClassificationBatchSource::Ready { relative_path, .. }
        | ClassificationBatchSource::Rejected { relative_path, .. } => relative_path,
    }
}

fn validate_relative_path(path: &str) -> Result<(), ClassificationError> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ClassificationError::Store(
            "batch source path was not a safe relative path".to_owned(),
        ));
    }
    Ok(())
}

fn should_process(
    prior: Option<&ClassificationAttemptRecord>,
    source: &ClassificationBatchSource,
    options: BatchOptions,
) -> bool {
    if options.cache_policy == CachePolicy::Refresh {
        return true;
    }
    let current_digest = match source {
        ClassificationBatchSource::Ready { source, .. } => Some(source.digest()),
        ClassificationBatchSource::Rejected { .. } => None,
    };
    if prior.is_some_and(|attempt| attempt.source_digest.as_ref() != current_digest) {
        return true;
    }
    match prior.map(|attempt| attempt.status) {
        None
        | Some(ClassificationAttemptStatus::Pending | ClassificationAttemptStatus::Running) => true,
        Some(ClassificationAttemptStatus::FailedRetryable) => options.retry_failed,
        Some(
            ClassificationAttemptStatus::Succeeded
            | ClassificationAttemptStatus::Cached
            | ClassificationAttemptStatus::FailedTerminal,
        ) => false,
    }
}

fn failure_attempt(
    batch_id: &str,
    source_path: String,
    attempt: u32,
    failure: ClassificationFailure,
) -> ClassificationAttemptRecord {
    ClassificationAttemptRecord {
        batch_id: batch_id.to_owned(),
        source_path,
        source_digest: None,
        attempt,
        status: if failure.retryable {
            ClassificationAttemptStatus::FailedRetryable
        } else {
            ClassificationAttemptStatus::FailedTerminal
        },
        extracted_characters: failure.extracted_characters,
        duration_ms: 0,
        failure_code: Some(failure.code),
        model: failure.model,
        failure_message: Some(failure.message),
        updated_at: Utc::now().to_rfc3339(),
    }
}

fn result_attempt(
    batch_id: String,
    item: PendingSource,
    result: Result<ClassificationExecution, ClassificationFailure>,
    started: Instant,
) -> ClassificationAttemptRecord {
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match result {
        Ok(execution) => ClassificationAttemptRecord {
            batch_id,
            source_path: item.relative_path,
            source_digest: Some(item.source.digest().clone()),
            attempt: item.attempt,
            status: if execution.cache_hit {
                ClassificationAttemptStatus::Cached
            } else {
                ClassificationAttemptStatus::Succeeded
            },
            model: Some(execution.record.analysis.model),
            extracted_characters: Some(execution.extracted_characters),
            duration_ms,
            failure_code: None,
            failure_message: None,
            updated_at: Utc::now().to_rfc3339(),
        },
        Err(failure) => ClassificationAttemptRecord {
            batch_id,
            source_path: item.relative_path,
            source_digest: Some(item.source.digest().clone()),
            attempt: item.attempt,
            status: if failure.retryable {
                ClassificationAttemptStatus::FailedRetryable
            } else {
                ClassificationAttemptStatus::FailedTerminal
            },
            model: failure.model,
            extracted_characters: failure.extracted_characters,
            duration_ms,
            failure_code: Some(failure.code),
            failure_message: Some(failure.message),
            updated_at: Utc::now().to_rfc3339(),
        },
    }
}

fn summarize(
    batch_id: &str,
    attempts: Vec<ClassificationAttemptRecord>,
) -> ClassificationBatchSummary {
    ClassificationBatchSummary {
        batch_id: batch_id.to_owned(),
        total: attempts.len(),
        succeeded: attempts
            .iter()
            .filter(|attempt| attempt.status == ClassificationAttemptStatus::Succeeded)
            .count(),
        cached: attempts
            .iter()
            .filter(|attempt| attempt.status == ClassificationAttemptStatus::Cached)
            .count(),
        failed_retryable: attempts
            .iter()
            .filter(|attempt| attempt.status == ClassificationAttemptStatus::FailedRetryable)
            .count(),
        failed_terminal: attempts
            .iter()
            .filter(|attempt| attempt.status == ClassificationAttemptStatus::FailedTerminal)
            .count(),
        attempts,
    }
}
