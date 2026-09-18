//! Artifact store integrity: round-trip, corruption detection,
//! self-heal on put, gc semantics, sidecar layout.

use std::collections::HashSet;
use std::fs;

use worldos_artifact::{ArtifactError, ArtifactRef, ArtifactStore};

fn store() -> (tempfile::TempDir, ArtifactStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = ArtifactStore::open(dir.path().join("blobs")).unwrap();
    (dir, store)
}

#[test]
fn put_get_roundtrip_and_dedup() {
    let (_d, store) = store();
    let bytes = b"ISO-10303-21; fake step payload".repeat(64);
    let out = store.put(&bytes).unwrap();
    assert!(out.written);
    assert_eq!(out.size, bytes.len() as u64);
    assert_eq!(out.artifact_ref.to_string().len(), "sha256:".len() + 64);

    // same bytes → same ref, dedup hit
    let out2 = store.put(&bytes).unwrap();
    assert_eq!(out.artifact_ref, out2.artifact_ref);
    assert!(!out2.written);

    assert_eq!(store.get(&out.artifact_ref).unwrap(), bytes);
    assert!(store.exists(&out.artifact_ref));
    assert!(store.verify(&out.artifact_ref).unwrap());
}

#[test]
fn corrupted_blob_is_detected_and_healed_by_put() {
    let (_d, store) = store();
    let bytes = b"payload";
    let out = store.put(bytes).unwrap();

    // corrupt the object on disk
    let path = store
        .dir()
        .join("objects")
        .join(&out.artifact_ref.hex()[..2])
        .join(out.artifact_ref.hex());
    fs::write(&path, b"evil").unwrap();

    assert!(!store.verify(&out.artifact_ref).unwrap());
    match store.get(&out.artifact_ref) {
        Err(ArtifactError::Corrupt { expected, actual }) => {
            assert_eq!(expected, out.artifact_ref.to_string());
            assert_ne!(actual, out.artifact_ref.to_string());
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }

    // put of the correct bytes heals the corrupt slot
    let healed = store.put(bytes).unwrap();
    assert!(healed.written);
    assert_eq!(store.get(&out.artifact_ref).unwrap(), bytes);
}

#[test]
fn missing_blob_is_not_found() {
    let (_d, store) = store();
    let r: ArtifactRef = "sha256:0000000000000000000000000000000000000000000000000000000000000001"
        .parse()
        .unwrap();
    assert!(!store.exists(&r));
    assert!(matches!(store.get(&r), Err(ArtifactError::NotFound(_))));
}

#[test]
fn invalid_refs_are_rejected() {
    for bad in [
        "",
        "sha256:",
        "sha256:zzzz",
        "sha256:ABCD",
        "md5:0000000000000000000000000000000000000000000000000000000000000000",
        &"x".repeat(65),
    ] {
        assert!(bad.parse::<ArtifactRef>().is_err(), "accepted {bad}");
    }
    // bare hex also accepted (no prefix required)
    assert!(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .parse::<ArtifactRef>()
            .is_ok()
    );
}

#[test]
fn gc_collects_unreferenced_and_keeps_live() {
    let (_d, store) = store();
    let a = store.put(b"a").unwrap().artifact_ref;
    let b = store.put(b"b").unwrap().artifact_ref;
    let c = store.put(b"c").unwrap().artifact_ref;

    let keep: HashSet<_> = [a.clone(), c.clone()].into_iter().collect();
    let report = store.gc(&keep).unwrap();
    assert_eq!(report.kept, 2);
    assert_eq!(report.removed, 1);
    assert!(report.freed_bytes >= 1);
    assert!(store.exists(&a));
    assert!(!store.exists(&b));
    assert!(store.exists(&c));

    // second gc is a no-op
    let report2 = store.gc(&keep).unwrap();
    assert_eq!(report2.removed, 0);
    assert_eq!(report2.kept, 2);
}

#[test]
fn sidecar_dir_derives_from_project_path() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("demo.worldos");
    let store = ArtifactStore::for_project(&project).unwrap();
    assert_eq!(
        store.dir().file_name().unwrap().to_str().unwrap(),
        "demo.worldos.artifacts"
    );
    assert!(store.dir().is_dir());
}

#[test]
fn ref_serde_roundtrips_as_prefixed_string() {
    let r = ArtifactRef::of(b"hello");
    let json = serde_json::to_string(&r).unwrap();
    assert!(json.starts_with("\"sha256:"));
    let back: ArtifactRef = serde_json::from_str(&json).unwrap();
    assert_eq!(r, back);
}
