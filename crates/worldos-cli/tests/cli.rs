//! End-to-end CLI tests through the real `worldos` binary.

use assert_cmd::Command;
use serde_json::Value;

fn worldos() -> Command {
    Command::cargo_bin("worldos").unwrap()
}

fn proj(dir: &tempfile::TempDir) -> String {
    dir.path().join("t.worldos").display().to_string()
}

#[test]
fn new_command_inspect_undo_redo_flow() {
    let dir = tempfile::tempdir().unwrap();
    let f = proj(&dir);

    worldos()
        .args(["new", "t", "--path", &f])
        .assert()
        .success();

    worldos()
        .args([
            "command",
            &f,
            "object.create",
            r#"{"type":"core:note","name":"cli-note","components":{"doc:text":{"text":"x"}}}"#,
        ])
        .assert()
        .success();

    let out = worldos().args(["inspect", &f, "--json"]).assert().success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["object_count"], 1);

    worldos().args(["undo", &f]).assert().success();
    let out = worldos().args(["inspect", &f, "--json"]).assert().success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["object_count"], 0);

    worldos().args(["redo", &f]).assert().success();
    let out = worldos().args(["inspect", &f, "--json"]).assert().success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["object_count"], 1);
}

#[test]
fn missing_file_and_bad_command_fail_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let f = proj(&dir);

    worldos().args(["inspect", &f]).assert().failure();

    worldos()
        .args(["new", "t", "--path", &f])
        .assert()
        .success();
    worldos()
        .args(["command", &f, "not.a.command", "{}"])
        .assert()
        .failure();
    // invalid JSON input
    worldos()
        .args(["command", &f, "object.create", "{bad json"])
        .assert()
        .failure();
}

#[test]
fn history_and_validate_subcommands() {
    let dir = tempfile::tempdir().unwrap();
    let f = proj(&dir);

    worldos()
        .args(["new", "t", "--path", &f])
        .assert()
        .success();
    worldos()
        .args([
            "command",
            &f,
            "geometry.create_primitive",
            r#"{"kind":"cube","name":"c1"}"#,
        ])
        .assert()
        .success();

    let out = worldos().args(["history", &f, "--json"]).assert().success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let txns = v["transactions"].as_array().unwrap();
    assert!(!txns.is_empty());
    assert_eq!(txns[0]["actor"], "local-user"); // attribution present

    worldos().args(["validate", &f]).assert().success();
    worldos().args(["doctor"]).assert().success();
    worldos()
        .args(["commands", &f, "--json"])
        .assert()
        .success();
}

/// Full plugin e2e: the `worldos` binary itself acts as the plugin via the
/// hidden `plugin-shim` subcommand — a real subprocess speaking the hosted
/// plugin protocol.
#[test]
fn plugin_run_commits_and_rollback_on_crash() {
    let dir = tempfile::tempdir().unwrap();
    let f = proj(&dir);
    worldos()
        .args(["new", "t", "--path", &f])
        .assert()
        .success();

    // resolve our own binary to act as the plugin process
    let exe = std::env::var("CARGO_BIN_EXE_worldos").unwrap_or_else(|_| {
        assert_cmd::cargo::cargo_bin("worldos")
            .display()
            .to_string()
    });

    let out = worldos()
        .args(["--json", "plugin", "run", &f, &exe, "plugin-shim"])
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["committed"], true, "{v}");

    let out = worldos().args(["inspect", &f, "--json"]).assert().success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["object_count"], 1, "plugin note persisted");

    // plugin crash → transaction rolls back, nothing persists
    let f2 = dir.path().join("t2.worldos").display().to_string();
    worldos()
        .args(["new", "t", "--path", &f2])
        .assert()
        .success();
    let out = worldos()
        .args([
            "--json",
            "plugin",
            "run",
            &f2,
            &exe,
            "plugin-shim",
            "--fail",
        ])
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["committed"], false, "{v}");

    let out = worldos()
        .args(["inspect", &f2, "--json"])
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["object_count"], 0, "crashed plugin left no objects");
}
