//! Save/Save-As durability proofs:
//!
//!   journaled staged save, overwrite protection, failure injection at
//!   every stage boundary, crash recovery via the commit journal,
//!   artifact migration + liveness verification, GC reachability.
//!
//! `Engine::save_as_staged` exposes a stage hook so every failure
//!   frontier is exercised for real — not simulated.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use worldos_adapter_cadrum::CadrumKernel;
use worldos_engine::{Engine, EngineError, SaveOptions, SaveStage};

fn sidecar(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".artifacts");
    s.into()
}

fn blob_exists(path: &Path, sha_ref: &str) -> bool {
    let hex = sha_ref.strip_prefix("sha256:").unwrap_or(sha_ref);
    sidecar(path)
        .join("objects")
        .join(&hex[..2])
        .join(hex)
        .is_file()
}

fn wip(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push("-wip");
    s.into()
}

fn journal(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push("-journal");
    s.into()
}

fn cad_engine(dir: &Path, name: &str) -> Engine {
    let mut e = Engine::create(name, dir.join(format!("{name}.worldos"))).unwrap();
    e.attach_cad(Arc::new(CadrumKernel::new())).unwrap();
    e
}

#[test]
fn save_as_refuses_foreign_destination_without_grant() {
    let dir = tempfile::tempdir().unwrap();
    let p1 = dir.path().join("a.worldos");
    let p2 = dir.path().join("b.worldos");

    let mut a = cad_engine(dir.path(), "a");
    a.execute("cad.create_box", json!({"size_mm": 10.0, "name": "box"}))
        .unwrap();

    // An unrelated project already occupies p2.
    let mut b = Engine::create("b", &p2).unwrap();
    b.execute(
        "object.create",
        json!({"type": "core:note", "name": "mine"}),
    )
    .unwrap();
    b.save().unwrap();
    drop(b);

    // Refused — and `a` still bound to p1, dirty state honest.
    let err = a.save_as(&p2).unwrap_err();
    assert!(
        matches!(err, EngineError::DestinationExists(_)),
        "unexpected: {err}"
    );
    assert_eq!(a.path().unwrap(), p1.as_path());
    assert!(a.is_dirty());

    // p2 untouched.
    let b2 = Engine::open(&p2).unwrap();
    assert!(b2.project().find_by_name("mine").is_some());
    assert!(b2.project().find_by_name("box").is_none());

    // With the explicit grant, overwrite works and artifacts migrate.
    a.save_as_opts(&p2, SaveOptions { overwrite: true })
        .unwrap();
    let brep = a
        .project()
        .find_by_name("box")
        .unwrap()
        .component_data("cad:shape")
        .unwrap()["brep"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(blob_exists(&p2, &brep));
    drop(a);
    let c = Engine::open(&p2).unwrap();
    assert!(c.project().find_by_name("box").is_some());
}

#[test]
fn failure_before_commit_never_touches_target_or_state() {
    let dir = tempfile::tempdir().unwrap();
    let p1 = dir.path().join("src.worldos");
    let p2 = dir.path().join("dst.worldos");

    let mut e = Engine::create("src", &p1).unwrap();
    e.attach_cad(Arc::new(CadrumKernel::new())).unwrap();
    e.execute("cad.create_box", json!({"size_mm": 10.0, "name": "box"}))
        .unwrap();
    e.save().unwrap();
    e.execute("cad.create_box", json!({"size_mm": 5.0, "name": "box2"}))
        .unwrap();

    // Inject failure at every pre-commit boundary; each must leave the
    // engine bound to p1 and p2 absent.
    for stage in [
        SaveStage::Artifacts,
        SaveStage::WipWrite,
        SaveStage::WipVerify,
        SaveStage::CommitPoint,
        SaveStage::Rename,
        SaveStage::Rebind,
    ] {
        let mut fired = false;
        e.save_as_staged(&p2, SaveOptions::default(), &mut |s| {
            if s == stage && !fired {
                fired = true;
                return Err(EngineError::Other(format!("injected at {stage:?}")));
            }
            Ok(())
        })
        .unwrap_err();
        assert!(fired);
        match stage {
            // A caught error at Rename reneges on the commit: cleanup
            // wipes journal+wip, so the target was never created and the
            // original is intact. (A real crash mid-rename leaves the
            // journal — covered by interrupted_save_recovers_via_journal.)
            SaveStage::Rename => {
                assert_eq!(e.path().unwrap(), p1.as_path());
                assert!(!p2.exists());
            }
            // Rebind fails AFTER the rename committed: the file on disk
            // is complete and valid; only the in-memory flip was skipped.
            SaveStage::Rebind => {
                assert_eq!(e.path().unwrap(), p1.as_path());
                let r = Engine::open(&p2).unwrap();
                assert!(r.project().find_by_name("box2").is_some());
                std::fs::remove_file(&p2).unwrap();
            }
            _ => {
                assert_eq!(e.path().unwrap(), p1.as_path());
                assert!(!p2.exists(), "{stage:?} leaked a target file");
                assert!(!wip(&p2).exists(), "{stage:?} left wip staging");
                assert!(!journal(&p2).exists(), "{stage:?} left journal");
            }
        }
        // Original remains perfectly usable throughout.
        e.save().unwrap();
    }
    drop(e);
    let check = Engine::open(&p1).unwrap();
    assert!(check.project().find_by_name("box").is_some());
    assert!(check.project().find_by_name("box2").is_some());
}

#[test]
fn interrupted_save_recovers_via_journal() {
    let dir = tempfile::tempdir().unwrap();
    let p2 = dir.path().join("dst.worldos");

    let mut e = cad_engine(dir.path(), "src");
    e.execute("cad.create_box", json!({"size_mm": 10.0, "name": "box"}))
        .unwrap();

    // Craft a crash after commit: journal=committed + a real staged db.
    {
        use worldos_store::{ProjectStore, SqliteStore};
        let mut st = SqliteStore::open_staged(wip(&p2)).unwrap();
        st.save(&e.snapshot()).unwrap();
        drop(st);
        std::fs::write(
            journal(&p2),
            json!({"v":1,"op":"save","phase":"committed"}).to_string(),
        )
        .unwrap();
    }
    // Reopen p2: reconcile finishes the rename — a crash between
    // commit-point and rename loses nothing.
    let r = Engine::open(&p2).unwrap();
    assert!(r.project().find_by_name("box").is_some());
    assert!(!wip(&p2).exists() && !journal(&p2).exists());

    // Crash BEFORE commit: journal=staged + wip present → both wiped,
    // pre-existing target untouched.
    let p3 = dir.path().join("pre.worldos");
    let mut keep = Engine::create("keep", &p3).unwrap();
    keep.execute(
        "object.create",
        json!({"type": "core:note", "name": "keep"}),
    )
    .unwrap();
    keep.save().unwrap();
    drop(keep);
    {
        use worldos_store::{ProjectStore, SqliteStore};
        let mut st = SqliteStore::open_staged(wip(&p3)).unwrap();
        st.save(&e.snapshot()).unwrap();
        drop(st);
        std::fs::write(
            journal(&p3),
            json!({"v":1,"op":"save","phase":"staged"}).to_string(),
        )
        .unwrap();
    }
    let r3 = Engine::open(&p3).unwrap();
    assert!(r3.project().find_by_name("keep").is_some());
    assert!(r3.project().find_by_name("box").is_none());
    assert!(!wip(&p3).exists() && !journal(&p3).exists());
}

#[test]
fn reopen_verifies_missing_and_corrupt_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let p1 = dir.path().join("art.worldos");
    let mut e = Engine::create("art", &p1).unwrap();
    e.attach_cad(Arc::new(CadrumKernel::new())).unwrap();
    e.execute("cad.create_box", json!({"size_mm": 10.0, "name": "box"}))
        .unwrap();
    let brep = e
        .project()
        .find_by_name("box")
        .unwrap()
        .component_data("cad:shape")
        .unwrap()["brep"]
        .as_str()
        .unwrap()
        .to_string();
    e.save().unwrap();
    drop(e);

    // Delete the blob → reopen + measure fails honestly.
    let hex = &brep["sha256:".len()..];
    let blob = sidecar(&p1).join("objects").join(&hex[..2]).join(hex);
    let bytes = std::fs::read(&blob).unwrap();
    std::fs::remove_file(&blob).unwrap();

    let mut e2 = Engine::open(&p1).unwrap();
    e2.attach_cad(Arc::new(CadrumKernel::new())).unwrap();
    let err = e2
        .execute("cad.measure", json!({"object": "box"}))
        .unwrap_err()
        .to_string();
    assert!(!err.is_empty());
    drop(e2);

    // Corrupt the blob → integrity check on read rejects it.
    std::fs::write(&blob, b"not the real brep").unwrap();
    let store = worldos_artifact::ArtifactStore::for_project(&p1).unwrap();
    assert!(store.get(&brep.parse().unwrap()).is_err());
    // restore for the next assertion
    std::fs::write(&blob, &bytes).unwrap();
    assert!(store.get(&brep.parse().unwrap()).unwrap() == bytes);
}

