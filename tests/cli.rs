use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

/// Tests that the CLI runs successfully with the noop reader.
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

/// Tests that the CLI falls back to the noop reader when the PC/SC reader is not available.
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

/// Tests the top-level `musicbox add` command writes the expected config entry.
#[test]
fn cli_add_command_writes_config() {
    let tmp = tempdir().expect("temp dir");
    let config_path = tmp.path().join("musicbox.toml");

    let mut cmd = Command::cargo_bin("musicbox").expect("binary");
    cmd.arg("add")
        .arg("--config")
        .arg(&config_path)
        .arg("--track")
        .arg("songs/example.mp3")
        .arg("--card")
        .arg("deadbeef")
        .arg("--reader")
        .arg("noop")
        .arg("--skip-tag-write");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Mapped card deadbeef to songs/example.mp3"));

    let contents =
        fs::read_to_string(&config_path).expect("command should create the target config");
    assert!(contents.contains("deadbeef"), "config should contain UID");
    assert!(
        contents.contains("songs/example.mp3"),
        "config should contain track path"
    );
}

/// Tests that `musicbox <CONFIG> tag add` falls back to the positional config path.
#[test]
fn cli_tag_add_uses_positional_config() {
    let tmp = tempdir().expect("temp dir");
    let config_path = tmp.path().join("library.toml");

    let mut cmd = Command::cargo_bin("musicbox").expect("binary");
    cmd.arg(&config_path)
        .arg("tag")
        .arg("add")
        .arg("--track")
        .arg("songs/other.mp3")
        .arg("--card")
        .arg("cafebabe")
        .arg("--reader")
        .arg("noop")
        .arg("--skip-tag-write");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Mapped card cafebabe to songs/other.mp3"));

    let contents =
        fs::read_to_string(&config_path).expect("command should create the target config");
    assert!(
        contents.contains("cafebabe"),
        "config should include the provided UID"
    );
    assert!(
        contents.contains("songs/other.mp3"),
        "config should include the provided track"
    );
}
