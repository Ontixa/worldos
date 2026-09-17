//! LLM-backed planning: prompt the provider for a JSON step list,
//! self-repair once on malformed output, then validate. `FallbackPlanner`
//! chains planners (LLM → rules) so a misconfigured or offline provider
//! degrades gracefully instead of failing the run.

use super::{PlanError, PlannedStep, Planner, parse_llm_steps};
use serde_json::{Value, json};
use worldos_capability::CapabilityHost;

/// LLM-backed planner. Generic over the provider so tests can substitute
/// a mock; `OpenAiCompatible` is the real backend (feature `llm`).
pub struct LlmPlanner<P: crate::provider::ModelProvider> {
    pub provider: P,
    /// Extra reprompt attempts after malformed output (default 1).
    pub max_repairs: usize,
}

impl<P: crate::provider::ModelProvider> LlmPlanner<P> {
    pub fn new(provider: P) -> Self {
        Self {
            provider,
            max_repairs: 1,
        }
    }
}

impl<P: crate::provider::ModelProvider> Planner for LlmPlanner<P> {
    fn plan(&self, goal: &str, host: &dyn CapabilityHost) -> Result<Vec<PlannedStep>, PlanError> {
        let prompt = build_prompt(goal, host, None);
        let text = self
            .provider
            .complete(&prompt)
            .map_err(|e| PlanError::Failed(e.to_string()))?;
        match parse_llm_steps(&text) {
            Ok(steps) => Ok(validate_steps(steps, host)?),
            Err(parse_err) if self.max_repairs > 0 => {
                // self-repair: feed the error back and ask for corrected JSON
                let repair_prompt = build_prompt(goal, host, Some(&parse_err.to_string()));
                let text2 = self
                    .provider
                    .complete(&repair_prompt)
                    .map_err(|e| PlanError::Failed(e.to_string()))?;
                let steps = parse_llm_steps(&text2).map_err(|e2| {
                    PlanError::Failed(format!(
                        "plan still malformed after repair (was: {parse_err}; now: {e2})"
                    ))
                })?;
                validate_steps(steps, host)
            }
            Err(e) => Err(e),
        }
    }
}

fn build_prompt(goal: &str, host: &dyn CapabilityHost, prev_error: Option<&str>) -> String {
    let schema_list: Vec<Value> = host
        .command_schemas()
        .iter()
        .map(|s| {
            json!({
                "command": s.command_type,
                "description": s.description,
                "input_schema": s.input_schema,
            })
        })
        .collect();
    let objects: Vec<Value> = host
        .project()
        .sorted_objects()
        .iter()
        .map(|o| json!({"id": o.id.to_string(), "name": o.name, "type": o.type_id}))
        .collect();
    let repair = prev_error
        .map(|e| format!("\n\nYour previous reply was not a valid JSON array of steps: {e}. Reply with ONLY the corrected JSON array."))
        .unwrap_or_default();
    format!(
        "You are a WorldOS agent. Produce a JSON array of steps to achieve the goal.\n\
         Each step: {{\"command\": <command>, \"input\": <object>, \"note\": <short>}}.\n\
         Refer to earlier outputs as \"$<index>.<field>\".\n\
         Available commands:\n{}\n\nCurrent objects:\n{}\n\nGoal: {goal}\n\
         Respond with JSON array only.{repair}",
        serde_json::to_string_pretty(&schema_list).unwrap_or_default(),
        serde_json::to_string_pretty(&objects).unwrap_or_default(),
    )
}

/// Reject steps naming unknown commands before they hit the engine.
fn validate_steps(
    steps: Vec<PlannedStep>,
    host: &dyn CapabilityHost,
) -> Result<Vec<PlannedStep>, PlanError> {
    let known: std::collections::BTreeSet<String> = host
        .command_schemas()
        .iter()
        .map(|s| s.command_type.clone())
        .collect();
    for s in &steps {
        if !known.contains(&s.command) {
            return Err(PlanError::Failed(format!(
                "model proposed unknown command `{}`",
                s.command
            )));
        }
    }
    Ok(steps)
}

/// Try each planner in order; the first to produce a usable plan wins.
/// `Unsupported`/`Failed` from an earlier planner falls through to the
/// next — LLM flake never blocks a goal the rules can handle.
pub struct FallbackPlanner {
    pub chain: Vec<Box<dyn Planner>>,
}

impl FallbackPlanner {
    pub fn new(chain: Vec<Box<dyn Planner>>) -> Self {
        Self { chain }
    }
}

impl Planner for FallbackPlanner {
    fn plan(&self, goal: &str, host: &dyn CapabilityHost) -> Result<Vec<PlannedStep>, PlanError> {
        let mut last = PlanError::Failed("no planners".into());
        for p in &self.chain {
            match p.plan(goal, host) {
                Ok(steps) => return Ok(steps),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}
