//! The agent runtime: a bounded observe → plan → act → inspect loop.
//!
//! Agents are actors: every mutation is a command executed through the
//! same engine path a human uses, attributed to `agent:<name>` and fully
//! undoable as a single transaction.
//!
//! A run opens ONE transaction, then iterates until a declared goal
//! predicate verifies against live engine state:
//! 1. **inspect** — evaluate the `done_when` predicate on the project;
//!    a pass ends the run successfully.
//! 2. **plan** — the planner observes prior step records and the last
//!    predicate verdict, then proposes the next batch of commands.
//! 3. **act** — execute the batch through `run_command_as`, bounded by
//!    the run-wide command budget.
//! 4. **verify** — every reported object/relation `id` is checked for
//!    presence before the next iteration and again before commit.
//!
//! Iteration/command caps, a step failure, a dead-end planner, or an
//! unverifiable reference end the run honestly: the transaction rolls
//! back and the report explains why. With no `done_when` predicate the
//! run is a single plan→act→verify pass — the original semantics.
//! Loop control is a pure function of planner output and project state;
//! nothing reads the wall clock, so tests are deterministic given a
//! deterministic planner.

#[cfg(feature = "llm")]
use crate::planner::FallbackPlanner;
use crate::planner::{Observation, PlanError, Planner, RulePlanner};
use crate::report::{AgentReport, RunStatus, StepRecord};
use serde_json::Value;
use worldos_capability::CapabilityHost;
use worldos_kernel::RequirementStatus;
use worldos_kernel::actor::Actor;
use worldos_kernel::ids::AgentRunId;
use worldos_kernel::known::{components, types};

/// Safety limits on a single run.
#[derive(Debug, Clone)]
pub struct Budget {
    /// Total planner-proposed commands a run may execute across all
    /// iterations. Internal bookkeeping (the `core:agent-task` marker)
    /// does not count.
    pub max_commands: usize,
    /// Maximum observe→plan→act→inspect cycles before the run fails.
    pub max_iterations: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_commands: 32,
            max_iterations: 8,
        }
    }
}

/// A run request: goal text plus its deterministic success condition.
///
/// `done_when` uses the requirement-expression grammar
/// (`exists_named("housing")`, `count(geom:cube) >= 2`,
/// `volume("hull") > 10 and exists_named("lid")`, …) and is evaluated
/// against live engine state after every iteration — a run with a
/// predicate succeeds only when it verifies true, never merely because
/// commands ran. With `done_when: None` the run is one bounded pass:
/// the planner's batch is executed and reference-checked, then the run
/// ends (the original single-shot semantics).
#[derive(Debug, Clone)]
pub struct RunSpec {
    pub goal: String,
    pub done_when: Option<String>,
}

