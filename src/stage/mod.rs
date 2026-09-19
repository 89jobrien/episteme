//! Source validation and immutable content identification.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path};

use rustix::fd::OwnedFd;
use rustix::fs::{Mode, OFlags, open, openat};
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
    let relative_source = source
        .strip_prefix(inbox_root)
        .map_err(|_| StageError::OutsideInbox)?;
    if relative_source.as_os_str().is_empty()
        || relative_source
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(StageError::OutsideInbox);
    }
    let inbox_root = inbox_root.canonicalize().map_err(StageError::Filesystem)?;
    let source_name = relative_source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(StageError::InvalidFileName)
        .and_then(|name| SourceFileName::new(name).map_err(|_| StageError::InvalidFileName))?;
    let extension = relative_source
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
    let bytes = read_source_beneath(&inbox_root, relative_source, maximum_source_bytes)?;
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
        let (staged_bytes, _) = read_source_nofollow(&staged_path, maximum_source_bytes)?;
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
        inbox_root.join(relative_source),
        digest,
        source_name,
        kind,
    ))
}

fn read_source_beneath(
    inbox_root: &Path,
    relative_source: &Path,
    maximum_source_bytes: u64,
) -> Result<Vec<u8>, StageError> {
    let mut directory = open(
        inbox_root,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::DIRECTORY,
        Mode::empty(),
    )
    .map_err(stage_open_error)?;
    let mut components = relative_source.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(StageError::OutsideInbox);
        };
        if components.peek().is_some() {
            directory = openat(
                &directory,
                name,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::DIRECTORY,
                Mode::empty(),
            )
            .map_err(stage_open_error)?;
        } else {
            let descriptor = openat(
                &directory,
                name,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .map_err(stage_open_error)?;
            return read_descriptor(descriptor, maximum_source_bytes).map(|(bytes, _)| bytes);
        }
    }
    Err(StageError::NotRegularFile)
}

fn read_source_nofollow(
    source: &Path,
    maximum_source_bytes: u64,
) -> Result<(Vec<u8>, u64), StageError> {
    let descriptor = open(
        source,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(stage_open_error)?;
    read_descriptor(descriptor, maximum_source_bytes)
}

fn read_descriptor(
    descriptor: OwnedFd,
    maximum_source_bytes: u64,
) -> Result<(Vec<u8>, u64), StageError> {
    let file = File::from(descriptor);
    let metadata_before = file.metadata().map_err(StageError::Filesystem)?;
    let metadata = &metadata_before;
    if !metadata.is_file() {
        return Err(StageError::NotRegularFile);
    }
    if metadata.len() > maximum_source_bytes {
        return Err(StageError::SourceTooLarge {
            actual: metadata.len(),
            maximum: maximum_source_bytes,
        });
    }
    let mut bytes = Vec::new();
    let mut reader = file.take(maximum_source_bytes.saturating_add(1));
    reader
        .read_to_end(&mut bytes)
        .map_err(StageError::Filesystem)?;
    let byte_count = u64::try_from(bytes.len()).map_err(|_| StageError::SourceTooLarge {
        actual: u64::MAX,
        maximum: maximum_source_bytes,
    })?;
    if byte_count > maximum_source_bytes {
        return Err(StageError::SourceTooLarge {
            actual: byte_count,
            maximum: maximum_source_bytes,
        });
    }
    if byte_count != metadata.len() {
        return Err(StageError::SourceChanged);
    }
    let metadata_after = reader
        .get_ref()
        .metadata()
        .map_err(StageError::Filesystem)?;
    if FileIdentity::from(&metadata_before) != FileIdentity::from(&metadata_after) {
        return Err(StageError::SourceChanged);
    }
    Ok((bytes, byte_count))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    length: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl From<&fs::Metadata> for FileIdentity {
    fn from(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            length: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }
}

fn stage_open_error(error: rustix::io::Errno) -> StageError {
    if matches!(error, rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR) {
        StageError::NotRegularFile
    } else {
        StageError::Filesystem(error.into())
    }
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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::{FileIdentity, StageError, read_source_nofollow};

    #[test]
    fn source_open_rejects_symlink_substitution() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let target = directory.path().join("target.html");
        let link = directory.path().join("source.html");
        std::fs::write(&target, "private source")?;
        symlink(&target, &link)?;

        assert!(matches!(
            read_source_nofollow(&link, 1024),
            Err(StageError::NotRegularFile)
        ));
        Ok(())
    }

    #[test]
    fn file_identity_detects_ctime_changes() {
        let before = FileIdentity {
            device: 1,
            inode: 2,
            length: 3,
            modified_seconds: 4,
            modified_nanoseconds: 5,
            changed_seconds: 6,
            changed_nanoseconds: 7,
        };
        let after = FileIdentity {
            changed_nanoseconds: 8,
            ..before
        };

        assert_ne!(before, after);
    }
}
