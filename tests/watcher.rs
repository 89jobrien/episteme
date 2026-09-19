use std::io::Write;

use episteme::watch::{StableFileTracker, WatchDecision};

#[test]
fn watcher_ignores_unstable_and_out_of_root_files() -> Result<(), Box<dyn std::error::Error>> {
    let inbox = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let source = inbox.path().join("paper.pdf");
    let outside_source = outside.path().join("private.pdf");
    std::fs::write(&source, b"first")?;
    std::fs::write(&outside_source, b"outside")?;
    let mut tracker = StableFileTracker::new(inbox.path())?;

    assert_eq!(tracker.observe(&source), WatchDecision::Pending);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&source)?
        .write_all(b" update")?;
    assert_eq!(tracker.observe(&source), WatchDecision::Pending);
    assert!(matches!(tracker.observe(&source), WatchDecision::Ready(_)));
    assert_eq!(tracker.observe(&outside_source), WatchDecision::Ignore);
    Ok(())
}
