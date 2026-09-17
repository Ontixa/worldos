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

pub trait Planner: Send + Sync {
    fn plan(&self, goal: &str, host: &dyn CapabilityHost) -> Result<Vec<PlannedStep>, PlanError>;
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
