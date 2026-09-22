use std::fs;
use worldos_artifact::{ArtifactReadError, ArtifactRef, ArtifactStore, MAX_EXPORT_BYTES};
use worldos_engine::{ArtifactExportError, Engine};
use worldos_kernel::actor::{Actor, PermissionSet};

#[test]
fn verified_copy_normalizes_refs_and_preserves_source() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("fixture.worldos");
    // Association only: this operation deliberately does not parse SQLite.
    fs::write(&project, b"project fixture").unwrap();
    let store = ArtifactStore::for_project(&project).unwrap();
    let reference = store.put(b"verified bytes").unwrap().artifact_ref;
    let actor = Actor::human("operator");
    for (index, text) in [reference.to_string(), reference.hex().into()]
        .iter()
        .enumerate()
    {
        let output = dir.path().join(format!("output-{index}"));
        let report = Engine::export_project_artifact(&project, &actor, text, &output).unwrap();
        assert!(report.verified);
        assert_eq!(report.reference, reference.to_string());
        assert_eq!(report.bytes, 14);
        assert_eq!(fs::read(output).unwrap(), b"verified bytes");
    }
    assert_eq!(fs::read(project).unwrap(), b"project fixture");
    assert_eq!(store.get(&reference).unwrap(), b"verified bytes");
}

#[test]
fn denied_actors_are_rejected_before_reference_or_filesystem_checks() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("missing");
    let output = dir.path().join("out");
    let mut agent = Actor::agent("fixture");
    agent.permissions = PermissionSet::all();
    assert!(matches!(
        Engine::export_project_artifact(&project, &agent, "bad", &output),
        Err(ArtifactExportError::OperatorRequired)
    ));
    for granted in ["artifact.export", "filesystem.write"] {
        let mut human = Actor::human("restricted");
        human.permissions = PermissionSet::default();
        human.permissions.grant(granted.into());
        assert!(matches!(
            Engine::export_project_artifact(&project, &human, "bad", &output),
            Err(ArtifactExportError::PermissionDenied(_))
        ));
    }
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn invalid_missing_corrupt_and_oversized_inputs_never_create_output() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let output = dir.path().join("out");
    let actor = Actor::human("operator");
    let reference = ArtifactRef::of(b"original");
    assert!(
        Engine::export_project_artifact(&project, &actor, &reference.to_string(), &output).is_err()
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    fs::write(&project, b"unchanged").unwrap();
    for text in ["bad", &reference.to_string()] {
        assert!(Engine::export_project_artifact(&project, &actor, text, &output).is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    let store = ArtifactStore::for_project(&project).unwrap();
    store.put(b"original").unwrap();
    let blob = store
        .dir()
        .join("objects")
        .join(&reference.hex()[..2])
        .join(reference.hex());
    fs::write(&blob, b"corrupt").unwrap();
    assert!(matches!(
        Engine::export_project_artifact(&project, &actor, &reference.to_string(), &output),
        Err(ArtifactExportError::Read(ArtifactReadError::Artifact(_)))
    ));
    fs::File::create(&blob)
        .unwrap()
        .set_len(MAX_EXPORT_BYTES + 1)
        .unwrap();
    assert!(matches!(
        Engine::export_project_artifact(&project, &actor, &reference.to_string(), &output),
        Err(ArtifactExportError::Read(ArtifactReadError::TooLarge))
    ));
    assert!(!output.exists());
    assert_eq!(fs::read(project).unwrap(), b"unchanged");
}

#[test]
fn existing_destinations_and_missing_parents_are_not_changed() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::write(&project, b"project").unwrap();
    let reference = ArtifactStore::for_project(&project)
        .unwrap()
        .put(b"blob")
        .unwrap()
        .artifact_ref;
    let actor = Actor::human("operator");
    let existing = dir.path().join("existing");
    fs::write(&existing, b"valuable").unwrap();
    for output in [&existing, dir.path(), &dir.path().join("absent/out")] {
        assert!(matches!(
            Engine::export_project_artifact(&project, &actor, &reference.to_string(), output),
            Err(ArtifactExportError::Create { .. })
        ));
    }
    assert_eq!(fs::read(&existing).unwrap(), b"valuable");
    assert!(!dir.path().join("absent").exists());
}

#[cfg(any(unix, windows))]
#[test]
fn existing_destination_symlink_is_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::write(&project, b"project").unwrap();
    let reference = ArtifactStore::for_project(&project)
        .unwrap()
        .put(b"blob")
        .unwrap()
        .artifact_ref;
    let target = dir.path().join("target");
    fs::write(&target, b"valuable").unwrap();
    let link = dir.path().join("link");
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(&target, &link);
    #[cfg(windows)]
    let result = std::os::windows::fs::symlink_file(&target, &link);
    if let Err(error) = result {
        #[cfg(windows)]
        if error.raw_os_error() == Some(1314) {
            eprintln!("SKIP symlink regression: Windows symlink privilege unavailable (1314)");
            return;
        }
        panic!("create fixture symlink: {error}");
    }
    assert!(matches!(
        Engine::export_project_artifact(
            &project,
            &Actor::human("operator"),
            &reference.to_string(),
            &link
        ),
        Err(ArtifactExportError::Create { .. })
    ));
    assert_eq!(fs::read(target).unwrap(), b"valuable");
    assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
}
