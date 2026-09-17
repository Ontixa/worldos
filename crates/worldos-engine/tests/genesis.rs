//! Genesis acceptance test — the §70 scenario, exercised in-process
//! against the real engine + storage + agent path.

use serde_json::json;
use std::sync::Arc;
use worldos_agent::AgentRun;
use worldos_engine::Engine;
use worldos_kernel::known::{components, types};

fn engine_at(path: &std::path::Path) -> Engine {
    let mut e = Engine::open(path).unwrap();
    e.register_capability(Arc::new(AgentRun));
    e
}

#[test]
fn genesis_acceptance_scenario() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("genesis.worldos");

    // 1-4: create project, requirement, note, cube, rename
    {
        let mut e = Engine::create("Genesis Lab", &path).unwrap();
        e.register_capability(Arc::new(AgentRun));

        e.execute(
            "requirement.create",
            json!({"name": "sensor housing must exist",
                   "expression": "exists_named(\"sensor-housing\")"}),
        )
        .unwrap();
        e.execute(
            "document.create",
            json!({"name": "plan",
                   "text": "Build a sensor housing next to the reference cube."}),
        )
        .unwrap();
        e.execute(
            "geometry.create_primitive",
            json!({"kind": "cube", "name": "cube-1"}),
        )
        .unwrap();
        e.execute(
            "object.rename",
            json!({"object": "cube-1", "name": "reference-cube"}),
        )
        .unwrap();

        // 5-10: agent task — create cube next to reference-cube
        let report = e
            .run_capability(
                "agent.run",
                json!({"goal": "Create another cube next to reference-cube and name it sensor-housing",
                       "agent": "genesis-bot"}),
            )
            .unwrap();
        assert_eq!(report["status"], "succeeded", "{}", report["summary"]);
        assert!(e.find_object("sensor-housing").is_some());
        assert!(e.find_object("reference-cube").is_some());

        // sensor-housing sits next to reference-cube on +x
        let ref_pos = pos_of(&e, "reference-cube");
        let new_pos = pos_of(&e, "sensor-housing");
        assert!(
            new_pos[0] > ref_pos[0],
            "housing should be placed beside the reference"
        );

        // requirement evaluated → pass
        let req = e.find_object("sensor housing must exist").unwrap();
        let status = req.component_data(components::REQUIREMENT_STATUS).unwrap();
        assert_eq!(status["status"], "pass");

        // history records human + agent transactions
        let labels: Vec<&str> = e
            .history()
            .records
            .iter()
            .map(|r| r.label.as_str())
            .collect();
        assert!(labels.iter().any(|l| l.contains("agent:genesis-bot")));
        let agent_txn = e
            .history()
            .records
            .iter()
            .find(|r| r.label.contains("agent:"))
            .unwrap();
        assert_eq!(agent_txn.actor.0, "agent:genesis-bot");
        assert!(agent_txn.commands.len() >= 3);

        // 11-14: save, drop, reopen, verify
        e.save().unwrap();
    }

    {
        let mut e = engine_at(&path);
        assert!(e.find_object("sensor-housing").is_some());
        assert!(e.find_object("reference-cube").is_some());
        assert_eq!(e.history().records.len(), 5, "history must persist");

        // 15: validation passes
        assert!(e.validate().passed, "{:?}", e.validate().diagnostics);

        // 21: undo the agent transaction — the whole thing reverts
        let before = e.project().objects.len();
        let undone = e.undo().unwrap().expect("undo must work");
        let rec = e.history().records.iter().find(|r| r.id == undone).unwrap();
        assert!(rec.label.contains("agent:"));
        assert!(
            e.find_object("sensor-housing").is_none(),
            "agent cube reverted"
        );
        assert!(
            e.find_object("reference-cube").is_some(),
            "reference survives"
        );
        assert!(e.project().objects.len() < before);

        // redo restores it
        e.redo().unwrap();
        assert!(e.find_object("sensor-housing").is_some());
    }
}

fn pos_of(e: &Engine, name: &str) -> [f64; 3] {
    let o = e.find_object(name).unwrap();
    let p = &o.component_data(components::TRANSFORM).unwrap()["position"];
    [
        p[0].as_f64().unwrap(),
        p[1].as_f64().unwrap(),
        p[2].as_f64().unwrap(),
    ]
}

#[test]
fn transaction_rollback_keeps_state_clean() {
    let mut e = Engine::new("t");
    e.execute("object.create", json!({"type": types::NOTE, "name": "a"}))
        .unwrap();
    e.begin_transaction("multi").unwrap();
    e.execute("object.create", json!({"type": types::NOTE, "name": "b"}))
        .unwrap();
    e.execute("object.create", json!({"type": types::NOTE, "name": "c"}))
        .unwrap();
    e.rollback_transaction().unwrap();
    assert_eq!(e.project().objects.len(), 1);
    assert!(e.find_object("a").is_some());
}

#[test]
fn failed_command_is_atomic() {
    let mut e = Engine::new("t");
    // object.delete on missing object fails after resolving — nothing corrupt
    assert!(
        e.execute("object.delete", json!({"name": "ghost"}))
            .is_err()
    );
    assert_eq!(e.project().objects.len(), 0);
    assert_eq!(
        e.history().records.len(),
        0,
        "failed command leaves no record"
    );
}

#[test]
fn undo_redo_round_trip() {
    let mut e = Engine::new("t");
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "cube", "name": "a"}),
    )
    .unwrap();
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "sphere", "name": "b"}),
    )
    .unwrap();
    assert_eq!(e.project().objects.len(), 2);
    e.undo().unwrap();
    assert!(e.find_object("b").is_none());
    e.undo().unwrap();
    assert_eq!(e.project().objects.len(), 0);
    e.redo().unwrap();
    e.redo().unwrap();
    assert!(e.find_object("a").is_some() && e.find_object("b").is_some());
}

#[test]
fn new_write_invalidates_redo() {
    let mut e = Engine::new("t");
    e.execute("object.create", json!({"type": types::NOTE, "name": "a"}))
        .unwrap();
    e.execute("object.create", json!({"type": types::NOTE, "name": "b"}))
        .unwrap();
    e.undo().unwrap(); // remove b
    assert!(!e.can_redo() || e.history().records.iter().any(|r| r.undone));
    // a fresh write must truncate the redo tail
    e.execute("object.create", json!({"type": types::NOTE, "name": "c"}))
        .unwrap();
    assert!(!e.can_redo(), "redo tail must be truncated by new write");
    e.redo().unwrap();
    assert!(e.find_object("b").is_none(), "b stays gone");
    assert!(e.find_object("c").is_some());
}

#[test]
fn permissions_are_enforced() {
    let mut e = Engine::new("t");
    let mut reader = worldos_kernel::Actor::human("reader");
    reader.permissions = worldos_kernel::PermissionSet::read_only();
    let res = e.execute_as(
        &reader,
        "object.create",
        json!({"type": types::NOTE, "name": "nope"}),
    );
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("permission denied"));
}
