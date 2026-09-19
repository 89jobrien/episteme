//! Local zk index refresh adapter.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;

use super::command::CommandRunner;
use crate::ports::{IndexError, VaultIndexer};

/// Refreshes the configured vault through the zk CLI.
#[derive(Debug, Clone)]
pub struct ZkIndexer {
    executable: PathBuf,
    vault_root: PathBuf,
    runner: CommandRunner,
}

impl ZkIndexer {
    /// Creates a bounded zk index adapter.
    #[must_use]
    pub const fn new(
        executable: PathBuf,
        vault_root: PathBuf,
        timeout: Duration,
        maximum_output_bytes: usize,
    ) -> Self {
        Self {
            executable,
            vault_root,
            runner: CommandRunner::new(timeout, maximum_output_bytes),
        }
    }
}

#[async_trait]
impl VaultIndexer for ZkIndexer {
    async fn index(&self) -> Result<(), IndexError> {
        self.runner
            .run(
                &self.executable,
                &[
                    OsString::from("--notebook-dir"),
                    self.vault_root.as_os_str().to_owned(),
                    OsString::from("index"),
                ],
            )
            .await
            .map(|_| ())
            .map_err(|error| IndexError(error.to_string()))
    }
}
