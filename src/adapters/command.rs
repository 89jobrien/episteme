//! Bounded shell-free process execution.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

/// Runs a configured executable with fixed argument arrays.
#[derive(Debug, Clone, Copy)]
pub struct CommandRunner {
    timeout: Duration,
    maximum_output_bytes: usize,
}

impl CommandRunner {
    /// Creates a bounded process runner.
    #[must_use]
    pub const fn new(timeout: Duration, maximum_output_bytes: usize) -> Self {
        Self {
            timeout,
            maximum_output_bytes,
        }
    }

    /// Runs one executable without a shell or inherited environment.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError`] for spawn, timeout, exit status, encoding, or output-limit
    /// failures.
    pub async fn run(
        &self,
        program: &Path,
        arguments: &[OsString],
    ) -> Result<String, CommandError> {
        let mut command = Command::new(program);
        command
            .args(arguments)
            .env_clear()
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(CommandError::Spawn)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CommandError::MissingPipe(program.display().to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CommandError::MissingPipe(program.display().to_string()))?;
        let operation = async {
            tokio::try_join!(
                read_bounded(stdout, self.maximum_output_bytes, program),
                read_bounded(stderr, self.maximum_output_bytes, program),
                async { child.wait().await.map_err(CommandError::Spawn) },
            )
        };
        let (stdout, _stderr, status) = tokio::time::timeout(self.timeout, operation)
            .await
            .map_err(|_| CommandError::Timeout(program.display().to_string()))??;
        if !status.success() {
            return Err(CommandError::Exit {
                program: program.display().to_string(),
                status: status.code(),
            });
        }
        String::from_utf8(stdout).map_err(CommandError::Encoding)
    }
}

async fn read_bounded(
    reader: impl AsyncRead + Unpin,
    maximum_output_bytes: usize,
    program: &Path,
) -> Result<Vec<u8>, CommandError> {
    let limit = u64::try_from(maximum_output_bytes)
        .map_err(|_| CommandError::OutputLimit(program.display().to_string()))?;
    let mut reader = reader.take(limit.saturating_add(1));
    let mut bytes = Vec::with_capacity(maximum_output_bytes.min(64 * 1024));
    reader
        .read_to_end(&mut bytes)
        .await
        .map_err(CommandError::Spawn)?;
    if bytes.len() > maximum_output_bytes {
        return Err(CommandError::OutputLimit(program.display().to_string()));
    }
    Ok(bytes)
}

/// External process execution failures.
#[derive(Debug, Error)]
pub enum CommandError {
    /// The executable could not be started or awaited.
    #[error("failed to run external tool: {0}")]
    Spawn(#[source] std::io::Error),
    /// The configured process timeout elapsed.
    #[error("external tool timed out: {0}")]
    Timeout(String),
    /// Output exceeded the configured byte limit.
    #[error("external tool exceeded output limit: {0}")]
    OutputLimit(String),
    /// A requested process output pipe was unavailable.
    #[error("external tool output pipe was unavailable: {0}")]
    MissingPipe(String),
    /// The process exited unsuccessfully.
    #[error("external tool {program} failed with status {status:?}")]
    Exit {
        /// Executable path.
        program: String,
        /// Numeric exit code when available.
        status: Option<i32>,
    },
    /// Standard output was not UTF-8.
    #[error("external tool output was not UTF-8: {0}")]
    Encoding(#[source] std::string::FromUtf8Error),
}
