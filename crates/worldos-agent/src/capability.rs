//! `agent.run` capability: the structured entry point for agent work.
//! Registered alongside builtin capabilities by every interface.

use crate::runtime::AgentRuntime;
use serde_json::{Value, json};
use worldos_capability::{Capability, CapabilityDescriptor, CapabilityError, CapabilityHost};

pub struct AgentRun;

impl Capability for AgentRun {
    fn descriptor(&self) -> CapabilityDescriptor {
        let mut d = CapabilityDescriptor::new(
            "agent.run",
            "Run an agent on a goal: plan → execute commands → verify, as one transaction",
            json!({
                "type": "object",
                "required": ["goal"],
                "properties": {
                    "goal": {"type": "string"},
                    "agent": {"type": "string", "description": "agent name, default `assistant`"}
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
        let rt = AgentRuntime::from_env();
        let report = rt.run(host, goal, agent);
        serde_json::to_value(report).map_err(CapabilityError::Serde)
    }
}
