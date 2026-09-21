//! Planner tests: LLM planning over a mock provider (no network),
//! self-repair on malformed output, unknown-command rejection, and the
//! fallback chain to rules.

use serde_json::json;
use worldos_agent::provider::ProviderError;
use worldos_agent::{
    AgentRuntime, FallbackPlanner, LlmPlanner, ModelProvider, Planner, RulePlanner, RunStatus,
};
use worldos_engine::Engine;

/// Scriptable mock provider — returns queued responses in order.
struct MockProvider {
    responses: std::sync::Mutex<VecDeque<String>>,
}
use std::collections::VecDeque;

impl MockProvider {
    fn of(responses: &[&str]) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses.iter().map(|s| s.to_string()).collect()),
        }
    }
}

impl ModelProvider for MockProvider {
    fn id(&self) -> &str {
        "mock"
    }
    fn complete(&self, _prompt: &str) -> Result<String, ProviderError> {
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| ProviderError::NotConfigured("no more responses".into()))
    }
}

#[test]
fn llm_planner_parses_valid_plan() {
    let e = Engine::new("t");
    let provider = MockProvider::of(&[r#"[
        {"command": "geometry.create_primitive", "input": {"kind": "cube", "name": "llm-cube"}, "note": "make cube"}
    ]"#]);
    let planner = LlmPlanner::new(provider);
    let steps = planner.plan("create a cube named llm-cube", &e).unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].command, "geometry.create_primitive");
    assert_eq!(steps[0].input["name"], "llm-cube");
}

#[test]
fn llm_planner_self_repairs_bad_json() {
    let e = Engine::new("t");
    // first response is prose, second is a valid plan
    let provider = MockProvider::of(&[
        "Sure! I'll create a cube for you.",
        r#"[{"command": "geometry.create_primitive", "input": {"kind": "cube"}, "note": "ok"}]"#,
    ]);
    let planner = LlmPlanner::new(provider);
    let steps = planner.plan("create a cube", &e).unwrap();
    assert_eq!(steps.len(), 1);
}

#[test]
fn llm_planner_rejects_unknown_commands() {
    let e = Engine::new("t");
    let provider = MockProvider::of(&[
        r#"[{"command": "system.delete_everything", "input": {}, "note": "evil"}]"#,
    ]);
    let planner = LlmPlanner::new(provider);
    let err = planner.plan("do something", &e).unwrap_err();
    assert!(err.to_string().contains("unknown command"), "{err}");
}

#[test]
fn fallback_chain_uses_rules_when_llm_fails() {
    let e = Engine::new("t");
    // provider that always errors → FallbackPlanner must reach RulePlanner
    struct Dead;
    impl ModelProvider for Dead {
        fn id(&self) -> &str {
            "dead"
        }
        fn complete(&self, _p: &str) -> Result<String, ProviderError> {
            Err(ProviderError::Request("offline".into()))
        }
    }
    let planner =
        FallbackPlanner::new(vec![Box::new(LlmPlanner::new(Dead)), Box::new(RulePlanner)]);
    let steps = planner.plan("create a cube named x", &e).unwrap();
    assert_eq!(steps[0].command, "geometry.create_primitive");
}

