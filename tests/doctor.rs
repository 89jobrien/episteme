use episteme::doctor::{Doctor, RequiredTool};

#[test]
fn doctor_reports_missing_dependency() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let report = Doctor::check_tools(&[RequiredTool {
        name: "missing".to_owned(),
        path: directory.path().join("not-installed"),
    }]);

    assert!(!report.is_healthy());
    assert_eq!(report.checks.len(), 1);
    assert!(!report.checks[0].available);
    Ok(())
}
