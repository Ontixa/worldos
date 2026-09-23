//! Adversarial property tests — state machine invariants (NEXT.md #7).
//!
//! - random valid command sequences → undo all → state equals initial;
//!   redo all → state equals final;
//! - undo/redo interleaved with new writes (redo-tail truncation) keep
//!   graph integrity and replay-consistency;
//! - transactions that fail validation, permission, or mid-handler leave
//!   zero observable residue on graph and journal — checked on every
//!   op, not just in dedicated cases;
//! - explicit transaction lifecycle: begin/commit/rollback interleaved
//!   with commands and undo/redo attempts.

mod proptest_harness;

use proptest::prelude::*;
use proptest_harness::*;
use serde_json::json;
use worldos_engine::Engine;

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// The core undo contract: a random mix of every builtin mutation
    /// domain (objects, documents, code files, geometry, requirements,
    /// decisions, relations, project meta) plus deliberately failing
    /// calls. Undoing all committed transactions must reproduce the
    /// initial state exactly — including object meta and project
    /// settings — and redoing must reproduce the final state.
    #[test]
    fn undo_all_recovers_initial_and_redo_all_recovers_final(ops in arb_ops()) {
        let mut e = Engine::new("prop-state");
        let initial = canonical_project(e.project());
        let mut m = Model::default();

        for op in &ops {
            run_op(&mut e, &mut m, op, false);
        }
        let final_state = canonical_project(e.project());
        let final_recs = e.history().records.len();

        undo_all(&mut e);
        assert_eq!(
            canonical_project(e.project()),
            initial,
            "undo all did not recover the initial state"
        );
        // undo never destroys the journal — every record survives undone.
        assert_eq!(e.history().records.len(), final_recs);
        assert_eq!(e.history().cursor, 0);

        redo_all(&mut e);
        assert_eq!(
            canonical_project(e.project()),
            final_state,
            "redo all did not recover the final state"
        );
        assert_eq!(e.history().cursor, final_recs);
    }

    /// Undo/redo mixed into the command stream: each undo moves the
    /// cursor back and each subsequent write truncates the redo tail.
    /// After every step the replayed ops must equal live state; at the
    /// end, undoing everything still reaches the initial state because
    /// truncated records were already reverted.
    #[test]
    fn interleaved_undo_redo_keeps_integrity(ops in arb_ops(), moves in prop::collection::vec(any::<u8>(), 8..48)) {
        let mut e = Engine::new("prop-interleave");
        let initial = canonical_project(e.project());
        let mut m = Model::default();

        // Interleave ops and undo/redo moves roughly 1:1.
        let n = ops.len().max(moves.len());
        for i in 0..n {
            if let Some(op) = ops.get(i) {
                run_op(&mut e, &mut m, op, false);
            }
            if let Some(mv) = moves.get(i) {
                match mv % 4 {
                    0 => {
                        let _ = e.undo();
                    }
                    1 => {
                        let _ = e.redo();
                    }
                    _ => {}
                }
                m.refresh(&e);
                assert!(e.project().dangling_relations().is_empty());
                assert_replay_matches(&e);
            }
        }

        undo_all(&mut e);
        assert_eq!(
            canonical_project(e.project()),
            initial,
            "undo all after interleaved moves did not recover initial state"
        );
        assert_replay_matches(&e);
        redo_all(&mut e);
        assert_replay_matches(&e);
    }

    /// Transaction lifecycle adversarial: random begin/commit/rollback
    /// interleaved with commands and undo/redo. A second begin while
    /// open must fail; commit/rollback with none open must fail;
    /// undo/redo while open must fail; rollback must restore the exact
    /// pre-transaction state; a failed command inside a transaction
    /// must not leak ops into a later commit.
    #[test]
    fn transaction_lifecycle(ops in prop::collection::vec(prop_oneof![
        3 => Just(TxOp::Begin),
        3 => Just(TxOp::Commit),
        2 => Just(TxOp::Rollback),
        8 => arb_op().prop_map(TxOp::Cmd),
        1 => Just(TxOp::Undo),
        1 => Just(TxOp::Redo),
    ], 6..40)) {
        let mut e = Engine::new("prop-txn");
        let initial = canonical_project(e.project());
        let mut m = Model::default();
        let mut in_txn = false;
        let mut txn_touched = false;
        let mut pre_txn_state = initial.clone();
        let mut pre_txn_recs = 0usize;
        let mut pre_txn_cursor = 0usize;

        for txop in &ops {
            let recs = e.history().records.len();
            let cur = e.history().cursor;
            match txop {
                TxOp::Begin => {
                    if in_txn {
                        assert!(
                            e.begin_transaction("prop").is_err(),
                            "second begin must be rejected"
                        );
                    } else {
                        e.begin_transaction("prop").unwrap();
                        in_txn = true;
                        txn_touched = false;
                        pre_txn_state = canonical_project(e.project());
                        pre_txn_recs = recs;
                        pre_txn_cursor = cur;
                    }
                }
                TxOp::Commit => {
                    if !in_txn {
                        assert!(e.commit_transaction().is_err());
                    } else {
                        e.commit_transaction().unwrap();
                        in_txn = false;
                        if txn_touched {
                            // push lands at the cursor, truncating any
                            // redo tail that was live before the txn.
                            assert_eq!(e.history().records.len(), cur + 1);
                            assert_eq!(e.history().cursor, cur + 1);
                        } else {
                            // an untouched transaction leaves no record.
                            assert_eq!(e.history().records.len(), recs);
                        }
                    }
                }
                TxOp::Rollback => {
                    if !in_txn {
                        assert!(e.rollback_transaction().is_err());
                    } else {
                        e.rollback_transaction().unwrap();
                        in_txn = false;
                        assert_eq!(
                            canonical_project(e.project()),
                            pre_txn_state,
                            "rollback did not restore pre-transaction state"
                        );
                        // rollback never touches the journal — including
                        // a redo tail that predates the transaction.
                        assert_eq!(e.history().records.len(), pre_txn_recs);
                        assert_eq!(e.history().cursor, pre_txn_cursor);
                        m.refresh(&e);
                    }
                }
                TxOp::Cmd(op) => {
                    let outcome = run_op(&mut e, &mut m, op, in_txn);
                    // A command "touches" the open transaction only if
                    // it got far enough to leave a record: success or
                    // a mid-handler failure. Pre-transaction rejections
                    // (bad schema, denied actor, unknown command) leave
                    // the txn empty, and committing it is a no-op.
                    if in_txn
                        && matches!(
                            outcome,
                            Outcome::Committed | Outcome::FailedRecorded
                        )
                    {
                        txn_touched = true;
                    }
                }
                TxOp::Undo => {
                    if in_txn {
                        assert!(e.undo().is_err(), "undo inside an open txn must fail");
                    } else {
                        let _ = e.undo();
                        m.refresh(&e);
                    }
                }
                TxOp::Redo => {
                    if in_txn {
                        assert!(e.redo().is_err(), "redo inside an open txn must fail");
                    } else {
                        let _ = e.redo();
                        m.refresh(&e);
                    }
                }
            }
            assert!(e.project().dangling_relations().is_empty());
        }

        if in_txn {
            e.rollback_transaction().unwrap();
            assert_eq!(canonical_project(e.project()), pre_txn_state);
        }
        assert_replay_matches(&e);
        undo_all(&mut e);
        assert_eq!(canonical_project(e.project()), initial);
    }
}

