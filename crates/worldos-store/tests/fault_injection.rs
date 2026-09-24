//! In-process I/O-fault injection tests (NEXT.md #7):
//!
//! `crash_recovery.rs` kills a real child process around commits; this
//! suite injects deterministic `Write`/read/sync-style failures *inside*
//! the store through the `fault-injection` seam and proves the recovery
//! classification at each real boundary:
//!
//! - a fault anywhere inside the save transaction (wipe, staged rows,
//!   journal append + index update, commit boundary) → the error
//!   propagates, the transaction rolls back, and the prior committed
//!   snapshot survives byte-exact — the file stays `integrity_check`
//!   clean;
//! - a fault AFTER commit (lost ack / post-commit sync failure) → the
//!   caller sees an error but the NEW snapshot is durable — an error
//!   does not mean "not saved";
//! - a fault mid-load → `StoreError`, never a panic or a partial
//!   snapshot, and the store stays usable;
//! - `inject_torn_save` commits the wipe phase without a transaction →
//!   a durable torn file that fails closed (`NotFound`) or loads as a
//!   hollow-but-valid project — never silently half-old/half-new;
//! - armed faults are one-shot: fire once, then the store recovers.
//!
//! Page-level tears below the transaction layer remain SQLite's own
//! atomicity guarantee (see `docs/engineering/LIMITATIONS.md`).

use proptest::prelude::*;
use serde_json::{Value, json};
use std::path::Path;
use worldos_commands::{History, TransactionRecord};
use worldos_kernel::delta::StateOp;
use worldos_kernel::ids::RelationId;
use worldos_kernel::{ActorId, Object, Project, Relation};
use worldos_store::{FaultPoint, ProjectStore, Snapshot, SqliteStore, StoreError};

// ------------------------------------------------------------------ canonical form

/// Order-independent serialization — identical to crash_recovery.rs so
/// both suites assert the same equality contract.
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

fn fixture_snapshot(name: &str, seed: u8) -> Snapshot {
    let actor = ActorId::new("fault");
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

/// Seed a file with `snap` via a clean open→save→close.
fn seed_file(path: &Path, snap: &Snapshot) {
    let mut store = SqliteStore::open(path).unwrap();
    store.save(snap).unwrap();
}

/// SQLite-level integrity of the file on disk — proves injected faults
/// never leave a physically corrupt database (indexes included).
fn integrity_check(path: &Path) -> String {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.pragma_query_value(None, "integrity_check", |r| r.get::<_, String>(0))
        .unwrap()
}

fn assert_injected_io(err: &StoreError) {
    match err {
        StoreError::Io(e) => assert!(
            e.to_string().contains("injected"),
            "expected injected fault, got {e}"
        ),
        other => panic!("expected StoreError::Io, got {other}"),
    }
}

// ------------------------------------------------------------------ save-boundary classification

/// Every fault point inside the save transaction: a failure here must
/// roll the whole save back — the commit boundary is the only place
/// the classification flips.
const PRE_COMMIT_POINTS: &[FaultPoint] = &[
    FaultPoint::SaveAfterWipe,
    FaultPoint::SaveAfterMeta,
    FaultPoint::SaveAfterObjects,
    FaultPoint::SaveAfterRelations,
    FaultPoint::SaveAfterJournal,
    FaultPoint::SaveBeforeCommit,
];

/// Injected write/sync failure at ANY point inside the save
/// transaction → `save` errors, the transaction rolls back, and the
/// previously committed snapshot survives byte-exact. Index updates
/// and the journal append share the transaction, so
/// `integrity_check` must still be clean.
#[test]
fn fault_inside_save_transaction_preserves_prior_snapshot() {
    for &point in PRE_COMMIT_POINTS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("{point:?}.worldos"));
        let before = fixture_snapshot("before", 1);
        seed_file(&path, &before);

        {
            let mut store = SqliteStore::open(&path).unwrap();
            store.inject_fault(point);
            let err = store
                .save(&fixture_snapshot("after", 2))
                .expect_err("armed fault must surface as an error");
            assert_injected_io(&err);
            assert_eq!(store.faults_armed(), 0, "fault must have fired once");
        }

        let store = SqliteStore::open(&path).unwrap();
        let got = store
            .load()
            .expect("prior committed snapshot must survive a mid-save fault");
        assert_eq!(
            canonical(&got.project),
            canonical(&before.project),
            "mid-save fault at {point:?} tore the snapshot"
        );
        assert_eq!(
            canonical_history(&got.history),
            canonical_history(&before.history),
            "mid-save fault at {point:?} lost the journal"
        );
        assert_eq!(integrity_check(&path), "ok", "file corrupt after {point:?}");
    }
}

