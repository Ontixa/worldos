//! Planners turn a goal + world snapshot into concrete command steps.
//!
//! `RulePlanner` is the deterministic builtin: it parses common
//! engineering-goal phrasings without any LLM, so Genesis works fully
//! offline. `LlmPlanner` (feature `llm`) asks a model provider for a
//! JSON plan validated against the live command schemas — plan, act,
//! verify — never freeform mutation. `FallbackPlanner` chains them:
//! LLM first (self-repairing once on bad output), rules as the floor.

mod llm;
mod rules;

pub use llm::{FallbackPlanner, LlmPlanner};
pub use rules::RulePlanner;

use crate::report::StepRecord;
use serde_json::Value;
use worldos_capability::CapabilityHost;

/// A single planned command invocation. `input` may contain
/// `"$stepN.path"` references resolved from earlier step outputs.
#[derive(Debug, Clone)]
pub struct PlannedStep {
    pub command: String,
    pub input: Value,
    pub note: String,
}

/// What a planner can observe between iterations of the tool-use loop.
#[derive(Debug, Clone)]
pub struct Observation<'a> {
    /// 0-based index of the upcoming iteration.
    pub iteration: usize,
    /// Commands already executed this run — inputs, outputs, errors.
    pub steps: &'a [StepRecord],
    /// Last goal-predicate verdict, when the run declared `done_when`
    /// (e.g. `` `count(geom:cube) >= 2` not satisfied ``).
    pub verdict: Option<&'a str>,
    /// The declared goal predicate the loop must verify, when any.
    pub goal_expr: Option<&'a str>,
    /// Commands the run may still execute before the budget trips.
    pub commands_left: usize,
}

impl Default for Observation<'_> {
    fn default() -> Self {
        Self {
            iteration: 0,
            steps: &[],
            verdict: None,
            goal_expr: None,
            commands_left: usize::MAX,
        }
    }
}

pub trait Planner: Send + Sync {
    /// Single-shot planning: turn a goal + world snapshot into steps.
    fn plan(&self, goal: &str, host: &dyn CapabilityHost) -> Result<Vec<PlannedStep>, PlanError>;

    /// Iterative planning for the tool-use loop: observe what the run has
    /// already done and propose the NEXT batch of steps. The default just
    /// replans the goal — correct for stateless planners (`RulePlanner`),
    /// while stateful ones (`LlmPlanner`) use the observation to react.
    /// Returning an empty batch means "no further steps to propose".
    fn plan_turn(
        &self,
        goal: &str,
        host: &dyn CapabilityHost,
        _obs: &Observation<'_>,
    ) -> Result<Vec<PlannedStep>, PlanError> {
        self.plan(goal, host)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("unsupported goal: {0}. Understood patterns: {1}")]
    Unsupported(String, String),
    #[error("cannot plan: {0}")]
    Failed(String),
}

pub(crate) const SUPPORTED: &str = "create <cube|sphere|cylinder|plane|note|code file> [named X] [next to Y]; \
    rename X to Y; move X to [x,y,z]; delete X; evaluate requirements; list/inspect project";

/// Parse a JSON step array out of model output (tolerates ```json fences).
pub fn parse_llm_steps(text: &str) -> Result<Vec<PlannedStep>, PlanError> {
    let t = text.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    // find the outermost [ ... ] — models sometimes add prose around it
    let t = match (t.find('['), t.rfind(']')) {
        (Some(a), Some(b)) if b > a => &t[a..=b],
        _ => t,
    };
    let arr: Vec<Value> =
        serde_json::from_str(t).map_err(|e| PlanError::Failed(format!("bad plan JSON: {e}")))?;
    arr.iter()
        .enumerate()
        .map(|(i, s)| {
            Ok(PlannedStep {
                command: s["command"]
                    .as_str()
                    .ok_or_else(|| PlanError::Failed(format!("step {i}: missing command")))?
                    .to_string(),
                input: s.get("input").cloned().unwrap_or(serde_json::json!({})),
                note: s
                    .get("note")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

/// Convert an executed step list into report records.
pub fn to_records(steps: &[StepRecord]) -> Vec<String> {
    steps
        .iter()
        .map(|s| format!("{}: {} ({})", s.index, s.command, s.note))
        .collect()
}