#[test]
fn llm_plan_executes_end_to_end() {
    let mut e = Engine::new("t");
    let provider = MockProvider::of(&[r#"[
        {"command": "geometry.create_primitive", "input": {"kind": "sphere", "name": "llm-ball"}, "note": "ball"},
        {"command": "geometry.transform", "input": {"name": "llm-ball", "position": [3,0,0]}, "note": "move"}
    ]"#]);
    let rt = AgentRuntime::new(Box::new(LlmPlanner::new(provider)));
    let report = rt.run(&mut e, "make a sphere called llm-ball at x=3", "llm-bot");
    assert_eq!(report.status, RunStatus::Succeeded, "{}", report.summary);
    assert!(!report.verification.is_empty());
    let ball = e.find_object("llm-ball").expect("ball created");
    assert_eq!(ball.type_id.0, "geom:sphere");
    assert_eq!(e.history().records.last().unwrap().actor.0, "agent:llm-bot");
}

/// A command that really mutates the engine but reports an incorrect reference.
struct IncorrectReference(serde_json::Value);

impl worldos_commands::CommandHandler for IncorrectReference {
    fn schema(&self) -> worldos_commands::CommandSchema {
        worldos_commands::CommandSchema::write(
            "test.incorrect_reference",
            "test",
            "Create an object but report an incorrect reference",
            json!({"type": "object"}),
        )
    }

    fn execute(
        &self,
        ctx: &mut worldos_commands::CommandContext,
        _input: &serde_json::Value,
    ) -> Result<serde_json::Value, worldos_commands::CommandError> {
        ctx.run_sub(
            "object.create",
            json!({"type": "core:note", "name": "unverified"}),
        )?;
        Ok(json!({"id": self.0}))
    }
}

#[test]
fn invalid_reported_references_fail_before_commit_and_preserve_evidence() {
    for (id, expected) in [
        (
            json!(worldos_kernel::ObjectId::new().to_string()),
            "does not exist",
        ),
        (json!("not-an-object-id"), "invalid object id"),
        (json!(42), "not a string"),
    ] {
        let mut e = Engine::new("t");
        let before = serde_json::to_value(e.project()).unwrap();
        let history_before = e.history().records.len();
        e.register_command(std::sync::Arc::new(IncorrectReference(id.clone())));
        let provider =
            MockProvider::of(&[r#"[{"command":"test.incorrect_reference","input":{}}]"#]);
        let rt = AgentRuntime::new(Box::new(LlmPlanner::new(provider)));
        let report = rt.run(&mut e, "create a note", "t");

        assert_eq!(report.status, RunStatus::Failed);
        assert!(
            report.summary.contains("verification failed"),
            "{}",
            report.summary
        );
        assert!(report.summary.contains(expected), "{}", report.summary);
        assert_eq!(report.steps.len(), 1);
        assert!(
            report.steps[0].ok,
            "command success is distinct from verification"
        );
        assert_eq!(report.steps[0].output["id"], id);
        assert!(report.verification.is_empty());
        assert!(report.created_objects.is_empty());
        assert_eq!(serde_json::to_value(e.project()).unwrap(), before);
        assert_eq!(e.history().records.len(), history_before);
        assert!(!worldos_capability::CapabilityHost::in_transaction(&e));
    }
}

#[test]
fn reference_removed_by_later_step_fails_atomically() {
    let mut e = Engine::new("t");
    let before = serde_json::to_value(e.project()).unwrap();
    let provider = MockProvider::of(&[r#"[
        {"command":"object.create","input":{"type":"core:note","name":"temporary"}},
        {"command":"object.delete","input":{"id":"$0.id"}}
    ]"#]);
    let rt = AgentRuntime::new(Box::new(LlmPlanner::new(provider)));
    let report = rt.run(&mut e, "create then delete a note", "t");
    assert_eq!(report.status, RunStatus::Failed);
    assert!(
        report.summary.contains("does not exist"),
        "{}",
        report.summary
    );
    assert!(report.steps.iter().all(|step| step.ok));
    assert_eq!(serde_json::to_value(e.project()).unwrap(), before);
    assert!(e.history().records.is_empty());
}

#[test]
fn relation_output_is_verified_as_a_relation_and_run_is_undoable() {
    let mut e = Engine::new("t");
    let provider = MockProvider::of(&[r#"[
        {"command":"object.create","input":{"type":"core:note","name":"a"}},
        {"command":"object.create","input":{"type":"core:note","name":"b"}},
        {"command":"relation.add","input":{"type":"core:references","from":"$0.id","to":"$1.id"}}
    ]"#]);
    let rt = AgentRuntime::new(Box::new(LlmPlanner::new(provider)));
    let report = rt.run(&mut e, "create linked notes", "t");
    assert_eq!(report.status, RunStatus::Succeeded, "{}", report.summary);
    assert_eq!(report.created_objects.len(), 2);
    assert_eq!(report.verification.len(), 3);
    assert!(report.verification[2].starts_with("verified relation "));
    assert_eq!(e.history().records.len(), 1);
    e.undo().unwrap();
    assert!(e.find_object("a").is_none());
    assert!(e.find_object("b").is_none());
}

#[test]
fn relation_removed_by_later_step_fails_atomically() {
    let mut e = Engine::new("t");
    let before = serde_json::to_value(e.project()).unwrap();
    let provider = MockProvider::of(&[r#"[
        {"command":"object.create","input":{"type":"core:note","name":"a"}},
        {"command":"object.create","input":{"type":"core:note","name":"b"}},
        {"command":"relation.add","input":{"type":"core:references","from":"$0.id","to":"$1.id"}},
        {"command":"relation.remove","input":{"id":"$2.id"}}
    ]"#]);
    let rt = AgentRuntime::new(Box::new(LlmPlanner::new(provider)));
    let report = rt.run(&mut e, "create then remove a relation", "t");
    assert_eq!(report.status, RunStatus::Failed);
    assert!(
        report.summary.contains("reported relation"),
        "{}",
        report.summary
    );
    assert!(
        report.summary.contains("does not exist"),
        "{}",
        report.summary
    );
    assert!(report.steps.iter().all(|step| step.ok));
    assert_eq!(serde_json::to_value(e.project()).unwrap(), before);
    assert!(e.history().records.is_empty());
}

#[test]
fn standalone_deletion_does_not_require_deleted_object_to_survive() {
    let mut e = Engine::new("t");
    e.execute("object.create", json!({"type":"core:note", "name":"old"}))
        .unwrap();
    let rt = AgentRuntime::new(Box::new(RulePlanner));
    let report = rt.run(&mut e, "delete old", "t");
    assert_eq!(report.status, RunStatus::Succeeded, "{}", report.summary);
    assert!(e.find_object("old").is_none());
    assert!(report.verification.is_empty());
    e.undo().unwrap();
    assert!(e.find_object("old").is_some());
}

#[test]
fn agent_run_uses_runtime_transaction() {
    // a plan whose step fails mid-run must roll back entirely
    let mut e = Engine::new("t");
    let provider = MockProvider::of(&[r#"[
        {"command": "object.create", "input": {"type": "core:note", "name": "kept?"}, "note": "a"},
        {"command": "object.delete", "input": {"name": "ghost"}, "note": "fails"}
    ]"#]);
    let rt = AgentRuntime::new(Box::new(LlmPlanner::new(provider)));
    let report = rt.run(&mut e, "do two things", "t");
    assert_eq!(report.status, RunStatus::Failed);
    assert!(e.find_object("kept?").is_none(), "partial work rolled back");
    let _ = json!({}); // silence unused import in some cfgs
}

#[test]
fn agent_run_scoped_permissions_deny_writes() {
    // multi-agent profile: a read-only agent must not be able to write,
    // even though the session user can.
    let mut e = Engine::new("t");
    let rt = AgentRuntime::new(Box::new(RulePlanner));
    let report = rt.run_scoped(
        &mut e,
        "create a cube named denied-cube",
        "readonly-bot",
        Some(&["project.read".into(), "project.search".into()]),
    );
    assert_eq!(report.status, RunStatus::Failed);
    assert!(e.find_object("denied-cube").is_none());
}

#[test]
fn agent_run_scoped_permissions_allow_writes() {
    let mut e = Engine::new("t");
    let rt = AgentRuntime::new(Box::new(RulePlanner));
    let report = rt.run_scoped(
        &mut e,
        "create a cube named ok-cube",
        "builder-bot",
        Some(&[
            "project.*".into(),
            "command.execute".into(),
            "capability.execute".into(),
        ]),
    );
    assert_eq!(report.status, RunStatus::Succeeded, "{}", report.summary);
    assert!(e.find_object("ok-cube").is_some());
    assert_eq!(
        e.history().records.last().unwrap().actor.0,
        "agent:builder-bot"
    );
}
