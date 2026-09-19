//! Source validation and immutable content identification.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use thiserror::Error;

use crate::domain::{DocumentKind, SourceDigest, SourceFileName, StagedSource};

/// Validates and identifies one source under the configured inbox root.
///
/// # Errors
///
/// Returns [`StageError`] when the source is outside the inbox, is a symlink, exceeds the byte
/// limit, has an unsafe filename, or uses an unsupported extension.
pub fn stage_source(
    inbox_root: &Path,
    source: &Path,
    staging_root: &Path,
    maximum_source_bytes: u64,
) -> Result<StagedSource, StageError> {
    let link_metadata = fs::symlink_metadata(source).map_err(StageError::Filesystem)?;
    if link_metadata.file_type().is_symlink() || !link_metadata.is_file() {
        return Err(StageError::NotRegularFile);
    }
    if link_metadata.len() > maximum_source_bytes {
        return Err(StageError::SourceTooLarge {
            actual: link_metadata.len(),
            maximum: maximum_source_bytes,
        });
    }
    let inbox_root = inbox_root.canonicalize().map_err(StageError::Filesystem)?;
    let source = source.canonicalize().map_err(StageError::Filesystem)?;
    if !source.starts_with(&inbox_root) {
        return Err(StageError::OutsideInbox);
    }
    let source_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(StageError::InvalidFileName)
        .and_then(|name| SourceFileName::new(name).map_err(|_| StageError::InvalidFileName))?;
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or(StageError::UnsupportedDocument)?;
    let kind = match extension.as_str() {
        "pdf" => DocumentKind::Pdf,
        "html" | "htm" => DocumentKind::Html,
        "png" | "jpg" | "jpeg" | "tif" | "tiff" | "webp" => DocumentKind::Image,
        _ => return Err(StageError::UnsupportedDocument),
    };
    let bytes = fs::read(&source).map_err(StageError::Filesystem)?;
    let byte_count = u64::try_from(bytes.len()).map_err(|_| StageError::SourceTooLarge {
        actual: u64::MAX,
        maximum: maximum_source_bytes,
    })?;
    if byte_count != link_metadata.len() {
        return Err(StageError::SourceChanged);
    }
    let digest = SourceDigest::from_bytes(&bytes);
    fs::create_dir_all(staging_root).map_err(StageError::Filesystem)?;
    let staging_metadata = fs::symlink_metadata(staging_root).map_err(StageError::Filesystem)?;
    if staging_metadata.file_type().is_symlink() || !staging_metadata.is_dir() {
        return Err(StageError::UnsafeStagingDirectory);
    }
    let staged_path = staging_root.join(format!("{}-{source_name}", digest.as_str()));
    if staged_path.exists() {
        let staged_metadata = fs::symlink_metadata(&staged_path).map_err(StageError::Filesystem)?;
        if staged_metadata.file_type().is_symlink() || !staged_metadata.is_file() {
            return Err(StageError::UnsafeStagingDirectory);
        }
        let staged_bytes = fs::read(&staged_path).map_err(StageError::Filesystem)?;
        if SourceDigest::from_bytes(&staged_bytes) != digest {
            return Err(StageError::SourceChanged);
        }
    } else {
        let mut temporary =
            tempfile::NamedTempFile::new_in(staging_root).map_err(StageError::Filesystem)?;
        temporary
            .write_all(&bytes)
            .map_err(StageError::Filesystem)?;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o400))
            .map_err(StageError::Filesystem)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(StageError::Filesystem)?;
        temporary
            .persist_noclobber(&staged_path)
            .map_err(|error| StageError::Filesystem(error.error))?;
    }
    Ok(StagedSource::with_original(
        staged_path,
        source,
        digest,
        source_name,
        kind,
    ))
}

/// Source staging failures.
#[derive(Debug, Error)]
pub enum StageError {
    /// A filesystem operation failed.
    #[error("source staging filesystem operation failed: {0}")]
    Filesystem(#[source] std::io::Error),
    /// The path was not a regular non-symlink file.
    #[error("source must be a regular non-symlink file")]
    NotRegularFile,
    /// The canonical source escaped the configured inbox.
    #[error("source is outside the configured inbox")]
    OutsideInbox,
    /// The source filename was not safe UTF-8.
    #[error("source filename is invalid")]
    InvalidFileName,
    /// The source extension is unsupported.
    #[error("unsupported document format")]
    UnsupportedDocument,
    /// The source exceeded the configured size limit.
    #[error("source size {actual} exceeds configured maximum {maximum}")]
    SourceTooLarge {
        /// Observed bytes.
        actual: u64,
        /// Configured byte limit.
        maximum: u64,
    },
    /// The source changed while being staged.
    #[error("source changed while being staged")]
    SourceChanged,
    /// The private staging directory or an existing staged source was unsafe.
    #[error("private staging directory is unsafe")]
    UnsafeStagingDirectory,
}
