//! Persistence-boundary adversarial tests (NEXT.md #7):
//!
//! - random snapshots → save → load → identical state (objects,
//!   relations, settings, full journal including undo cursor);
//! - process abort mid-write (uncommitted frames in the WAL) → reopen
//!   recovers the last committed snapshot, never a torn one;
//! - process abort right after a committed save → the commit survives;
//! - truncated/garbage files fail cleanly — error, not panic.

use proptest::prelude::*;
use serde_json::{Value, json};
use std::path::Path;
use worldos_commands::{CommandEnvelope, CommandRecord, History, TransactionRecord};
use worldos_kernel::delta::StateOp;
use worldos_kernel::ids::RelationId;
use worldos_kernel::{ActorId, Component, Object, Project, Relation};
use worldos_store::{ProjectStore, Snapshot, SqliteStore};

// ------------------------------------------------------------------ canonical form

/// Order-independent serialization — the store must reproduce it
/// exactly (ids, meta timestamps, JSON payload byte-for-byte).
fn canonical(p: &Project) -> Value {
    let mut objs: Vec<_> = p.objects.values().collect();
    objs.sort_by_key(|o| o.id);
    let mut rels: Vec<_> = p.relations.values().collect();
    rels.sort_by_key(|r| r.id);
    json!({
        "id": p.id.to_string(),
        "name": p.name,
        "schema_version": p.schema_version,
        "created_at": p.created_at,
        "settings": p.settings,
        "objects": objs,
        "relations": rels,
    })
}

fn canonical_history(h: &History) -> Value {
    json!({
        "records": serde_json::to_value(&h.records).unwrap(),
        "cursor": h.cursor,
    })
}

// ------------------------------------------------------------------ fixtures

const TYPES: &[&str] = &[
    "core:note",
    "core:folder",
    "geom:cube",
    "code:file",
    "app:widget",
];
const RELS: &[&str] = &["core:contains", "core:references", "core:depends-on"];

fn fixture_snapshot(name: &str, seed: u8) -> Snapshot {
    let actor = ActorId::new("prop");
    let mut project = Project::new(name);
    let a = Object::new("core:note", format!("{name}-a-{seed}"), &actor);
    let b = Object::new("core:folder", format!("{name}-b-{seed}"), &actor);
    project.relations.insert(
        RelationId::new(),
        Relation::new("core:contains", b.id, a.id, &actor),
    );
    let aid = a.id;
    let bid = b.id;
    project.objects.insert(aid, a);
    project.objects.insert(bid, b);

    let mut history = History::new();
    history.push(TransactionRecord {
        id: worldos_kernel::ids::TransactionId::new(),
        index: 0,
        actor: actor.clone(),
        label: format!("seed-{seed}"),
        started_at: 1_700_000_000_000,
        committed_at: 1_700_000_000_001,
        commands: vec![],
        ops: vec![StateOp::SetProjectMeta {
            key: "seed".into(),
            before: None,
            after: Some(json!(seed)),
        }],
        undone: false,
    });
    Snapshot::new(project, history)
}

// ------------------------------------------------------------------ crash injection
//
// The child is this same test binary re-invoked: it writes, then
// `abort()`s before any clean-close checkpoint — a real killed process
// leaving a real WAL on disk. `WORLDOS_CRASH_MODE` selects the cut
// point; without it the worker is a trivial pass so `cargo test` stays
// green.

const MODE_ENV: &str = "WORLDOS_CRASH_MODE";
const DB_ENV: &str = "WORLDOS_CRASH_DB";

#[test]
fn crash_child_worker() {
    let Ok(mode) = std::env::var(MODE_ENV) else {
        return; // normal run — nothing to do
    };
    let path = std::env::var(DB_ENV).expect("crash db path");
    match mode.as_str() {
        // Die inside a save-shaped write: rows deleted, replacements
        // never committed — the WAL holds an uncommitted tail.
        "partial" => {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch("BEGIN IMMEDIATE").unwrap();
            conn.execute_batch(
                "DELETE FROM objects; DELETE FROM relations;
                 DELETE FROM transactions; DELETE FROM meta;",
            )
            .unwrap();
            conn.execute("INSERT INTO meta(key, value) VALUES('name', 'corpse')", [])
                .unwrap();
            std::process::abort();
        }
        // Die right after commit() returned: frames are fsync'd into
        // the WAL but the process never checkpointed or closed.
        "committed" => {
            let mut store = SqliteStore::open(&path).unwrap();
            store.save(&fixture_snapshot("after-crash", 2)).unwrap();
            std::process::abort();
        }
        _ => panic!("unknown crash mode {mode}"),
    }
}

