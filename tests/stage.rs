//! Tests immutable staging copies and rejection of symlinked paths.

use std::os::unix::fs::symlink;

use episteme::stage::{StageError, stage_source};

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

#[test]
fn staging_rejects_symlinked_parent_components() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let inbox = root.path().join("inbox");
    let real_directory = inbox.join("real");
    let alias_directory = inbox.join("alias");
    let staging = root.path().join("state/staging");
    std::fs::create_dir_all(&real_directory)?;
    std::fs::write(real_directory.join("paper.html"), "source")?;
    symlink(&real_directory, &alias_directory)?;

    assert!(matches!(
        stage_source(&inbox, &alias_directory.join("paper.html"), &staging, 1024),
        Err(StageError::NotRegularFile)
    ));
    Ok(())
}
