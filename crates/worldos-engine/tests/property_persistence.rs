//! Adversarial property tests — persistence invariants (NEXT.md #7).
//!
//! - random project states → save → reopen → identical semantic state:
//!   objects, relations, settings, name AND the full journal (records +
//!   undo cursor, including a persisted redo tail);
//! - reopening without saving must discard unsaved work exactly;
//! - a failed save leaves the in-memory engine and the on-disk file
//!   untouched.

mod proptest_harness;

use proptest::prelude::*;
use proptest_harness::*;
use serde_json::json;
use worldos_engine::Engine;

/// Persistence stream ops: commands plus save/reopen boundaries and
/// undo/redo, so the journal survives reopen in every cursor position.
#[derive(Debug, Clone)]
enum PersistOp {
    Cmd(Op),
    Undo,
    Redo,
    /// save → reopen → semantic state must be identical across it.
    SaveReopen,
    /// reopen WITHOUT saving — must return to the last-saved state.
    Reopen,
}

fn arb_persist() -> impl Strategy<Value = Vec<PersistOp>> {
    prop::collection::vec(
        prop_oneof![
            8 => arb_op().prop_map(PersistOp::Cmd),
            2 => Just(PersistOp::Undo),
            1 => Just(PersistOp::Redo),
            1 => Just(PersistOp::SaveReopen),
            1 => Just(PersistOp::Reopen),
        ],
        6..20,
    )
}

proptest! {
    // Bounded cases: every case pays a real `Engine::create` (SQLite
    // file + WAL + fsync), so this stays cheap on CI.
    #![proptest_config(ProptestConfig { cases: 6, ..ProptestConfig::default() })]

    #[test]
    fn save_reopen_preserves_semantic_state(ops in arb_persist()) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prop.worldos");
        let mut e = Engine::create("prop-persist", &path).unwrap();
        let initial = canonical_project(e.project());
        let mut m = Model::default();
        // What the file currently holds — starts as the empty snapshot
        // Engine::create persisted.
        let mut last_saved = (canonical_project(e.project()), canonical_history(e.history()));

        for op in &ops {
            match op {
                PersistOp::Cmd(op) => {
                    run_op(&mut e, &mut m, op, false);
                }
                PersistOp::Undo => {
                    let _ = e.undo();
                    m.refresh(&e);
                }
                PersistOp::Redo => {
                    let _ = e.redo();
                    m.refresh(&e);
                }
                PersistOp::SaveReopen => {
                    e.save().unwrap();
                    let before = (
                        canonical_project(e.project()),
                        canonical_history(e.history()),
                    );
                    e = Engine::open(&path).unwrap();
                    assert_eq!(canonical_project(e.project()), before.0, "reopened project diverged");
                    assert_eq!(
                        canonical_history(e.history()),
                        before.1,
                        "reopened history diverged (records/cursor)"
                    );
                    last_saved = before;
                    m.refresh(&e);
                }
                PersistOp::Reopen => {
                    e = Engine::open(&path).unwrap();
                    // Unsaved work is gone; the file's last snapshot is law.
                    assert_eq!(canonical_project(e.project()), last_saved.0);
                    assert_eq!(canonical_history(e.history()), last_saved.1);
                    m.refresh(&e);
                }
            }
            assert!(e.project().dangling_relations().is_empty());
        }

        // Final boundary: full save → reopen → identical semantics,
        // then undo-all on the reopened engine still reaches the
        // initial project state — history survives the file.
        e.save().unwrap();
        let want = (
            canonical_project(e.project()),
            canonical_history(e.history()),
        );
        let mut e2 = Engine::open(&path).unwrap();
        assert_eq!(canonical_project(e2.project()), want.0);
        assert_eq!(canonical_history(e2.history()), want.1);
        assert_eq!(e2.can_undo(), e.can_undo());
        assert_eq!(e2.can_redo(), e.can_redo());
        assert_replay_matches(&e2);
        undo_all(&mut e2);
        assert_eq!(canonical_project(e2.project()), initial);
    }
}

/// A failed save must not corrupt the existing file or dirty the
/// in-memory state.
#[test]
fn failed_save_leaves_engine_and_file_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ok.worldos");
    let mut e = Engine::create("t", &path).unwrap();
    e.execute("object.create", json!({"type": "core:note", "name": "a"}))
        .unwrap();
    e.save().unwrap();
    let saved = canonical_project(e.project());

    e.execute("object.create", json!({"type": "core:note", "name": "b"}))
        .unwrap();
    let with_b = canonical_project(e.project());

    // Target inside a directory that does not exist → open/write fails.
    let bad = dir.path().join("no-such-dir").join("x.worldos");
    assert!(e.save_as(&bad).is_err());
    assert_eq!(canonical_project(e.project()), with_b);
    // path must NOT have been rebound to the failed target.
    assert_eq!(e.path().unwrap(), path.as_path());

    // original file still holds exactly the pre-`b` snapshot
    let re = Engine::open(&path).unwrap();
    assert_eq!(canonical_project(re.project()), saved);
    assert!(re.find_object("b").is_none());
}

/// `save_as` to a second file + reopening the first must be independent.
#[test]
fn reopen_discards_unsaved_changes_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.worldos");
    let mut e = Engine::create("t", &path).unwrap();
    e.execute(
        "object.create",
        json!({"type": "core:note", "name": "persisted"}),
    )
    .unwrap();
    e.save().unwrap();
    e.execute(
        "object.create",
        json!({"type": "core:note", "name": "ephemeral"}),
    )
    .unwrap();
    e.undo().unwrap(); // undo ephemeral — also unsaved

    let re = Engine::open(&path).unwrap();
    assert!(re.find_object("persisted").is_some());
    assert!(re.find_object("ephemeral").is_none());
    // the unsaved create+undo of "ephemeral" never reached the file:
    // the journal holds only the first committed transaction.
    assert_eq!(re.history().records.len(), 1);
}