fn spawn_crash_child(path: &Path, mode: &str) -> std::process::ExitStatus {
    std::process::Command::new(std::env::current_exe().unwrap())
        .arg("crash_child_worker")
        .arg("--exact")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(MODE_ENV, mode)
        .env(DB_ENV, path)
        .status()
        .expect("spawn crash child")
}

#[test]
fn crash_mid_write_recovers_last_committed_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crash.worldos");
    let want = fixture_snapshot("before", 1);
    {
        let mut store = SqliteStore::open(&path).unwrap();
        store.save(&want).unwrap();
    } // clean close → WAL checkpointed away

    let status = spawn_crash_child(&path, "partial");
    assert!(!status.success(), "child must die mid-write");

    let store = SqliteStore::open(&path).unwrap();
    let got = store.load().expect("prior snapshot must survive the crash");
    assert_eq!(canonical(&got.project), canonical(&want.project));
    assert_eq!(
        canonical_history(&got.history),
        canonical_history(&want.history)
    );
}

#[test]
fn committed_save_survives_process_abort() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crash.worldos");
    {
        let mut store = SqliteStore::open(&path).unwrap();
        store.save(&fixture_snapshot("before", 1)).unwrap();
    }

    let status = spawn_crash_child(&path, "committed");
    assert!(!status.success(), "child must die after commit");

    let store = SqliteStore::open(&path).unwrap();
    let got = store
        .load()
        .expect("committed state must be recovered from the WAL");
    // The committed snapshot is the child's — ids and timestamps are
    // minted in that process, so exact equality against a parent-built
    // fixture is impossible. Assert the markers that distinguish the
    // post-commit state from the pre-crash "before"/seed-1 snapshot.
    assert_eq!(got.project.name, "after-crash");
    assert_eq!(got.project.objects.len(), 2);
    assert!(
        got.project
            .objects
            .values()
            .any(|o| o.name == "after-crash-a-2"),
        "child's committed object missing after abort"
    );
    assert!(
        got.project
            .objects
            .values()
            .any(|o| o.name == "after-crash-b-2")
    );
    assert_eq!(got.project.relations.len(), 1);
    assert_eq!(got.history.records.len(), 1);
    assert_eq!(got.history.records[0].label, "seed-2");
    assert_eq!(
        serde_json::to_value(&got.history.records[0].ops[0]).unwrap(),
        serde_json::to_value(StateOp::SetProjectMeta {
            key: "seed".into(),
            before: None,
            after: Some(json!(2)),
        })
        .unwrap()
    );
}

// ------------------------------------------------------------------ corruption

/// Truncated and garbage project files must fail cleanly — a
/// `StoreError`, never a panic and never silent partial state.
#[test]
fn truncated_and_garbage_files_fail_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("good.worldos");
    {
        let mut store = SqliteStore::open(&good).unwrap();
        store.save(&fixture_snapshot("intact", 9)).unwrap();
    }
    let bytes = std::fs::read(&good).unwrap();
    assert!(!bytes.is_empty());

    for keep in [0usize, 32, 512, bytes.len() / 2] {
        let p = dir.path().join(format!("trunc-{keep}.worldos"));
        std::fs::write(&p, &bytes[..keep]).unwrap();
        assert!(
            SqliteStore::open(&p).and_then(|s| s.load()).is_err(),
            "file truncated to {keep} bytes must not load"
        );
    }
    // One byte short of whole: must not panic; SQLite decides whether
    // the tail byte was slack.
    let p = dir.path().join("trunc-minus1.worldos");
    std::fs::write(&p, &bytes[..bytes.len() - 1]).unwrap();
    let _ = SqliteStore::open(&p).and_then(|s| s.load());

    let garbage = dir.path().join("garbage.worldos");
    std::fs::write(&garbage, b"this is not a sqlite file at all").unwrap();
    assert!(SqliteStore::open(&garbage).and_then(|s| s.load()).is_err());
}

// ------------------------------------------------------------------ round-trip property

