//! Forge milestone tests: richer requirement expressions, dependency
//! tracing, and the hosted-plugin session protocol.

use serde_json::json;
use std::io::BufReader;
use worldos_capability::plugin;
use worldos_engine::Engine;
use worldos_kernel::known::{rel, types};

fn status_of(e: &Engine, name: &str) -> String {
    e.find_object(name)
        .and_then(|o| o.component_data("core:requirement-status").cloned())
        .and_then(|c| c.get("status").and_then(|s| s.as_str().map(String::from)))
        .unwrap_or_default()
}

fn dep_targets(e: &Engine, req_name: &str) -> Vec<String> {
    let req = e.find_object(req_name).unwrap();
    e.project()
        .relations_from(req.id)
        .filter(|r| r.type_id == rel::DEPENDS_ON)
        .filter_map(|r| e.project().get(r.to).map(|o| o.name.clone()))
        .collect()
}

#[test]
fn requirement_boolean_combinators() {
    let mut e = Engine::new("t");
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "cube", "name": "a"}),
    )
    .unwrap();
    e.execute(
        "requirement.create",
        json!({"name": "r-or",
               "expression": "exists_named(\"ghost\") or exists_named(\"a\")"}),
    )
    .unwrap();
    e.execute(
        "requirement.create",
        json!({"name": "r-and",
               "expression": "exists(geom:cube) and count(geom:cube) >= 1"}),
    )
    .unwrap();
    e.execute(
        "requirement.create",
        json!({"name": "r-not", "expression": "not exists(geom:sphere)"}),
    )
    .unwrap();
    e.execute(
        "requirement.create",
        json!({"name": "r-paren",
               "expression": "(exists_named(\"a\") or exists_named(\"b\")) and not exists(geom:plane)"}),
    )
    .unwrap();
    e.execute("requirement.evaluate", json!({"all": true}))
        .unwrap();
    assert_eq!(status_of(&e, "r-or"), "pass");
    assert_eq!(status_of(&e, "r-and"), "pass");
    assert_eq!(status_of(&e, "r-not"), "pass");
    assert_eq!(status_of(&e, "r-paren"), "pass");
}

#[test]
fn requirement_measure_terms_and_tracing() {
    let mut e = Engine::new("t");
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "cube", "name": "big", "size": 2.0, "position": [0,0,0]}),
    )
    .unwrap();
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "cube", "name": "small", "size": 1.0, "position": [5,0,0]}),
    )
    .unwrap();
    e.execute(
        "requirement.create",
        json!({"name": "vol",
               "expression": "volume(\"big\") >= 8 and volume(\"small\") < volume(\"big\")"}),
    )
    .unwrap();
    e.execute(
        "requirement.create",
        json!({"name": "dist", "expression": "distance(\"big\", \"small\") == 5"}),
    )
    .unwrap();
    e.execute("requirement.evaluate", json!({"all": true}))
        .unwrap();
    assert_eq!(status_of(&e, "vol"), "pass");
    assert_eq!(status_of(&e, "dist"), "pass");

    // dependency tracing: both requirements depend on both cubes
    let mut deps = dep_targets(&e, "vol");
    deps.sort();
    assert_eq!(deps, ["big", "small"]);
    let mut deps2 = dep_targets(&e, "dist");
    deps2.sort();
    assert_eq!(deps2, ["big", "small"]);

    // move `small` closer → dist fails, edges stay accurate
    e.execute(
        "geometry.transform",
        json!({"name": "small", "position": [2, 0, 0]}),
    )
    .unwrap();
    e.execute("requirement.evaluate", json!({"name": "dist"}))
        .unwrap();
    assert_eq!(status_of(&e, "dist"), "fail");
}

