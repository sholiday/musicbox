use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::path::Path;
use std::process::Command;

#[test]
fn cli_runs_with_noop_reader() {
    let config = Path::new("examples/config.example.toml");
    assert!(config.exists(), "example config missing");

    let mut cmd = Command::cargo_bin("musicbox").expect("binary");
    cmd.arg(config)
        .arg("--reader")
        .arg("noop")
        .arg("--poll-interval-ms")
        .arg("10")
        .arg("--silent")
        .env("MUSICBOX_NOOP_SHUTDOWN", "1");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Loaded configuration"))
        .stdout(predicate::str::contains("Reader requested shutdown"));
}

#[test]
fn cli_auto_reader_falls_back_when_pcsc_missing() {
    let config = Path::new("examples/config.example.toml");
    assert!(config.exists(), "example config missing");

    let mut cmd = Command::cargo_bin("musicbox").expect("binary");
    cmd.arg(config)
        .arg("--reader")
        .arg("auto")
        .arg("--poll-interval-ms")
        .arg("10")
        .arg("--silent")
        .env("MUSICBOX_NOOP_SHUTDOWN", "1");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Loaded configuration"));
}
