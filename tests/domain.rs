use episteme::domain::{SourceDigest, SourceFileName};

#[test]
fn source_metadata_rejects_unsafe_values() {
    assert!(SourceDigest::parse("ABC").is_err());
    assert!(SourceFileName::new("../secret.pdf").is_err());
    assert!(SourceFileName::new("folder/secret.pdf").is_err());
    assert!(SourceFileName::new("safe-document.pdf").is_ok());
}
