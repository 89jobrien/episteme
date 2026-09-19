//! Stable-file filtering for inbox watcher events.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use thiserror::Error;

/// Result of evaluating one filesystem event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchDecision {
    /// The path is outside policy or not a regular file.
    Ignore,
    /// Another unchanged observation is required.
    Pending,
    /// The file is stable and safe to enqueue.
    Ready(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileFingerprint {
    length: u64,
    modified: SystemTime,
}

/// Tracks repeated inbox observations until a file is unchanged.
#[derive(Debug)]
pub struct StableFileTracker {
    root: PathBuf,
    observations: HashMap<PathBuf, FileFingerprint>,
}

impl StableFileTracker {
    /// Creates a tracker anchored to a canonical inbox root.
    ///
    /// # Errors
    ///
    /// Returns [`WatchError`] when the inbox root cannot be canonicalized.
    pub fn new(root: &Path) -> Result<Self, WatchError> {
        let root = root.canonicalize().map_err(WatchError::Filesystem)?;
        Ok(Self {
            root,
            observations: HashMap::new(),
        })
    }

    /// Records one observation and returns whether the path is stable.
    #[must_use]
    pub fn observe(&mut self, path: &Path) -> WatchDecision {
        let Ok(link_metadata) = fs::symlink_metadata(path) else {
            return WatchDecision::Ignore;
        };
        if link_metadata.file_type().is_symlink() || !link_metadata.is_file() {
            return WatchDecision::Ignore;
        }
        let Ok(path) = path.canonicalize() else {
            return WatchDecision::Ignore;
        };
        if !path.starts_with(&self.root) {
            return WatchDecision::Ignore;
        }
        let Ok(metadata) = fs::metadata(&path) else {
            return WatchDecision::Ignore;
        };
        let Ok(modified) = metadata.modified() else {
            return WatchDecision::Ignore;
        };
        let fingerprint = FileFingerprint {
            length: metadata.len(),
            modified,
        };

        if self.observations.get(&path) == Some(&fingerprint) {
            self.observations.remove(&path);
            WatchDecision::Ready(path)
        } else {
            self.observations.insert(path, fingerprint);
            WatchDecision::Pending
        }
    }
}

/// Watcher initialization failures.
#[derive(Debug, Error)]
pub enum WatchError {
    /// A filesystem operation failed.
    #[error("watcher filesystem operation failed: {0}")]
    Filesystem(#[source] std::io::Error),
}
