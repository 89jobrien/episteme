//! Tests format routing and OCR fallback during document extraction.

use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use episteme::adapters::DocumentCliExtractor;
use episteme::domain::{
    DocumentKind, ExtractionMethod, SourceDigest, SourceFileName, StagedSource,
};
use episteme::ports::{DocumentExtractor, DocumentTools, ExtractionError};

#[derive(Debug, Default)]
struct FakeTools {
    calls: Mutex<Vec<&'static str>>,
}

#[async_trait]
impl DocumentTools for FakeTools {
    async fn pdf_text(&self, _source: &Path) -> Result<String, ExtractionError> {
        self.calls.lock().map_err(lock_error)?.push("pdf_text");
        Ok("   ".to_owned())
    }

    async fn pdf_ocr(&self, _source: &Path) -> Result<String, ExtractionError> {
        self.calls.lock().map_err(lock_error)?.push("pdf_ocr");
        Ok("grounded OCR output".to_owned())
    }

    async fn image_text(&self, _source: &Path) -> Result<String, ExtractionError> {
        self.calls.lock().map_err(lock_error)?.push("image_text");
        Ok("image OCR output".to_owned())
    }

    async fn html_text(&self, _source: &Path) -> Result<String, ExtractionError> {
        self.calls.lock().map_err(lock_error)?.push("html_text");
        Ok("normalized HTML".to_owned())
    }
}

fn lock_error<T>(_error: std::sync::PoisonError<T>) -> ExtractionError {
    ExtractionError::Tool("fake call log lock was poisoned".to_owned())
}

#[tokio::test]
async fn extractor_routes_formats_and_uses_ocr_fallback() -> Result<(), Box<dyn std::error::Error>>
{
    let tools = FakeTools::default();
    let extractor = DocumentCliExtractor::new(tools, 8);
    let directory = tempfile::tempdir()?;
    let source_path = directory.path().join("paper.pdf");
    std::fs::write(&source_path, b"pdf")?;
    let source = StagedSource::new(
        source_path,
        SourceDigest::from_bytes(b"pdf"),
        SourceFileName::new("paper.pdf")?,
        DocumentKind::Pdf,
    );

    let extracted = extractor.extract(&source).await?;

    assert_eq!(extracted.method(), ExtractionMethod::PdfOcr);
    assert_eq!(extracted.text(), "grounded OCR output");
    Ok(())
}
