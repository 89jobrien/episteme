//! Shell-free adapters for Poppler, Tesseract, and Pandoc.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;

use super::command::CommandRunner;
use crate::config::ToolPaths;
use crate::ports::{DocumentTools, ExtractionError};

/// Executes configured document conversion tools with bounded resources.
#[derive(Debug, Clone)]
pub struct SystemDocumentTools {
    paths: ToolPaths,
    runner: CommandRunner,
}

impl SystemDocumentTools {
    /// Creates the concrete document tools adapter.
    #[must_use]
    pub const fn new(paths: ToolPaths, timeout: Duration, maximum_output_bytes: usize) -> Self {
        Self {
            paths,
            runner: CommandRunner::new(timeout, maximum_output_bytes),
        }
    }

    async fn ocr(&self, image: &Path) -> Result<String, ExtractionError> {
        self.runner
            .run(
                &self.paths.tesseract,
                &[image.as_os_str().to_owned(), OsString::from("stdout")],
            )
            .await
            .map_err(tool_error)
    }
}

#[async_trait]
impl DocumentTools for SystemDocumentTools {
    async fn pdf_text(&self, source: &Path) -> Result<String, ExtractionError> {
        self.runner
            .run(
                &self.paths.pdftotext,
                &[source.as_os_str().to_owned(), OsString::from("-")],
            )
            .await
            .map_err(tool_error)
    }

    async fn pdf_ocr(&self, source: &Path) -> Result<String, ExtractionError> {
        let directory = tempfile::tempdir().map_err(|error| io_error(&error))?;
        let prefix = directory.path().join("page");
        self.runner
            .run(
                &self.paths.pdftoppm,
                &[
                    OsString::from("-png"),
                    source.as_os_str().to_owned(),
                    prefix.as_os_str().to_owned(),
                ],
            )
            .await
            .map_err(tool_error)?;
        let mut images = fs::read_dir(directory.path())
            .map_err(|error| io_error(&error))?
            .map(|entry| {
                entry
                    .map(|value| value.path())
                    .map_err(|error| io_error(&error))
            })
            .collect::<Result<Vec<PathBuf>, ExtractionError>>()?;
        images.sort();
        if images.is_empty() {
            return Err(ExtractionError::Empty);
        }
        let mut text = String::new();
        for image in images {
            text.push_str(&self.ocr(&image).await?);
            text.push('\n');
        }
        Ok(text)
    }

    async fn image_text(&self, source: &Path) -> Result<String, ExtractionError> {
        self.ocr(source).await
    }

    async fn html_text(&self, source: &Path) -> Result<String, ExtractionError> {
        self.runner
            .run(
                &self.paths.pandoc,
                &[
                    OsString::from("--from=html"),
                    OsString::from("--to=plain"),
                    source.as_os_str().to_owned(),
                ],
            )
            .await
            .map_err(tool_error)
    }
}

fn tool_error(error: impl std::error::Error) -> ExtractionError {
    ExtractionError::Tool(error.to_string())
}

fn io_error(error: &std::io::Error) -> ExtractionError {
    ExtractionError::Tool(error.to_string())
}
