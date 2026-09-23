//! `agent.run` capability: the structured entry point for agent work.
//! Registered alongside builtin capabilities by every interface.

use crate::runtime::{AgentRuntime, Budget, RunSpec};
use serde_json::{Value, json};
use worldos_capability::{Capability, CapabilityDescriptor, CapabilityError, CapabilityHost};

pub struct AgentRun;

impl Capability for AgentRun {
    fn descriptor(&self) -> CapabilityDescriptor {
        let mut d = CapabilityDescriptor::new(
            "agent.run",
            "Run an agent on a goal: bounded observe→plan→act→inspect loop, one transaction",
            json!({
                "type": "object",
                "required": ["goal"],
                "properties": {
                    "goal": {"type": "string"},
                    "agent": {"type": "string", "description": "agent name, default `assistant`"},
                    "permissions": {"type": "array", "items": {"type": "string"},
                        "description": "exact permission grants for this run (multi-agent profiles); omit for the agent default"},
                    "done_when": {"type": "string",
                        "description": "success predicate in the requirement-expression grammar (e.g. `exists_named(\"housing\")`, `count(geom:cube) >= 2`); the run iterates until it verifies true on live state — omit for a single plan→act→verify pass"},
                    "max_iterations": {"type": "integer", "minimum": 0,
                        "description": "observe→plan→act→inspect cycles before honest failure (default 8)"},
                    "max_commands": {"type": "integer", "minimum": 0,
                        "description": "planner-proposed commands per run (default 32)"}
                }
            }),
        );
        d.permissions = vec![
            worldos_kernel::known::permissions::PROJECT_READ.into(),
            worldos_kernel::known::permissions::COMMAND_EXECUTE.into(),
        ];
        d
    }
    fn execute(
        &self,
        host: &mut dyn CapabilityHost,
        input: &Value,
    ) -> Result<Value, CapabilityError> {
        let goal = input["goal"]
            .as_str()
            .ok_or_else(|| CapabilityError::Failed("missing `goal`".into()))?;
        let agent = input
            .get("agent")
            .and_then(|a| a.as_str())
            .unwrap_or("assistant");
        let grants: Option<Vec<String>> =
            input
                .get("permissions")
                .and_then(|p| p.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                });
        // Per-call budget overrides; absent/invalid values keep defaults.
        let mut budget = Budget::default();
        if let Some(n) = input.get("max_iterations").and_then(|v| v.as_u64()) {
            budget.max_iterations = n as usize;
        }
        if let Some(n) = input.get("max_commands").and_then(|v| v.as_u64()) {
            budget.max_commands = n as usize;
        }
        let spec = RunSpec {
            goal: goal.to_string(),
            done_when: input
                .get("done_when")
                .and_then(|v| v.as_str())
                .map(String::from),
        };
        let rt = AgentRuntime::from_env().with_budget(budget);
        let report = rt.run_spec(host, &spec, agent, grants.as_deref());
        serde_json::to_value(report).map_err(CapabilityError::Serde)
    }
}
