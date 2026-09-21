//! Tests atomic note creation, content sanitization, and source archiving.

use episteme::adapters::VaultFileStore;
use episteme::domain::{
    AnalysisProvenance, DocumentKind, EvidenceReference, ExtractionMethod, ResearchDraft,
    ResearchNote, SourceDigest, SourceFileName, StagedSource,
};
use episteme::ports::{VaultError, VaultStore};

#[test]
fn vault_create_note_is_atomic_and_no_clobber() -> Result<(), Box<dyn std::error::Error>> {
    let vault = tempfile::tempdir()?;
    let store = VaultFileStore::new(
        vault.path().to_path_buf(),
        "04_Research".into(),
        "09_Archive/Sources".into(),
    );
    let note = ResearchNote {
        draft: ResearchDraft {
            title: "Reliable Research".to_owned(),
            citation: "Local fixture".to_owned(),
            topics: vec!["testing".to_owned()],
            summary: "A grounded summary. <img src=\"https://example.invalid/track\">".to_owned(),
            key_ideas: vec!["Atomic writes matter. ![[private-note]]".to_owned()],
            implementation_notes: Vec::new(),
            critique: "Fixture scope is intentionally narrow.".to_owned(),
            evidence: vec![EvidenceReference {
                quote: "Atomic writes matter.".to_owned(),
                location: "paragraph 1".to_owned(),
            }],
        },
        source_digest: SourceDigest::from_bytes(b"fixture"),
        source_name: SourceFileName::new("fixture.pdf")?,
        extraction_method: ExtractionMethod::PdfText,
        analysis: AnalysisProvenance {
            function: "DistillResearch".to_owned(),
            client: "LocalDistiller".to_owned(),
            model: "fixture-model".to_owned(),
            pipeline_version: "0.1.0".to_owned(),
            processed_at: "2026-09-18T12:00:00Z".to_owned(),
        },
    };

    let stored = store.create_note(&note)?;
    let content = std::fs::read_to_string(vault.path().join(&stored.relative_path))?;
    assert!(content.contains("status: unprocessed"));
    assert!(!content.contains("<img"));
    assert!(!content.contains("![[private-note]]"));
    assert!(matches!(
        store.create_note(&note),
        Err(VaultError::AlreadyExists(_))
    ));
    assert_eq!(
        std::fs::read_dir(vault.path().join("04_Research"))?.count(),
        1
    );

    let unsafe_vault = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    std::os::unix::fs::symlink(outside.path(), unsafe_vault.path().join("research"))?;
    let unsafe_store = VaultFileStore::new(
        unsafe_vault.path().to_path_buf(),
        "research".into(),
        "archive".into(),
    );
    assert!(matches!(
        unsafe_store.create_note(&note),
        Err(VaultError::UnsafePath(_))
    ));

    let original = vault.path().join("original.pdf");
    let staged_path = vault.path().join("staged.pdf");
    std::fs::write(&original, b"archive fixture")?;
    std::fs::write(&staged_path, b"archive fixture")?;
    let staged = StagedSource::with_original(
        staged_path.clone(),
        original.clone(),
        SourceDigest::from_bytes(b"archive fixture"),
        SourceFileName::new("original.pdf")?,
        DocumentKind::Pdf,
    );
    store.archive_source(&staged)?;
    assert!(!staged_path.exists());
    assert!(original.exists());
    assert!(store.archive_source(&staged).is_ok());
    Ok(())
}