fn arb_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        any::<i64>().prop_map(|i| json!(i)),
        any::<u8>().prop_map(|b| json!(b % 2 == 0)),
        ".{0,24}".prop_map(|s| json!(s)),
        // non-finite f64 can't exist in a Value — `json!`/`From` map it
        // to Null, so this stays round-trip safe.
        any::<f64>().prop_map(|f| json!(f)),
        Just(Value::Null),
    ];
    leaf.prop_recursive(3, 32, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::btree_map(".{0,8}", inner, 0..4)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

type ObjSpec = (u8, String, Vec<String>, Vec<(u8, Value)>);

fn arb_obj_spec() -> impl Strategy<Value = ObjSpec> {
    (
        any::<u8>(),
        ".{0,12}",
        prop::collection::vec("[a-z]{1,4}", 0..4),
        prop::collection::vec((any::<u8>(), arb_json()), 0..4),
    )
}

/// One recorded mutation. Indices resolve against the generated entity
/// pools; out-of-range indices skip the op.
#[derive(Debug, Clone)]
enum OpSpec {
    CreateObj(u8),
    UpdateObj(u8, u8, Value),
    DeleteObj(u8),
    CreateRel(u8, u8, u8),
    DeleteRel(u8),
    MetaSet(u8, Value),
}

fn arb_op_spec() -> impl Strategy<Value = OpSpec> {
    prop_oneof![
        3 => any::<u8>().prop_map(OpSpec::CreateObj),
        2 => (any::<u8>(), any::<u8>(), arb_json()).prop_map(|(a, b, v)| OpSpec::UpdateObj(a, b, v)),
        2 => any::<u8>().prop_map(OpSpec::DeleteObj),
        3 => (any::<u8>(), any::<u8>(), any::<u8>()).prop_map(|(a, b, c)| OpSpec::CreateRel(a, b, c)),
        1 => any::<u8>().prop_map(OpSpec::DeleteRel),
        2 => (any::<u8>(), arb_json()).prop_map(|(k, v)| OpSpec::MetaSet(k, v)),
    ]
}

fn arb_snapshot() -> impl Strategy<Value = Snapshot> {
    (
        ".{0,16}",
        prop::collection::vec(arb_obj_spec(), 0..16),
        prop::collection::vec((any::<u8>(), any::<u8>(), any::<u8>()), 0..10),
        prop::collection::btree_map("[a-z_]{1,6}", arb_json(), 0..6),
        prop::collection::vec(
            (
                ".{0,10}",
                prop::collection::vec(arb_op_spec(), 0..6),
                prop::collection::vec((".{0,8}", arb_json(), any::<bool>()), 0..3),
            ),
            0..8,
        ),
        prop::option::of(any::<usize>()),
    )
        .prop_map(
            |(name, obj_specs, rel_specs, settings, rec_specs, cursor)| {
                let actor = ActorId::new("prop");
                let mut project = Project::new(name);
                let mut obj_ids = Vec::new();
                for (ty, oname, tags, comps) in &obj_specs {
                    let mut o = Object::new(TYPES[*ty as usize % TYPES.len()], oname, &actor);
                    o.tags = tags.clone();
                    for (cix, data) in comps {
                        o.set_component(Component::new(format!("app:c{}", cix % 4), data.clone()));
                    }
                    obj_ids.push(o.id);
                    project.objects.insert(o.id, o);
                }
                let mut rel_ids = Vec::new();
                for (rix, a, b) in &rel_specs {
                    if obj_ids.is_empty() {
                        break;
                    }
                    // deliberately allow a dangling endpoint 1/16 of the
                    // time — the store round-trips it verbatim; integrity
                    // is the validator's job, not the file format's.
                    let from = obj_ids[*a as usize % obj_ids.len()];
                    let to = if b % 16 == 15 {
                        worldos_kernel::ObjectId::new()
                    } else {
                        obj_ids[*b as usize % obj_ids.len()]
                    };
                    let r = Relation::new(RELS[*rix as usize % RELS.len()], from, to, &actor);
                    rel_ids.push(r.id);
                    project.relations.insert(r.id, r);
                }
                project.settings = settings.clone();

                let mut history = History::new();
                for (i, (label, ops, cmds)) in rec_specs.iter().enumerate() {
                    let mut rec = TransactionRecord {
                        id: worldos_kernel::ids::TransactionId::new(),
                        index: i as u64,
                        actor: actor.clone(),
                        label: label.clone(),
                        started_at: 1_700_000_000_000 + i as i64 * 1000,
                        committed_at: 1_700_000_000_000 + i as i64 * 1000 + 7,
                        commands: vec![],
                        ops: vec![],
                        undone: false,
                    };
                    for (ct, input, ok) in cmds {
                        let mut env = CommandEnvelope::new(
                            format!("prop.cmd-{ct}"),
                            actor.clone(),
                            input.clone(),
                        );
                        env.transaction_id = Some(rec.id);
                        rec.commands.push(CommandRecord {
                            envelope: env,
                            ok: *ok,
                            output: if *ok { json!({"k": 1}) } else { Value::Null },
                            error: if *ok { None } else { Some("boom".into()) },
                        });
                    }
                    for spec in ops {
                        let op = match *spec {
                            OpSpec::CreateObj(ix) => obj_specs
                                .get(ix as usize % obj_specs.len().max(1))
                                .map(|(ty, oname, tags, comps)| {
                                    let mut o = Object::new(
                                        TYPES[*ty as usize % TYPES.len()],
                                        oname,
                                        &actor,
                                    );
                                    o.tags = tags.clone();
                                    for (cix, data) in comps {
                                        o.set_component(Component::new(
                                            format!("app:c{}", cix % 4),
                                            data.clone(),
                                        ));
                                    }
                                    StateOp::object_created(o)
                                }),
                            OpSpec::UpdateObj(ix, _cix, ref v) => obj_ids
                                .get(ix as usize % obj_ids.len().max(1))
                                .and_then(|id| project.objects.get(id))
                                .map(|o| {
                                    let mut after = o.clone();
                                    after.set_component(Component::new("app:edited", v.clone()));
                                    StateOp::object_updated(o.clone(), after)
                                }),
                            OpSpec::DeleteObj(ix) => obj_ids
                                .get(ix as usize % obj_ids.len().max(1))
                                .and_then(|id| project.objects.get(id))
                                .map(|o| StateOp::object_deleted(o.clone())),
                            OpSpec::CreateRel(rix, a, b) => {
                                if obj_ids.len() < 2 {
                                    None
                                } else {
                                    let r = Relation::new(
                                        RELS[rix as usize % RELS.len()],
                                        obj_ids[a as usize % obj_ids.len()],
                                        obj_ids[b as usize % obj_ids.len()],
                                        &actor,
                                    );
                                    Some(StateOp::relation_created(r))
                                }
                            }
                            OpSpec::DeleteRel(ix) => rel_ids
                                .get(ix as usize % rel_ids.len().max(1))
                                .and_then(|rid| project.relations.get(rid))
                                .map(|r| StateOp::relation_deleted(r.clone())),
                            OpSpec::MetaSet(k, ref v) => Some(StateOp::SetProjectMeta {
                                key: format!("mk{}", k % 4),
                                before: None,
                                after: Some(v.clone()),
                            }),
                        };
                        if let Some(op) = op {
                            rec.ops.push(op);
                        }
                    }
                    history.records.push(rec);
                }
                // Undo cursor = length minus a contiguous undone suffix.
                let cursor = cursor.map(|c| c % (history.records.len() + 1)).unwrap_or(0);
                history.cursor = cursor;
                for rec in &mut history.records[cursor..] {
                    rec.undone = true;
                }
                Snapshot::new(project, history)
            },
        )
}

proptest! {
    // Bounded: each case writes a real SQLite file.
    #![proptest_config(ProptestConfig { cases: 12, ..ProptestConfig::default() })]

    /// Any snapshot — objects, relations, settings, journal with
    /// commands/ops and an undo tail — must survive save → load
    /// byte-exact in canonical form.
    #[test]
    fn snapshot_save_load_round_trip(snap in arb_snapshot()) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.worldos");
        let want_proj = canonical(&snap.project);
        let want_hist = canonical_history(&snap.history);
        {
            let mut store = SqliteStore::open(&path).unwrap();
            store.save(&snap).unwrap();
        }
        let store = SqliteStore::open(&path).unwrap();
        let got = store.load().unwrap();
        assert_eq!(canonical(&got.project), want_proj);
        assert_eq!(canonical_history(&got.history), want_hist);
    }
}
