//! Real CLI boundary tests; CAD remains an explicit opt-in, never an agent run.
use assert_cmd::Command;
use serde_json::Value;

fn invoke(args: &[&str], success: bool) -> Value {
    let mut command = Command::cargo_bin("worldos").unwrap();
    command.timeout(std::time::Duration::from_secs(120));
    let result = command.args(args).arg("--json").assert();
    let result = if success {
        result.success()
    } else {
        result.failure()
    };
    serde_json::from_slice(&result.get_output().stdout)
        .expect("CLI stdout must be exactly one JSON value, including native CAD calls")
}

#[cfg(not(feature = "cad"))]
#[test]
fn unavailable_cad_opt_in_rejects_before_creating_project_or_sidecar() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("unavailable.worldos");
    let error = invoke(
        &["--cad", "new", "fixture", "--path", file.to_str().unwrap()],
        false,
    );
    assert!(error["error"].as_str().unwrap().contains("--features cad"));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn commands_without_opt_in_do_not_attach_cad() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plain.worldos");
    let file = file.to_str().unwrap();
    invoke(&["new", "plain", "--path", file], true);
    let schemas = invoke(&["commands", file], true);
    assert!(
        !schemas
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["command_type"] == "cad.create_box")
    );
    invoke(
        &["command", file, "cad.create_box", r#"{"size_mm":10}"#],
        false,
    );
    assert!(!std::path::Path::new(&format!("{file}.artifacts")).exists());
}

#[cfg(feature = "cad")]
mod native {
    use super::*;
    use serde_json::json;
    use std::path::Path;

    fn command(file: &str, kind: &str, input: Value, success: bool) -> Value {
        invoke(
            &["--cad", "command", file, kind, &input.to_string()],
            success,
        )
    }

    fn volume(output: &Value) {
        let actual = output["measures"]["volume_mm3"].as_f64().unwrap();
        assert!(
            (actual - 40_000.0).abs() < 0.04,
            "unexpected kernel volume: {actual}"
        );
    }

    fn artifact(file: &str, reference: &str) -> Vec<u8> {
        let digest = reference.strip_prefix("sha256:").unwrap();
        assert_eq!(digest.len(), 64);
        std::fs::read(
            Path::new(&format!("{file}.artifacts"))
                .join("objects")
                .join(&digest[..2])
                .join(digest),
        )
        .unwrap()
    }

    #[test]
    fn cad_cli_create_measure_step_import_undo_redo_reopen_regenerate() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("cad.worldos");
        let file = file.to_str().unwrap();
        invoke(&["--cad", "new", "cad", "--path", file], true);
        let schemas = invoke(&["--cad", "commands", file], true);
        assert!(
            schemas
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["command_type"] == "cad.create_box")
        );
        let created = command(
            file,
            "cad.create_box",
            json!({"name":"block","size_mm":[50,40,20]}),
            true,
        );
        volume(&created["output"]);
        assert_eq!(created["output"]["topology"]["faces"], 6);
        assert_eq!(created["output"]["topology"]["edges"], 12);
        assert_eq!(created["output"]["topology"]["is_valid"], true);
        assert!(!artifact(file, created["output"]["brep"].as_str().unwrap()).is_empty());
        volume(&command(file, "cad.measure", json!({"object":"block"}), true)["output"]);
        let exported = command(file, "cad.export_step", json!({"object":"block"}), true);
        let step = exported["output"]["step"].as_str().unwrap();
        assert!(String::from_utf8_lossy(&artifact(file, step)).contains("ISO-10303-21"));
        let step_file = dir.path().join("block.step");
        let project_before_copy = std::fs::read(file).unwrap();
        let copied = invoke(
            &[
                "artifact",
                "export",
                file,
                step,
                step_file.to_str().unwrap(),
            ],
            true,
        );
        assert_eq!(copied["verified"], true);
        assert_eq!(copied["reference"], step);
        assert_eq!(std::fs::read(file).unwrap(), project_before_copy);
        assert_eq!(std::fs::read(&step_file).unwrap(), artifact(file, step));
        let imported = command(
            file,
            "cad.import_step",
            json!({"file":step_file,"name":"roundtrip"}),
            true,
        );
        volume(&imported["output"]);
        invoke(&["--cad", "undo", file], true);
        assert_eq!(invoke(&["inspect", file], true)["object_count"], 1);
        // Undo removes graph references, not the content-addressed blob.
        assert!(!artifact(file, step).is_empty());
        invoke(&["--cad", "redo", file], true);
        assert_eq!(invoke(&["inspect", file], true)["object_count"], 2);
        command(file, "cad.regenerate", json!({"object":"block"}), true);
        volume(&command(file, "cad.measure", json!({"object":"block"}), true)["output"]);
        let history = invoke(&["history", file], true);
        assert!(
            history["transactions"]
                .as_array()
                .unwrap()
                .iter()
                .all(|t| t["actor"] == "local-user")
        );
    }

    #[test]
    fn documented_cad_batches_are_one_transaction_and_regenerate_after_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("examples.worldos");
        let file = file.to_str().unwrap();
        let create_script = dir.path().join("box.json");
        let regenerate_script = dir.path().join("regenerate.json");
        std::fs::write(
            &create_script,
            include_str!("../../../examples/cad/box.json"),
        )
        .unwrap();
        std::fs::write(
            &regenerate_script,
            include_str!("../../../examples/cad/regenerate.json"),
        )
        .unwrap();
        invoke(&["--cad", "new", "examples", "--path", file], true);
        let created = invoke(
            &["--cad", "batch", file, create_script.to_str().unwrap()],
            true,
        );
        volume(&created["outputs"][1]);
        assert_eq!(
            invoke(&["history", file], true)["transactions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        invoke(&["undo", file], true);
        assert_eq!(invoke(&["inspect", file], true)["object_count"], 0);
        invoke(&["redo", file], true);
        let regenerated = invoke(
            &["--cad", "batch", file, regenerate_script.to_str().unwrap()],
            true,
        );
        volume(&regenerated["outputs"][1]);
    }

    #[test]
    fn cad_cli_invalid_geometry_and_missing_artifact_preserve_graph_and_history() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("negative.worldos");
        let file = file.to_str().unwrap();
        invoke(&["--cad", "new", "negative", "--path", file], true);
        let before_graph = invoke(&["graph", file], true);
        let before_history = invoke(&["history", file], true);
        let bad = command(file, "cad.create_box", json!({"size_mm":[-1,40,20]}), false);
        assert!(bad["error"].is_string());
        let missing = command(
            file,
            "cad.import_step",
            json!({"step":format!("sha256:{}", "0".repeat(64))}),
            false,
        );
        assert!(missing["error"].as_str().unwrap().contains("step artifact"));
        assert_eq!(invoke(&["graph", file], true), before_graph);
        assert_eq!(invoke(&["history", file], true), before_history);
    }
}
