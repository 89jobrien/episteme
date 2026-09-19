//! Atomic filesystem persistence for Obsidian research notes.

use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use tempfile::NamedTempFile;

use crate::domain::{ArchivedSource, ResearchNote, SourceDigest, StagedSource, StoredNote};
use crate::ports::{VaultError, VaultStore};

/// Creates one vault-relative directory without traversing symlink components.
///
/// # Errors
///
/// Returns [`VaultError`] when the directory is unsafe or cannot be created.
pub fn prepare_vault_directory(root: &Path, relative: &Path) -> Result<PathBuf, VaultError> {
    ensure_safe_directory(root, relative)
}

/// Writes research notes beneath a configured vault-relative directory.
#[derive(Debug, Clone)]
pub struct VaultFileStore {
    vault_root: PathBuf,
    research_directory: PathBuf,
    archive_directory: PathBuf,
}

impl VaultFileStore {
    /// Creates a vault adapter rooted at trusted directories.
    #[must_use]
    pub fn new(
        vault_root: PathBuf,
        research_directory: PathBuf,
        archive_directory: PathBuf,
    ) -> Self {
        Self {
            vault_root,
            research_directory,
            archive_directory,
        }
    }

    fn destination(&self, note: &ResearchNote) -> PathBuf {
        let slug = slugify(&note.draft.title);
        let digest_prefix = &note.source_digest.as_str()[..12];
        self.research_directory
            .join(format!("{slug}-{digest_prefix}.md"))
    }

    /// Creates configured vault directories without traversing symlink components.
    ///
    /// # Errors
    ///
    /// Returns [`VaultError`] when a directory escapes the vault, traverses a symlink, or cannot
    /// be created.
    pub fn prepare_directories(&self) -> Result<(), VaultError> {
        ensure_safe_directory(&self.vault_root, &self.research_directory)?;
        ensure_safe_directory(&self.vault_root, &self.archive_directory)?;
        Ok(())
    }
}

impl VaultStore for VaultFileStore {
    fn find_by_digest(&self, digest: &SourceDigest) -> Result<Option<StoredNote>, VaultError> {
        let directory = ensure_safe_directory(&self.vault_root, &self.research_directory)?;
        if !directory.exists() {
            return Ok(None);
        }
        let needle = format!("source_digest: {digest}");
        for entry in fs::read_dir(&directory).map_err(VaultError::Filesystem)? {
            let entry = entry.map_err(VaultError::Filesystem)?;
            if !entry.file_type().map_err(VaultError::Filesystem)?.is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
                continue;
            }
            let content = fs::read_to_string(&path).map_err(VaultError::Filesystem)?;
            if content.lines().any(|line| line == needle) {
                let relative_path = path
                    .strip_prefix(&self.vault_root)
                    .map_err(|error| VaultError::Rendering(error.to_string()))?
                    .to_path_buf();
                return Ok(Some(StoredNote { relative_path }));
            }
        }
        Ok(None)
    }

    fn create_note(&self, note: &ResearchNote) -> Result<StoredNote, VaultError> {
        let relative_path = self.destination(note);
        let parent = ensure_safe_directory(&self.vault_root, &self.research_directory)?;
        let destination = self.vault_root.join(&relative_path);
        if destination.exists() {
            return Err(VaultError::AlreadyExists(
                relative_path.display().to_string(),
            ));
        }

        let rendered = render_note(note)?;
        let mut temporary = NamedTempFile::new_in(parent).map_err(VaultError::Filesystem)?;
        temporary
            .write_all(rendered.as_bytes())
            .map_err(VaultError::Filesystem)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(VaultError::Filesystem)?;
        temporary.persist_noclobber(&destination).map_err(|error| {
            if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                VaultError::AlreadyExists(relative_path.display().to_string())
            } else {
                VaultError::Filesystem(error.error)
            }
        })?;

        Ok(StoredNote { relative_path })
    }

    fn archive_source(&self, source: &StagedSource) -> Result<ArchivedSource, VaultError> {
        let relative_path = self.archive_directory.join(format!(
            "{}-{}",
            &source.digest().as_str()[..12],
            source.source_name()
        ));
        let parent = ensure_safe_directory(&self.vault_root, &self.archive_directory)?;
        let destination = self.vault_root.join(&relative_path);

        if destination.exists() {
            let metadata = fs::symlink_metadata(&destination).map_err(VaultError::Filesystem)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(VaultError::UnsafePath(destination.display().to_string()));
            }
            let archived_bytes = fs::read(&destination).map_err(VaultError::Filesystem)?;
            if crate::domain::SourceDigest::from_bytes(&archived_bytes) != *source.digest() {
                return Err(VaultError::DigestMismatch);
            }
            if source.path() != source.original_path() && source.path().exists() {
                fs::remove_file(source.path()).map_err(VaultError::Filesystem)?;
            }
            return Ok(ArchivedSource { relative_path });
        }

        let source_bytes = fs::read(source.path()).map_err(VaultError::Filesystem)?;
        if &crate::domain::SourceDigest::from_bytes(&source_bytes) != source.digest() {
            return Err(VaultError::DigestMismatch);
        }
        if source.original_path().exists() {
            let original_bytes =
                fs::read(source.original_path()).map_err(VaultError::Filesystem)?;
            if crate::domain::SourceDigest::from_bytes(&original_bytes) != *source.digest() {
                return Err(VaultError::DigestMismatch);
            }
        }

        let mut temporary = NamedTempFile::new_in(parent).map_err(VaultError::Filesystem)?;
        temporary
            .write_all(&source_bytes)
            .map_err(VaultError::Filesystem)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(VaultError::Filesystem)?;
        temporary.persist_noclobber(&destination).map_err(|error| {
            if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                VaultError::AlreadyExists(relative_path.display().to_string())
            } else {
                VaultError::Filesystem(error.error)
            }
        })?;
        if source.path() != source.original_path() && source.path().exists() {
            fs::remove_file(source.path()).map_err(VaultError::Filesystem)?;
        }
        Ok(ArchivedSource { relative_path })
    }
}

