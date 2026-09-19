use episteme::stage::stage_source;

#[test]
fn staging_uses_an_immutable_private_copy() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let inbox = root.path().join("inbox");
    let staging = root.path().join("state/staging");
    std::fs::create_dir(&inbox)?;
    let original = inbox.join("paper.html");
    std::fs::write(&original, "original source")?;

    let staged = stage_source(&inbox, &original, &staging, 1024)?;
    std::fs::write(&original, "replacement source")?;

    assert_ne!(staged.path(), staged.original_path());
    assert_eq!(std::fs::read_to_string(staged.path())?, "original source");
    assert_eq!(
        std::fs::read_to_string(staged.original_path())?,
        "replacement source"
    );
    Ok(())
}