/// The commit boundary flips the classification: a fault AFTER
/// `commit()` returned — a lost ack or post-commit sync failure — is
/// reported to the caller as an error even though the effect IS
/// durable. Reopen must find the NEW snapshot; callers that retry on
/// error must reconcile (e.g. re-load) rather than assume "not saved".
#[test]
fn fault_after_commit_reports_error_but_snapshot_is_durable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lost-ack.worldos");
    seed_file(&path, &fixture_snapshot("before", 1));
    let after = fixture_snapshot("after", 2);

    {
        let mut store = SqliteStore::open(&path).unwrap();
        store.inject_fault(FaultPoint::SaveAfterCommit);
        let err = store
            .save(&after)
            .expect_err("post-commit fault must surface as an error");
        assert_injected_io(&err);
        assert_eq!(store.faults_armed(), 0);
    }

    let store = SqliteStore::open(&path).unwrap();
    let got = store.load().unwrap();
    assert_eq!(
        canonical(&got.project),
        canonical(&after.project),
        "committed snapshot must be durable despite the reported error"
    );
    assert_eq!(
        canonical_history(&got.history),
        canonical_history(&after.history)
    );
    assert_eq!(integrity_check(&path), "ok");
}

// ------------------------------------------------------------------ load-boundary classification

const LOAD_POINTS: &[FaultPoint] = &[
    FaultPoint::LoadAfterMeta,
    FaultPoint::LoadObjects,
    FaultPoint::LoadRelations,
    FaultPoint::LoadJournal,
];

/// Injected read failure anywhere in the load path → `StoreError`,
/// never a panic and never a partial snapshot. The store stays usable:
/// once the (one-shot) fault has fired, the same `load` succeeds.
#[test]
fn fault_mid_load_fails_cleanly_and_store_stays_usable() {
    for &point in LOAD_POINTS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("{point:?}.worldos"));
        let before = fixture_snapshot("intact", 7);
        seed_file(&path, &before);

        let mut store = SqliteStore::open(&path).unwrap();
        store.inject_fault(point);
        let err = store
            .load()
            .expect_err("armed fault must surface as an error");
        assert_injected_io(&err);
        assert_eq!(store.faults_armed(), 0, "fault must have fired once");
        assert!(
            store.exists(),
            "project rows must be untouched by a failed read"
        );

        // One-shot disarm → same connection recovers with intact data.
        let got = store.load().unwrap();
        assert_eq!(canonical(&got.project), canonical(&before.project));
        assert_eq!(
            canonical_history(&got.history),
            canonical_history(&before.history)
        );
    }
}

// ------------------------------------------------------------------ torn writes

/// `inject_torn_save` commits the wipe phase statement-by-statement in
/// autocommit — a durable torn write on disk. Wiping `meta` too makes
/// the project vanish: reopen must classify the file as absent
/// (`NotFound`), never as a partial load. The file itself stays a
/// valid, integrity-clean SQLite database — the tear is logical.
#[test]
fn torn_save_full_wipe_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("torn.worldos");
    seed_file(&path, &fixture_snapshot("before", 1));

    {
        let mut store = SqliteStore::open(&path).unwrap();
        let err = store
            .inject_torn_save(usize::MAX)
            .expect_err("torn save must report failure");
        assert_injected_io(&err);
    }

    let store = SqliteStore::open(&path).unwrap();
    assert!(
        !store.exists(),
        "fully wiped meta must not report a live project"
    );
    let err = store.load().expect_err("torn file must not load");
    assert!(
        matches!(err, StoreError::NotFound(_)),
        "expected NotFound, got {err}"
    );
    assert_eq!(integrity_check(&path), "ok");
}

