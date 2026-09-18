//! The agent runtime: plan → act → verify, all inside one transaction.
//!
//! Agents are actors: every mutation is a command executed through the
//! same engine path a human uses, attributed to `agent:<name>` and fully
//! undoable as a single transaction.

#[cfg(feature = "llm")]
use crate::planner::FallbackPlanner;
use crate::planner::{PlanError, PlannedStep, Planner, RulePlanner};
use crate::report::{AgentReport, RunStatus, StepRecord};
use serde_json::Value;
use worldos_capability::CapabilityHost;
use worldos_kernel::actor::Actor;
use worldos_kernel::ids::AgentRunId;
use worldos_kernel::known::{components, types};

/// Safety limits on a single run.
#[derive(Debug, Clone)]
pub struct Budget {
    pub max_steps: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self { max_steps: 32 }
    }
}

pub struct AgentRuntime {
    planner: Box<dyn Planner>,
    budget: Budget,
}

impl Default for AgentRuntime {
    fn default() -> Self {
        Self {
            planner: Box::new(RulePlanner),
            budget: Budget::default(),
        }
    }
}

impl AgentRuntime {
    /// Planner chosen from env: `WORLDOS_LLM_KIND=openai-compatible`
    /// (feature `llm`) → LLM planner with the rule planner as fallback.
    /// Anything else / missing feature → rules only. Never panics on a
    /// half-configured provider — it just falls back.
    pub fn from_env() -> Self {
        #[cfg(feature = "llm")]
        {
            let cfg = crate::provider::ProviderConfig::default();
            if cfg.kind == "openai-compatible"
                && let Ok(p) = crate::provider::OpenAiCompatible::from_env()
            {
                return Self {
                    planner: Box::new(FallbackPlanner::new(vec![
                        Box::new(crate::planner::LlmPlanner::new(p)),
                        Box::new(RulePlanner),
                    ])),
                    budget: Budget::default(),
                };
            }
        }
        Self::default()
    }
}

impl AgentRuntime {
    pub fn new(planner: Box<dyn Planner>) -> Self {
        Self {
            planner,
            budget: Budget::default(),
        }
    }
    pub fn with_budget(mut self, budget: Budget) -> Self {
        self.budget = budget;
        self
    }

    /// Execute a goal against the host. All effects commit as ONE
    /// transaction attributed to the agent actor; any failure rolls back
    /// cleanly and is reported — never silently half-applied.
    pub fn run(&self, host: &mut dyn CapabilityHost, goal: &str, agent_name: &str) -> AgentReport {
        self.run_scoped(host, goal, agent_name, None)
    }

    /// Run with caller-declared permission grants — a multi-agent profile.
    /// `grants: Some(&["project.read", …])` gives the agent actor EXACTLY
    /// those permissions; `None` uses the agent default.
    pub fn run_scoped(
        &self,
        host: &mut dyn CapabilityHost,
        goal: &str,
        agent_name: &str,
        grants: Option<&[String]>,
    ) -> AgentReport {
        let run_id = AgentRunId::new();
        let mut agent = Actor::agent(agent_name);
        if let Some(grants) = grants {
            agent.permissions = worldos_kernel::actor::PermissionSet {
                grants: grants
                    .iter()
                    .map(worldos_kernel::actor::Permission::new)
                    .collect(),
            };
        }

        // 1. plan — inspect state, resolve commands
        let steps = match self.planner.plan(goal, host) {
            Ok(s) => s,
            Err(PlanError::Unsupported(g, supported)) => {
                return AgentReport {
                    run_id,
                    agent: agent_name.into(),
                    goal: goal.into(),
                    status: RunStatus::Unsupported,
                    steps: vec![],
                    transaction_id: None,
                    created_objects: vec![],
                    verification: vec![],
                    summary: format!("unsupported goal `{g}`. {supported}"),
                };
            }
            Err(PlanError::Failed(e)) => {
                return fail_report(run_id, agent_name, goal, format!("planning failed: {e}"));
            }
        };
        if steps.len() > self.budget.max_steps {
            return fail_report(
                run_id,
                agent_name,
                goal,
                format!(
                    "plan needs {} steps, budget is {}",
                    steps.len(),
                    self.budget.max_steps
                ),
            );
        }

        // 2. act — one transaction for the whole run
        let label = format!("agent:{agent_name}: {}", truncate(goal, 60));
        if let Err(e) = host.begin_transaction_as(&agent, &label) {
            return fail_report(run_id, agent_name, goal, e.to_string());
        }

        // record the task itself as an object → visible in graph + history
        let mut records: Vec<StepRecord> = Vec::new();
        let mut outputs: Vec<Value> = Vec::new();
        let mut created: std::collections::BTreeSet<String> = Default::default();
        let task_id = host
            .run_command_as(
                &agent,
                "object.create",
                serde_json::json!({
                    "type": types::AGENT_TASK,
                    "name": format!("agent-task-{}", &run_id.to_string()[..8]),
                    "components": { components::AGENT_TASK_INFO: {
                        "goal": goal, "agent": agent_name, "status": "running",
                        "run_id": run_id.to_string(),
                    }},
                }),
            )
            .ok()
            .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(String::from));

