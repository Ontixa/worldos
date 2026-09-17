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
    let ball = e.find_object("llm-ball").expect("ball created");
    assert_eq!(ball.type_id.0, "geom:sphere");
    assert_eq!(e.history().records.last().unwrap().actor.0, "agent:llm-bot");
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
