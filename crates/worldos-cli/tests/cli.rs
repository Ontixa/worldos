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

    worldos().args(["new", "t", "--path", &f]).assert().success();

    worldos()
        .args([
            "command", &f, "object.create",
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

    worldos().args(["new", "t", "--path", &f]).assert().success();
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

    worldos().args(["new", "t", "--path", &f]).assert().success();
    worldos()
        .args(["command", &f, "geometry.create_primitive",
               r#"{"kind":"cube","name":"c1"}"#])
        .assert()
        .success();

    let out = worldos().args(["history", &f, "--json"]).assert().success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let txns = v["transactions"].as_array().unwrap();
    assert!(!txns.is_empty());
    assert_eq!(txns[0]["actor"], "local-user"); // attribution present

    worldos().args(["validate", &f]).assert().success();
    worldos().args(["doctor"]).assert().success();
    worldos().args(["commands", &f, "--json"]).assert().success();
}