#[test]
fn plugin_session_commits_as_one_transaction() {
    let mut e = Engine::new("t");
    // a canned plugin script: read info, create a note, end session
    let script = concat!(
        r#"{"id":1,"method":"project.info"}"#,
        "\n",
        r#"{"id":2,"method":"command.execute","params":{"type":"object.create","input":{"type":"core:note","name":"from-plugin"}}}"#,
        "\n",
        r#"{"id":3,"method":"session.end"}"#,
        "\n",
    );
    let reader = BufReader::new(script.as_bytes());
    let mut sink = Vec::new();
    let actor = worldos_kernel::Actor::plugin("test");
    e.begin_transaction_as(&actor, "plugin:test session")
        .unwrap();
    let n = plugin::serve_stream(&mut e, &actor, reader, &mut sink).unwrap();
    e.commit_transaction().unwrap();

    assert_eq!(n, 3);
    assert!(e.find_object("from-plugin").is_some());
    // one transaction, attributed to the plugin actor
    let rec = e.history().records.last().unwrap();
    assert_eq!(rec.actor.0, "plugin:test");
    // responses were written for each request
    let responses: Vec<serde_json::Value> = String::from_utf8(sink)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(responses.len(), 3);
    assert!(responses[1]["result"]["id"].is_string());
}

#[test]
fn plugin_session_rollback_on_abort() {
    let mut e = Engine::new("t");
    // plugin creates a note then dies without session.end → host rolls back
    let script = concat!(
        r#"{"id":1,"method":"command.execute","params":{"type":"object.create","input":{"type":"core:note","name":"should-not-persist"}}}"#,
        "\n",
        // stream ends abruptly (EOF without session.end)
    );
    let reader = BufReader::new(script.as_bytes());
    let mut sink = Vec::new();
    let actor = worldos_kernel::Actor::plugin("crashy");
    e.begin_transaction_as(&actor, "plugin:crashy session")
        .unwrap();
    plugin::serve_stream(&mut e, &actor, reader, &mut sink).unwrap();
    // host decides EOF mid-session = abort
    e.rollback_transaction().unwrap();
    assert!(e.find_object("should-not-persist").is_none());
    assert_eq!(e.history().records.len(), 0);
}

#[test]
fn requirement_goes_stale_when_dependency_changes() {
    let mut e = Engine::new("t");
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "cube", "name": "big", "size": 2.0}),
    )
    .unwrap();
    e.execute(
        "requirement.create",
        json!({"name": "vol", "expression": "volume(\"big\") >= 8"}),
    )
    .unwrap();
    e.execute("requirement.evaluate", json!({"name": "vol"}))
        .unwrap();
    assert_eq!(status_of(&e, "vol"), "pass");

    // touching the depended-on object flips the requirement to stale
    // inside the same transaction — undoing the change restores `pass`.
    e.execute(
        "geometry.transform",
        json!({"name": "big", "scale": [0.5, 0.5, 0.5]}),
    )
    .unwrap();
    assert_eq!(status_of(&e, "vol"), "stale");

    e.undo().unwrap();
    assert_eq!(status_of(&e, "vol"), "pass");
    e.redo().unwrap();
    assert_eq!(status_of(&e, "vol"), "stale");
}

#[test]
fn plugin_manifest_declares_exact_permissions() {
    // manifest sidecar: <stem>.json next to the plugin file
    let dir = std::env::temp_dir().join(format!("wos-mf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let prog = dir.join("worldos-plugin-locked.py");
    std::fs::write(&prog, "pass").unwrap();
    std::fs::write(
        dir.join("worldos-plugin-locked.json"),
        r#"{"description": "locked down", "permissions": ["project.read"]}"#,
    )
    .unwrap();

    let m = plugin::manifest_for(&prog).expect("manifest loads");
    assert_eq!(m.description.as_deref(), Some("locked down"));

    let spec = plugin::PluginSpec {
        name: "locked".into(),
        program: prog.clone(),
        args: vec![],
        timeout_ms: 5000,
    };
    let actor = plugin::actor_for(&spec);
    // declared set is exact: project.read yes, command.execute no
    assert!(
        actor
            .permissions
            .is_allowed(&worldos_kernel::actor::Permission::new("project.read"))
    );
    assert!(
        !actor
            .permissions
            .is_allowed(&worldos_kernel::actor::Permission::new("command.execute"))
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plugin_actor_cannot_exceed_permissions() {
    let mut e = Engine::new("t");
    let mut plugin_actor = worldos_kernel::Actor::plugin("unprivileged");
    plugin_actor.permissions = worldos_kernel::PermissionSet::read_only();
    // the plugin session host executes commands as the plugin actor —
    // a write attempt must be refused even though the session user can write
    let res = e.execute_as(
        &plugin_actor,
        "object.create",
        json!({"type": types::NOTE, "name": "denied"}),
    );
    assert!(res.unwrap_err().to_string().contains("permission denied"));
}