fn ensure_safe_directory(root: &Path, relative: &Path) -> Result<PathBuf, VaultError> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(VaultError::UnsafePath(relative.display().to_string()));
    }
    let root = root.canonicalize().map_err(VaultError::Filesystem)?;
    let mut current = root.clone();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(VaultError::UnsafePath(relative.display().to_string()));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(VaultError::UnsafePath(current.display().to_string()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(VaultError::Filesystem)?;
            }
            Err(error) => return Err(VaultError::Filesystem(error)),
        }
    }
    let canonical = current.canonicalize().map_err(VaultError::Filesystem)?;
    if !canonical.starts_with(root) {
        return Err(VaultError::UnsafePath(canonical.display().to_string()));
    }
    Ok(canonical)
}

fn slugify(title: &str) -> String {
    let mut slug = String::new();
    let mut separator_pending = false;
    for character in title.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            if separator_pending && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(character);
            separator_pending = false;
        } else {
            separator_pending = true;
        }
    }
    if slug.is_empty() {
        "research".to_owned()
    } else {
        slug
    }
}

fn render_note(note: &ResearchNote) -> Result<String, VaultError> {
    let citation = yaml_value(&note.draft.citation)?;
    let source_file = yaml_value(note.source_name.as_str())?;
    let topics = yaml_value(&note.draft.topics)?;
    let baml_function = yaml_value(&note.analysis.function)?;
    let baml_client = yaml_value(&note.analysis.client)?;
    let baml_model = yaml_value(&note.analysis.model)?;
    let pipeline_version = yaml_value(&note.analysis.pipeline_version)?;
    let processed_at = yaml_value(&note.analysis.processed_at)?;
    let key_ideas = markdown_list(&note.draft.key_ideas);
    let implementation_notes = markdown_list(&note.draft.implementation_notes);
    let evidence = note
        .draft
        .evidence
        .iter()
        .map(|reference| {
            format!(
                "- {} — {}",
                inert_markdown(&reference.quote),
                inert_markdown(&reference.location)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    Ok(format!(
        "---\n\
         type: research\n\
         source_type: document\n\
         citation: {citation}\n\
         topic: {topics}\n\
         status: unprocessed\n\
         tags: [research, ingested]\n\
         source_digest: {}\n\
         source_file: {source_file}\n\
         extraction_method: {}\n\
         baml_function: {baml_function}\n\
         baml_client: {baml_client}\n\
         baml_model: {baml_model}\n\
         pipeline_version: {pipeline_version}\n\
         processed: {processed_at}\n\
         ---\n\n\
         # {}\n\n\
         ## Summary\n\n{}\n\n\
         ## Key Ideas\n\n{key_ideas}\n\n\
         ## Implementation Notes\n\n{implementation_notes}\n\n\
         ## Critique\n\n{}\n\n\
         ## Evidence\n\n{evidence}\n",
        note.source_digest,
        note.extraction_method.as_str(),
        inert_markdown(&note.draft.title),
        inert_markdown(&note.draft.summary),
        inert_markdown(&note.draft.critique),
    ))
}

fn yaml_value(value: &(impl serde::Serialize + ?Sized)) -> Result<String, VaultError> {
    serde_json::to_string(value).map_err(|error| VaultError::Rendering(error.to_string()))
}

fn inert_markdown(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\0' => escaped.push('�'),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '(' | ')' | '#' | '+' | '-' | '.'
            | '!' | '|' => {
                escaped.push('\\');
                escaped.push(character);
            }
            _ => escaped.push(character),
        }
    }
    escaped
}

fn markdown_list(items: &[String]) -> String {
    if items.is_empty() {
        return "- None noted".to_owned();
    }
    items
        .iter()
        .map(|item| format!("- {}", inert_markdown(item)))
        .collect::<Vec<_>>()
        .join("\n")
}