        let mut failed: Option<String> = None;
        for (i, step) in steps.iter().enumerate() {
            let input = resolve_refs(&step.input, &outputs);
            match host.run_command_as(&agent, &step.command, input.clone()) {
                Ok(out) => {
                    if let Some(id) = out.get("id").and_then(|v| v.as_str()) {
                        created.insert(id.to_string());
                    }
                    records.push(StepRecord {
                        index: i,
                        command: step.command.clone(),
                        input,
                        note: step.note.clone(),
                        ok: true,
                        output: out.clone(),
                        error: None,
                    });
                    outputs.push(out);
                }
                Err(e) => {
                    let msg = e.to_string();
                    records.push(StepRecord {
                        index: i,
                        command: step.command.clone(),
                        input,
                        note: step.note.clone(),
                        ok: false,
                        output: Value::Null,
                        error: Some(msg.clone()),
                    });
                    failed = Some(format!("step {} `{}` failed: {msg}", i, step.command));
                    break;
                }
            }
        }

        if let Some(msg) = failed {
            let _ = host.rollback_transaction();
            let mut rep = fail_report(run_id, agent_name, goal, msg);
            rep.steps = records;
            return rep;
        }

        // mark the task object done before commit
        if let Some(tid) = &task_id {
            let _ = host.run_command_as(
                &agent,
                "object.set_property",
                serde_json::json!({
                    "id": tid, "component": components::AGENT_TASK_INFO,
                    "path": "status", "value": "succeeded",
                }),
            );
        }
        if let Err(e) = host.commit_transaction() {
            let _ = host.rollback_transaction();
            return fail_report(run_id, agent_name, goal, e.to_string());
        }

        // 3. verify — re-read the world, confirm intended effects exist
        let created: Vec<String> = created.into_iter().collect();
        let verification = verify(host, &steps, &created);
        let summary = if records.is_empty() {
            format!(
                "read-only goal; project has {} objects",
                host.project().objects.len()
            )
        } else {
            format!(
                "{} step(s) succeeded; {} object(s) affected",
                records.iter().filter(|s| s.ok).count(),
                created.len()
            )
        };
        AgentReport {
            run_id,
            agent: agent_name.into(),
            goal: goal.into(),
            status: RunStatus::Succeeded,
            steps: records,
            transaction_id: None,
            created_objects: created,
            verification,
            summary,
        }
    }
}

/// Replace `"$N.path"` placeholders in step inputs with earlier outputs.
fn resolve_refs(input: &Value, outputs: &[Value]) -> Value {
    match input {
        Value::String(s) => {
            if let Some(rest) = s.strip_prefix('$')
                && let Some((idx, path)) = rest.split_once('.')
                && let Ok(i) = idx.parse::<usize>()
                && let Some(out) = outputs.get(i)
            {
                let mut cur = out;
                for seg in path.split('.') {
                    cur = &cur[seg];
                }
                return cur.clone();
            }
            input.clone()
        }
        Value::Array(a) => Value::Array(a.iter().map(|v| resolve_refs(v, outputs)).collect()),
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| (k.clone(), resolve_refs(v, outputs)))
                .collect(),
        ),
        _ => input.clone(),
    }
}

/// Post-run checks: every object a step claims to have created must exist.
fn verify(host: &dyn CapabilityHost, _steps: &[PlannedStep], created: &[String]) -> Vec<String> {
    created
        .iter()
        .filter_map(|id| {
            let oid = id.parse().ok()?;
            let obj = host.project().get(oid)?;
            Some(format!(
                "verified object {} ({}, {})",
                obj.name, obj.type_id, id
            ))
        })
        .collect()
}

fn fail_report(run_id: AgentRunId, agent: &str, goal: &str, msg: String) -> AgentReport {
    AgentReport {
        run_id,
        agent: agent.into(),
        goal: goal.into(),
        status: RunStatus::Failed,
        steps: vec![],
        transaction_id: None,
        created_objects: vec![],
        verification: vec![],
        summary: msg,
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}