/// Extra step kind for the transaction test: either an engine-level
/// transaction control or a command.
#[derive(Debug, Clone)]
pub enum TxOp {
    Begin,
    Commit,
    Rollback,
    Cmd(Op),
    Undo,
    Redo,
}

// ------------------------------------------------- deterministic regressions

/// `project.rename` writes `project.name` — a top-level field, not a
/// settings entry. Undo/redo must restore it, and no `__name` sentinel
/// may leak into `settings`.
#[test]
fn project_rename_undoes_and_redoes_exactly() {
    let mut e = Engine::new("orig");
    e.execute("project.rename", json!({"name": "renamed"}))
        .unwrap();
    assert_eq!(e.project().name, "renamed");
    e.undo().unwrap();
    assert_eq!(e.project().name, "orig");
    assert!(
        !e.project().settings.contains_key("__name"),
        "rename sentinel leaked into settings"
    );
    e.redo().unwrap();
    assert_eq!(e.project().name, "renamed");
    assert!(!e.project().settings.contains_key("__name"));
    e.undo().unwrap();
    assert_eq!(e.project().name, "orig");
}

/// A command that fails mid-handler inside a caller-managed
/// transaction must have its partial ops reverted: a later commit may
/// keep the attempt in the journal (`ok:false` record) but no graph
/// residue, and undo of the transaction removes only the good effects.
#[test]
fn failed_command_inside_explicit_transaction_leaves_no_residue() {
    let mut e = Engine::new("t");
    e.begin_transaction("batch").unwrap();
    e.execute(
        "object.create",
        json!({"type": "core:note", "name": "kept"}),
    )
    .unwrap();
    // object.create inserts the object before resolving `parent` —
    // this fails after the first op was already applied.
    let err = e
        .execute(
            "object.create",
            json!({"type": "core:note", "name": "ghost-child", "parent": "ghost-zzz"}),
        )
        .unwrap_err();
    assert!(err.to_string().contains("ghost-zzz") || err.to_string().contains("not found"));
    e.commit_transaction().unwrap();

    assert!(e.find_object("kept").is_some());
    assert!(
        e.find_object("ghost-child").is_none(),
        "failed command committed graph residue"
    );
    assert!(e.project().dangling_relations().is_empty());

    let rec = e.history().records.last().unwrap();
    assert_eq!(rec.commands.len(), 2);
    assert!(rec.commands[0].ok);
    assert!(
        !rec.commands[1].ok,
        "the failed attempt stays in the journal"
    );

    e.undo().unwrap();
    assert!(e.find_object("kept").is_none());
    e.redo().unwrap();
    assert!(e.find_object("kept").is_some());
    assert!(e.find_object("ghost-child").is_none());
}

/// `object.delete cascade` walks `core:contains` edges. A containment
/// cycle must terminate, not loop forever.
#[test]
fn contains_cycle_cascade_delete_terminates() {
    let mut e = Engine::new("t");
    e.execute("object.create", json!({"type": "core:folder", "name": "a"}))
        .unwrap();
    e.execute("object.create", json!({"type": "core:folder", "name": "b"}))
        .unwrap();
    e.execute(
        "relation.add",
        json!({"type": "core:contains", "from": "a", "to": "b"}),
    )
    .unwrap();
    e.execute(
        "relation.add",
        json!({"type": "core:contains", "from": "b", "to": "a"}),
    )
    .unwrap();
    e.execute("object.delete", json!({"name": "a", "cascade": true}))
        .unwrap();
    assert!(e.find_object("a").is_none());
    assert!(e.find_object("b").is_none());
    assert!(e.project().relations.is_empty());

    // self-loop
    e.execute("object.create", json!({"type": "core:folder", "name": "s"}))
        .unwrap();
    e.execute(
        "relation.add",
        json!({"type": "core:contains", "from": "s", "to": "s"}),
    )
    .unwrap();
    e.execute("object.delete", json!({"name": "s", "cascade": true}))
        .unwrap();
    assert!(e.find_object("s").is_none());
}
