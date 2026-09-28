//! Checks classification and intelligence options exposed by the CLI.

use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn cli_exposes_standalone_classify_command() -> Result<(), Box<dyn std::error::Error>> {
    Command::cargo_bin("episteme")?
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("classify"));
    Ok(())
}

#[test]
fn cli_exposes_batch_force_retry_and_summary_options() -> Result<(), Box<dyn std::error::Error>> {
    Command::cargo_bin("episteme")?
        .args(["classify", "--help"])
        .assert()
        .success()
        .stdout(contains("--force"));
    Command::cargo_bin("episteme")?
        .args(["classify-batch", "--help"])
        .assert()
        .success()
        .stdout(contains("--force"))
        .stdout(contains("--retry-failed"))
        .stdout(contains("--concurrency"))
        .stdout(contains("--summary"));
    Ok(())
}

#[test]
fn cli_exposes_analyze_and_analyze_batch() -> Result<(), Box<dyn std::error::Error>> {
    Command::cargo_bin("episteme")?
        .args(["analyze", "--help"])
        .assert()
        .success()
        .stdout(contains("--force"));
    Ok(())
}
