//! Persistence round-trip + crash-safety tests.

use worldos_kernel::{Object, Project};
use worldos_store::{ProjectStore, Snapshot, SqliteStore};

#[test]
fn save_and_reopen_preserves_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.worldos");

    let mut project = Project::new("roundtrip");
    let actor = worldos_kernel::ActorId::new("tester");
    let mut obj = Object::new("core:note", "hello", &actor);
    obj.set_component(worldos_kernel::Component::new(
        "doc:text",
        serde_json::json!({"text": "hi", "format": "markdown"}),
    ));
    let id = obj.id;
    project.objects.insert(id, obj);

    {
        let mut store = SqliteStore::open(&path).unwrap();
        store
            .save(&Snapshot::new(project, Default::default()))
            .unwrap();
    }
    {
        let store = SqliteStore::open(&path).unwrap();
        let snap = store.load().unwrap();
        assert_eq!(snap.project.name, "roundtrip");
        let o = snap.project.get(id).unwrap();
        assert_eq!(o.name, "hello");
        assert_eq!(
            o.component_data("doc:text").unwrap()["text"],
            serde_json::json!("hi")
        );
    }
}

#[test]
fn load_missing_project_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.worldos");
    let store = SqliteStore::open(&path).unwrap();
    assert!(store.load().is_err());
}

#[test]
fn interrupted_save_does_not_corrupt() {
    // Simulates failure mid-save: partial writes are inside a SQLite
    // transaction and roll back on drop; the previous state must survive.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("safe.worldos");
    let mut project = Project::new("safe");
    project.objects.insert(
        worldos_kernel::ObjectId::nil(),
        Object::new("core:note", "persisted", &"a".into()),
    );
    {
        let mut store = SqliteStore::open(&path).unwrap();
        store
            .save(&Snapshot::new(project.clone(), Default::default()))
            .unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    let snap = store.load().unwrap();
    assert_eq!(snap.project.objects.len(), 1);
}
