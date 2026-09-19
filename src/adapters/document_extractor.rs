//! Format routing and deterministic OCR fallback.

use std::fs;

use async_trait::async_trait;

use crate::domain::{DocumentKind, ExtractedDocument, ExtractionMethod, StagedSource};
use crate::ports::{DocumentExtractor, DocumentTools, ExtractionError};

/// Routes supported document formats through configured conversion tools.
#[derive(Debug)]
pub struct DocumentCliExtractor<T> {
    tools: T,
    minimum_text_characters: usize,
}

impl<T> DocumentCliExtractor<T> {
    /// Creates an extractor with a deterministic direct-text quality threshold.
    #[must_use]
    pub const fn new(tools: T, minimum_text_characters: usize) -> Self {
        Self {
            tools,
            minimum_text_characters,
        }
    }
}

#[async_trait]
impl<T> DocumentExtractor for DocumentCliExtractor<T>
where
    T: DocumentTools,
{
    async fn extract(&self, source: &StagedSource) -> Result<ExtractedDocument, ExtractionError> {
        verify_source(source)?;
        let (text, method) = match source.kind() {
            DocumentKind::Pdf => {
                let direct_text = self.tools.pdf_text(source.path()).await?;
                if direct_text
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .count()
                    >= self.minimum_text_characters
                {
                    (direct_text, ExtractionMethod::PdfText)
                } else {
                    (
                        self.tools.pdf_ocr(source.path()).await?,
                        ExtractionMethod::PdfOcr,
                    )
                }
            }
            DocumentKind::Html => (
                self.tools.html_text(source.path()).await?,
                ExtractionMethod::HtmlPandoc,
            ),
            DocumentKind::Image => (
                self.tools.image_text(source.path()).await?,
                ExtractionMethod::ImageOcr,
            ),
        };
        verify_source(source)?;

        ExtractedDocument::new(source.clone(), text, method)
            .map_err(|error| ExtractionError::Invalid(error.to_string()))
    }
}

fn verify_source(source: &StagedSource) -> Result<(), ExtractionError> {
    let bytes =
        fs::read(source.path()).map_err(|error| ExtractionError::Tool(error.to_string()))?;
    if &crate::domain::SourceDigest::from_bytes(&bytes) != source.digest() {
        return Err(ExtractionError::SourceChanged);
    }
    Ok(())
}