impl RunSpec {
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            done_when: None,
        }
    }
    /// Declare the success predicate the loop must verify.
    pub fn with_done_when(mut self, expr: impl Into<String>) -> Self {
        self.done_when = Some(expr.into());
        self
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
        self.run_spec(host, &RunSpec::new(goal), agent_name, grants)
    }

    /// The bounded tool-use loop. `spec.done_when` declares the success
    /// predicate: when present the run terminates successfully only if it
    /// verifies true against live project state inside the transaction.
    /// Every mutation stays a governed command attributed to
    /// `agent:<name>`; reported references are validated per-iteration
    /// and again before commit; any failure rolls the whole run back.
    pub fn run_spec(
        &self,
        host: &mut dyn CapabilityHost,
        spec: &RunSpec,
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
        let goal = spec.goal.as_str();
        // An empty/whitespace predicate is no predicate at all.
        let predicate = spec
            .done_when
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty());

        // One transaction for the whole run — project() reflects pending
        // ops, so every iteration observes the effects of its own work.
        let label = format!("agent:{agent_name}: {}", truncate(goal, 60));
        if let Err(e) = host.begin_transaction_as(&agent, &label) {
            return fail_report(run_id, agent_name, spec, e.to_string());
        }

        // record the task itself as an object → visible in graph + history
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

        let mut records: Vec<StepRecord> = Vec::new();
        let mut outputs: Vec<Value> = Vec::new();
        let mut created: std::collections::BTreeSet<String> = Default::default();
        let mut verification: Vec<String> = Vec::new();
        let mut last_verdict: Option<String> = None;
        let mut iterations = 0usize;
        let mut goal_met = false;
        let mut failure: Option<String> = None;

        loop {
            // 1. inspect — the declared predicate is the only success exit.
            if let Some(expr) = predicate {
                let (status, verdict) = eval_goal(host.project(), expr);
                last_verdict = Some(verdict.clone());
                if status == RequirementStatus::Pass {
                    verification.push(format!("goal predicate `{expr}` satisfied: {verdict}"));
                    goal_met = true;
                    break;
                }
            }

            // iteration cap — checked before planning burns a cycle.
            if iterations >= self.budget.max_iterations {
                failure = Some(match &last_verdict {
                    Some(v) => format!(
                        "iteration cap {} reached; goal still unsatisfied (last check: {v})",
                        self.budget.max_iterations
                    ),
                    None => format!("iteration cap {} reached", self.budget.max_iterations),
                });
                break;
            }

            // 2. plan — the planner observes the run so far.
            let obs = Observation {
                iteration: iterations,
                steps: &records,
                verdict: last_verdict.as_deref(),
                goal_expr: predicate,
                commands_left: self.budget.max_commands.saturating_sub(records.len()),
            };
            let steps = match self.planner.plan_turn(goal, host, &obs) {
                Ok(s) => s,
                Err(PlanError::Unsupported(g, supported)) => {
                    if iterations == 0 {
                        let _ = host.rollback_transaction();
                        return AgentReport {
                            run_id,
                            agent: agent_name.into(),
                            goal: goal.into(),
                            status: RunStatus::Unsupported,
                            steps: records,
                            transaction_id: None,
                            created_objects: vec![],
                            verification,
                            summary: format!("unsupported goal `{g}`. {supported}"),
                            iterations,
                            done_when: predicate.map(String::from),
                        };
                    }
                    failure = Some(format!(
                        "planner has no further steps: unsupported goal `{g}`. {supported}"
                    ));
                    break;
                }
                Err(PlanError::Failed(e)) => {
                    failure = Some(format!("planning failed: {e}"));
                    break;
                }
            };

            if steps.is_empty() {
                if predicate.is_none() {
                    // compat: an empty plan IS the whole run (read-only goals)
                    goal_met = true;
                } else {
                    failure = Some(format!(
                        "planner produced no further steps; goal predicate unsatisfied ({})",
                        last_verdict.as_deref().unwrap_or("not checked")
                    ));
                }
                break;
            }

            // 3. act — bounded by the run-wide command budget. The batch is
            //    all-or-nothing: refuse it up front when it cannot fit.
            let remaining = self.budget.max_commands.saturating_sub(records.len());
            if steps.len() > remaining {
                failure = Some(format!(
                    "plan asks for {} more commands; only {remaining} of {} remain",
                    steps.len(),
                    self.budget.max_commands
                ));
                break;
            }
            let batch_start = records.len();
            for step in &steps {
                let i = records.len();
                let input = resolve_refs(&step.input, &outputs);
                match host.run_command_as(&agent, &step.command, input.clone()) {
                    Ok(out) => {
                        if step.command != "relation.add"
                            && let Some(id) = out.get("id").and_then(|v| v.as_str())
                        {
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
                        failure = Some(format!("step {} `{}` failed: {msg}", i, step.command));
                        break;
                    }
                }
            }
            if failure.is_some() {
                break;
            }

            // 4. verify — reported references must exist while the
            //    transaction can still roll back. Per-batch, so a bad
            //    reference fails before the loop spends another cycle.
            match verify(host, &records[batch_start..]) {
                Ok(v) => verification.extend(v),
                Err(e) => {
                    failure = Some(format!("verification failed: {e}"));
                    break;
                }
            }

            iterations += 1;
            if predicate.is_none() {
                // compat: no declared predicate → one bounded pass is the run.
                goal_met = true;
                break;
            }
        }

        if let Some(msg) = failure {
            let mut message = msg;
            if let Err(e) = host.rollback_transaction() {
                message.push_str(&format!("; rollback failed: {e}"));
            }
            let mut rep = fail_report(run_id, agent_name, spec, message);
            rep.steps = records;
            rep.iterations = iterations;
            rep.verification = verification;
            return rep;
        }
        // Every loop exit sets `goal_met` or `failure` — failure returned.
        debug_assert!(goal_met);

        // Final reference sweep: a step may have reported an object a later
        // iteration removed — the report must never name dead ids.
        if let Err(e) = verify(host, &records) {
            let mut message = format!("verification failed: {e}");
            if let Err(e) = host.rollback_transaction() {
                message.push_str(&format!("; rollback failed: {e}"));
            }
            let mut rep = fail_report(run_id, agent_name, spec, message);
            rep.steps = records;
            rep.iterations = iterations;
            rep.verification = verification;
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
            return fail_report(run_id, agent_name, spec, e.to_string());
        }

        let created: Vec<String> = created.into_iter().collect();
        let ok_steps = records.iter().filter(|s| s.ok).count();
        let summary = if predicate.is_some() {
            format!(
                "goal predicate satisfied after {iterations} iteration(s); {ok_steps} command(s) succeeded; {} object(s) affected",
                created.len()
            )
        } else if records.is_empty() {
            format!(
                "read-only goal; project has {} objects",
                host.project().objects.len()
            )
        } else {
            format!(
                "{ok_steps} step(s) succeeded; {} object(s) affected",
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
            iterations,
            done_when: predicate.map(String::from),
        }
    }
}

