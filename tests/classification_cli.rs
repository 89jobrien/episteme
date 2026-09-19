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