#[test]
fn gc_keeps_history_reachable_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let p1 = dir.path().join("gc.worldos");
    let mut e = Engine::create("gc", &p1).unwrap();
    e.attach_cad(Arc::new(CadrumKernel::new())).unwrap();
    e.execute("cad.create_box", json!({"size_mm": 10.0, "name": "box"}))
        .unwrap();
    let brep = e
        .project()
        .find_by_name("box")
        .unwrap()
        .component_data("cad:shape")
        .unwrap()["brep"]
        .as_str()
        .unwrap()
        .to_string();
    e.save().unwrap();

    // Garbage blob directly into the store.
    let store = e.cad().unwrap().artifacts();
    let junk = store.put(b"junk blob").unwrap().artifact_ref.to_string();
    assert!(blob_exists(&p1, &junk));

    // Dry-run reports, deletes nothing.
    let rep = e.gc_artifacts(true).unwrap().unwrap();
    assert!(rep.removed >= 1);
    assert!(blob_exists(&p1, &junk));
    assert!(blob_exists(&p1, &brep));

    // Delete the object: its brep is still referenced by HISTORY (undo
    // can revive it) — GC must keep it.
    let id = e.project().find_by_name("box").unwrap().id.to_string();
    e.execute("object.delete", json!({"id": id})).unwrap();
    let rep = e.gc_artifacts(false).unwrap().unwrap();
    assert!(rep.removed >= 1);
    assert!(!blob_exists(&p1, &junk), "unreferenced junk survived gc");
    assert!(
        blob_exists(&p1, &brep),
        "gc removed a blob still reachable through undo history"
    );

    // Undo revives the object — and the brep still resolves.
    e.undo().unwrap();
    e.execute("cad.measure", json!({"object": "box"})).unwrap();
}