/// A partial wipe keeps `meta` but loses row data. Honest
/// classification: the file "exists" and loads — but as a HOLLOW
/// project (meta + journal, no objects/relations), never a mixed
/// half-old/half-new state, and never silently resurrecting deleted
/// rows. Row loss is indistinguishable from "deliberately empty" at
/// this layer; the journal rows that survive are verbatim.
#[test]
fn torn_save_partial_wipe_loads_hollow_not_half_old() {
    let dir = tempfile::tempdir().unwrap();

    // n=1: only `objects` wiped — relations/journal/meta survive.
    // The dangling relation round-trips verbatim: referential integrity
    // is the validator's job, not the file format's.
    let p1 = dir.path().join("tear-1.worldos");
    let before = fixture_snapshot("before", 1);
    seed_file(&p1, &before);
    {
        let mut store = SqliteStore::open(&p1).unwrap();
        assert_injected_io(&store.inject_torn_save(1).unwrap_err());
    }
    let got = SqliteStore::open(&p1).unwrap().load().unwrap();
    assert_eq!(got.project.name, before.project.name);
    assert_eq!(got.project.objects.len(), 0, "wiped objects must stay gone");
    assert_eq!(
        got.project.relations.len(),
        1,
        "dangling relation survives verbatim"
    );
    assert_eq!(got.history.records.len(), 1);
    assert_eq!(integrity_check(&p1), "ok");

    // n=2: objects + relations wiped — meta and journal survive.
    let p2 = dir.path().join("tear-2.worldos");
    seed_file(&p2, &before);
    {
        let mut store = SqliteStore::open(&p2).unwrap();
        assert_injected_io(&store.inject_torn_save(2).unwrap_err());
    }
    let got = SqliteStore::open(&p2).unwrap().load().unwrap();
    assert_eq!(got.project.objects.len(), 0);
    assert_eq!(got.project.relations.len(), 0);
    assert_eq!(got.history.records.len(), 1, "journal not yet wiped");
    assert_eq!(integrity_check(&p2), "ok");

    // n=3: every row table wiped, meta intact — fully hollow project.
    let p3 = dir.path().join("tear-3.worldos");
    seed_file(&p3, &before);
    {
        let mut store = SqliteStore::open(&p3).unwrap();
        assert_injected_io(&store.inject_torn_save(3).unwrap_err());
    }
    let got = SqliteStore::open(&p3).unwrap().load().unwrap();
    assert_eq!(got.project.objects.len(), 0);
    assert_eq!(got.project.relations.len(), 0);
    assert_eq!(got.history.records.len(), 0);
    assert_eq!(got.project.name, before.project.name);
    assert_eq!(integrity_check(&p3), "ok");
}

// ------------------------------------------------------------------ injector semantics

/// Armed faults are one-shot: fire once, then the store recovers —
/// a retried save lands normally and a cleared fault never fires.
#[test]
fn armed_fault_fires_once_then_store_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("one-shot.worldos");
    seed_file(&path, &fixture_snapshot("before", 1));
    let after = fixture_snapshot("after", 2);

    let mut store = SqliteStore::open(&path).unwrap();
    store.inject_fault(FaultPoint::SaveBeforeCommit);
    assert_injected_io(&store.save(&after).unwrap_err());
    assert_eq!(store.faults_armed(), 0, "fault must disarm on fire");

    // Retry without re-arming → lands; reopen proves it.
    store.save(&after).unwrap();
    let got = SqliteStore::open(&path).unwrap().load().unwrap();
    assert_eq!(canonical(&got.project), canonical(&after.project));

    // clear_faults disarms without firing.
    store.inject_faults([FaultPoint::SaveAfterWipe, FaultPoint::SaveBeforeCommit]);
    assert_eq!(store.faults_armed(), 2);
    store.clear_faults();
    assert_eq!(store.faults_armed(), 0);
    store.save(&fixture_snapshot("third", 3)).unwrap();
}

// ------------------------------------------------------------------ property

fn arb_pre_commit_point() -> impl Strategy<Value = FaultPoint> {
    (0..PRE_COMMIT_POINTS.len()).prop_map(|i| PRE_COMMIT_POINTS[i])
}

proptest! {
    // Bounded: each case writes a real SQLite file (fsync-priced).
    #![proptest_config(ProptestConfig { cases: 8, ..ProptestConfig::default() })]

    /// For ANY snapshot written on top of a committed one: an injected
    /// fault anywhere before the commit boundary must leave the prior
    /// snapshot byte-exact — recovery classification is independent of
    /// payload and of which staged write phase died.
    #[test]
    fn pre_commit_fault_never_produces_torn_snapshot(
        seed_before in any::<u8>(),
        seed_after in any::<u8>(),
        point in arb_pre_commit_point(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prop.worldos");
        let before = fixture_snapshot("prop", seed_before);
        seed_file(&path, &before);

        {
            let mut store = SqliteStore::open(&path).unwrap();
            store.inject_fault(point);
            prop_assert!(store
                .save(&fixture_snapshot("prop-after", seed_after))
                .is_err());
        }

        let got = SqliteStore::open(&path).unwrap().load().unwrap();
        prop_assert_eq!(canonical(&got.project), canonical(&before.project));
        prop_assert_eq!(
            canonical_history(&got.history),
            canonical_history(&before.history)
        );
        prop_assert_eq!(integrity_check(&path), "ok".to_string());
    }
}
