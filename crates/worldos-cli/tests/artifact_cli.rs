use assert_cmd::Command;
use serde_json::Value;
use std::fs;

const DIGEST: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

fn export(
    project: &std::path::Path,
    reference: &str,
    output: &std::path::Path,
    success: bool,
) -> Value {
    let mut command = Command::cargo_bin("worldos").unwrap();
    command.timeout(std::time::Duration::from_secs(120));
    let result = command
        .args(["artifact", "export"])
        .arg(project)
        .arg(reference)
        .arg(output)
        .arg("--json")
        .assert();
    let result = if success {
        result.success()
    } else {
        result.failure()
    };
    serde_json::from_slice(&result.get_output().stdout).unwrap()
}

#[test]
fn artifact_copy_needs_no_cad_and_never_opens_or_initializes_database() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("fixture.worldos");
    let output = dir.path().join("hello.txt");
    let reference = format!("sha256:{DIGEST}");
    assert!(export(&project, &reference, &output, false)["error"].is_string());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    fs::write(&project, b"not a database; association only").unwrap();
    assert!(export(&project, &reference, &output, false)["error"].is_string());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    let fan = dir.path().join("fixture.worldos.artifacts/objects/2c");
    fs::create_dir_all(&fan).unwrap();
    fs::write(fan.join(DIGEST), b"hello").unwrap();
    let result = export(&project, DIGEST, &output, true);
    assert_eq!(result["reference"], reference);
    assert_eq!(result["bytes"], 5);
    assert_eq!(result["verified"], true);
    assert_eq!(fs::read(&output).unwrap(), b"hello");
    assert!(export(&project, &reference, &output, false)["error"].is_string());
    assert_eq!(fs::read(&output).unwrap(), b"hello");
    assert_eq!(
        fs::read(project).unwrap(),
        b"not a database; association only"
    );
    assert!(!dir.path().join("fixture.worldos.artifacts/tmp").exists());
}