/// Evaluate a `done_when` predicate against live project state using the
/// requirement-expression engine: the goal spec is a transient
/// requirement object, so agents and `requirement.evaluate` share one
/// deterministic grammar.
fn eval_goal(project: &worldos_kernel::Project, expr: &str) -> (RequirementStatus, String) {
    let probe_actor = worldos_kernel::ids::ActorId::new("agent:goal-probe");
    let mut probe = worldos_kernel::Object::new(types::REQUIREMENT, "agent-goal", &probe_actor);
    probe.set_component(worldos_kernel::Component::new(
        components::REQUIREMENT_EXPR,
        serde_json::json!({"expression": expr}),
    ));
    worldos_kernel::requirement::evaluate(project, &probe)
}

/// Replace `"$N.path"` placeholders in step inputs with earlier outputs.
/// `N` is the run-wide step index — outputs accumulate across iterations.
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

/// Every top-level output `id` must identify a surviving object, except
/// `relation.add`, whose documented output identifies a relation.
/// Commands with no such output have no reference-presence check.
fn verify(host: &dyn CapabilityHost, records: &[StepRecord]) -> Result<Vec<String>, String> {
    let mut verification = Vec::new();
    for record in records {
        let Some(value) = record.output.get("id") else {
            continue;
        };
        let context = format!("step {} `{}`", record.index, record.command);
        let id = value
            .as_str()
            .ok_or_else(|| format!("{context}: reported id is not a string"))?;
        if record.command == "relation.add" {
            let rid: worldos_kernel::ids::RelationId = id
                .parse()
                .map_err(|_| format!("{context}: invalid relation id `{id}`"))?;
            if !host.project().relations.contains_key(&rid) {
                return Err(format!(
                    "{context}: reported relation `{id}` does not exist"
                ));
            }
            verification.push(format!("verified relation {id}"));
        } else {
            let oid = id
                .parse()
                .map_err(|_| format!("{context}: invalid object id `{id}`"))?;
            let obj = host
                .project()
                .get(oid)
                .ok_or_else(|| format!("{context}: reported object `{id}` does not exist"))?;
            verification.push(format!(
                "verified object {} ({}, {})",
                obj.name, obj.type_id, id
            ));
        }
    }
    Ok(verification)
}

fn fail_report(run_id: AgentRunId, agent: &str, spec: &RunSpec, msg: String) -> AgentReport {
    AgentReport {
        run_id,
        agent: agent.into(),
        goal: spec.goal.clone(),
        status: RunStatus::Failed,
        steps: vec![],
        transaction_id: None,
        created_objects: vec![],
        verification: vec![],
        summary: msg,
        iterations: 0,
        done_when: spec
            .done_when
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .map(String::from),
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}
