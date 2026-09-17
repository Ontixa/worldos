//! Planners turn a goal + world snapshot into concrete command steps.
//!
//! `RulePlanner` is the deterministic builtin: it parses common
//! engineering-goal phrasings without any LLM, so Genesis works fully
//! offline. `LlmPlanner` (feature `llm`) asks a model provider for a
//! JSON plan validated against the live command schemas — plan, act,
//! verify — never freeform mutation.

use crate::report::StepRecord;
use serde_json::{Value, json};
use worldos_capability::CapabilityHost;
use worldos_kernel::known::{components, types};

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

const SUPPORTED: &str = "create <cube|sphere|cylinder|plane|note|code file> [named X] [next to Y]; \
    rename X to Y; move X to [x,y,z]; delete X; evaluate requirements; list/inspect project";

// ------------------------------------------------------------------ rules

pub struct RulePlanner;

impl Planner for RulePlanner {
    fn plan(&self, goal: &str, host: &dyn CapabilityHost) -> Result<Vec<PlannedStep>, PlanError> {
        let g = goal.trim();
        let lower = g.to_lowercase();

        if let Some(steps) = try_create(&lower, host)? {
            return Ok(steps);
        }
        if let Some(steps) = try_rename(&lower)? {
            return Ok(steps);
        }
        if let Some(steps) = try_move(&lower, host)? {
            return Ok(steps);
        }
        if let Some(steps) = try_delete(&lower, host)? {
            return Ok(steps);
        }
        if lower.contains("evaluate") && lower.contains("requirement")
            || lower.contains("check requirement")
        {
            return Ok(vec![PlannedStep {
                command: "requirement.evaluate".into(),
                input: json!({"all": true}),
                note: "evaluate all requirements".into(),
            }]);
        }
        if lower.starts_with("inspect") || lower.starts_with("list") || lower.contains("what is in")
        {
            // read-only goal → no steps; the runtime reports project state
            return Ok(vec![]);
        }
        Err(PlanError::Unsupported(g.into(), SUPPORTED.into()))
    }
}

/// Extract a quoted or bare name after markers like `named`, `name it`,
/// `called`.
fn extract_name(lower: &str) -> Option<String> {
    for marker in ["name it", "named", "called", "name it:"] {
        if let Some(i) = lower.find(marker) {
            let rest = lower[i + marker.len()..]
                .trim()
                .trim_start_matches([':', ' '])
                .trim_end_matches(['.', '!', ';']);
            let rest = rest.trim_matches(|c| c == '\'' || c == '"');
            // strip trailing clauses like "and verify"
            let rest = rest.split(" and ").next().unwrap_or(rest).trim();
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

/// Extract anchor after "next to" / "beside" / "near".
fn extract_anchor(lower: &str) -> Option<String> {
    for marker in ["next to", "beside", "near"] {
        if let Some(i) = lower.find(marker) {
            let rest = lower[i + marker.len()..].trim();
            let rest = rest.trim_start_matches("the ").trim_end_matches(['.', ',']);
            // anchor may be followed by " named X" / " and name it X" — cut there
            let cut = [" and name", " and call", " named ", " called "]
                .iter()
                .filter_map(|m| rest.find(m))
                .min()
                .unwrap_or(rest.len());
            let anchor = rest[..cut].trim().trim_matches(|c| c == '\'' || c == '"');
            if !anchor.is_empty() {
                return Some(anchor.to_string());
            }
        }
    }
    None
}

fn resolve_anchor(host: &dyn CapabilityHost, anchor: &str) -> Option<worldos_kernel::ObjectId> {
    if anchor == "this one" || anchor == "it" || anchor == "that" {
        // most recently created spatial object
        return host
            .project()
            .objects
            .values()
            .filter(|o| types::is_primitive(&o.type_id.0))
            .max_by_key(|o| o.id.0.timestamp_ms())
            .map(|o| o.id);
    }
    host.resolve_object(anchor)
}

/// Width (x-dimension) of an object for "next to" placement.
fn object_width(host: &dyn CapabilityHost, id: worldos_kernel::ObjectId) -> f64 {
    let Some(obj) = host.project().get(id) else {
        return 1.0;
    };
    let geom = obj.component_data(components::GEOMETRY);
    let xf = obj.component_data(components::TRANSFORM);
    let sx = xf
        .and_then(|t| t.get("scale"))
        .and_then(|s| s.get(0))
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);
    let base = geom
        .and_then(|g| g.get("size"))
        .map(|s| match s {
            Value::Number(n) => n.as_f64().unwrap_or(1.0),
            Value::Array(a) => a.first().and_then(|v| v.as_f64()).unwrap_or(1.0),
            _ => 1.0,
        })
        .unwrap_or(1.0);
    base * sx
}

fn object_position(host: &dyn CapabilityHost, id: worldos_kernel::ObjectId) -> [f64; 3] {
    host.project()
        .get(id)
        .and_then(|o| o.component_data(components::TRANSFORM))
        .and_then(|t| t.get("position"))
        .map(|p| {
            [
                p.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0),
                p.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0),
                p.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0),
            ]
        })
        .unwrap_or([0.0, 0.0, 0.0])
}

fn try_create(
    lower: &str,
    host: &dyn CapabilityHost,
) -> Result<Option<Vec<PlannedStep>>, PlanError> {
    let is_create =
        lower.starts_with("create") || lower.starts_with("add") || lower.starts_with("make");
    if !is_create {
        return Ok(None);
    }
    let name = extract_name(lower);
    let anchor = extract_anchor(lower).and_then(|a| resolve_anchor(host, &a));

    if lower.contains("note") || lower.contains("document") {
        let text = lower.split(['\'', '"']).nth(1).unwrap_or("").to_string();
        return Ok(Some(vec![PlannedStep {
            command: "document.create".into(),
            input: json!({
                "name": name.unwrap_or_else(|| "note".into()),
                "text": text,
            }),
            note: "create note".into(),
        }]));
    }

    for kind in ["cube", "sphere", "cylinder", "plane"] {
        if lower.contains(kind) {
            let mut steps = vec![PlannedStep {
                command: "geometry.create_primitive".into(),
                input: json!({"kind": kind, "name": name.clone().unwrap_or_else(|| kind.into())}),
                note: format!("create {kind}"),
            }];
            if let Some(anchor_id) = anchor {
                let pos = object_position(host, anchor_id);
                let w = object_width(host, anchor_id);
                steps.push(PlannedStep {
                    command: "geometry.transform".into(),
                    input: json!({
                        "id": "$0.id",
                        "position": [pos[0] + w + 1.0, pos[1], pos[2]],
                    }),
                    note: format!("place next to anchor (x + {:.1})", w + 1.0),
                });
            }
            // re-evaluate requirements after structural change
            if host
                .project()
                .objects_of_type(types::REQUIREMENT)
                .next()
                .is_some()
            {
                steps.push(PlannedStep {
                    command: "requirement.evaluate".into(),
                    input: json!({"all": true}),
                    note: "re-evaluate requirements".into(),
                });
            }
            return Ok(Some(steps));
        }
    }

    if lower.contains("code") || lower.contains("file") {
        return Ok(Some(vec![PlannedStep {
            command: "code.create_file".into(),
            input: json!({
                "name": name.clone().unwrap_or_else(|| "main".into()),
                "language": "text",
                "source": "",
            }),
            note: "create code file".into(),
        }]));
    }
    Err(PlanError::Unsupported(lower.into(), SUPPORTED.into()))
}

fn try_rename(lower: &str) -> Result<Option<Vec<PlannedStep>>, PlanError> {
    if !lower.starts_with("rename") {
        return Ok(None);
    }
    // "rename X to Y"
    let body = lower.trim_start_matches("rename").trim();
    let Some((from, to)) = body.split_once(" to ") else {
        return Err(PlanError::Failed(
            "expected `rename <object> to <new name>`".into(),
        ));
    };
    Ok(Some(vec![PlannedStep {
        command: "object.rename".into(),
        input: json!({"object": from.trim().trim_matches('"'), "name": to.trim().trim_matches(|c| c == '"' || c == '.')}),
        note: format!("rename {} → {}", from.trim(), to.trim()),
    }]))
}

fn try_move(lower: &str, host: &dyn CapabilityHost) -> Result<Option<Vec<PlannedStep>>, PlanError> {
    if !lower.starts_with("move") && !lower.starts_with("place") {
        return Ok(None);
    }
    let body = lower
        .trim_start_matches("move")
        .trim_start_matches("place")
        .trim();
    // "move X to [x,y,z]" or "move X to x,y,z"
    let Some((name, target)) = body.split_once(" to ") else {
        return Err(PlanError::Failed(
            "expected `move <object> to [x,y,z]`".into(),
        ));
    };
    let nums: Vec<f64> = target
        .trim()
        .trim_start_matches('[')
        .trim_end_matches([']', '.'])
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if nums.len() != 3 {
        return Err(PlanError::Failed("position needs three numbers".into()));
    }
    let name = name.trim().trim_matches('"');
    if host.resolve_object(name).is_none() {
        return Err(PlanError::Failed(format!("object `{name}` not found")));
    }
    Ok(Some(vec![PlannedStep {
        command: "geometry.transform".into(),
        input: json!({"name": name, "position": nums}),
        note: format!("move {name} to {nums:?}"),
    }]))
}

fn try_delete(
    lower: &str,
    host: &dyn CapabilityHost,
) -> Result<Option<Vec<PlannedStep>>, PlanError> {
    if !lower.starts_with("delete") && !lower.starts_with("remove") {
        return Ok(None);
    }
    let name = lower
        .trim_start_matches("delete")
        .trim_start_matches("remove")
        .trim()
        .trim_end_matches('.')
        .trim_matches('"');
    if host.resolve_object(name).is_none() {
        return Err(PlanError::Failed(format!("object `{name}` not found")));
    }
    Ok(Some(vec![PlannedStep {
        command: "object.delete".into(),
        input: json!({"name": name, "cascade": true}),
        note: format!("delete {name}"),
    }]))
}

// ------------------------------------------------------------------ llm

/// LLM-backed planner: asks the provider for a JSON array of
/// `[{"command": ..., "input": {...}, "note": ...}]` and validates each
/// against the registered command schemas. Only available with `--features llm`.
pub struct LlmPlanner<P: crate::provider::ModelProvider> {
    pub provider: P,
}

impl<P: crate::provider::ModelProvider> Planner for LlmPlanner<P> {
    fn plan(&self, goal: &str, host: &dyn CapabilityHost) -> Result<Vec<PlannedStep>, PlanError> {
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
        let prompt = format!(
            "You are a WorldOS agent. Produce a JSON array of steps to achieve the goal.\n\
             Each step: {{\"command\": <command>, \"input\": <object>, \"note\": <short>}}.\n\
             Refer to earlier outputs as \"$<index>.<field>\".\n\
             Available commands:\n{}\n\nCurrent objects:\n{}\n\nGoal: {goal}\n\
             Respond with JSON array only.",
            serde_json::to_string_pretty(&schema_list).unwrap_or_default(),
            serde_json::to_string_pretty(&objects).unwrap_or_default(),
        );
        let text = self
            .provider
            .complete(&prompt)
            .map_err(|e| PlanError::Failed(e.to_string()))?;
        parse_llm_steps(&text)
    }
}

pub fn parse_llm_steps(text: &str) -> Result<Vec<PlannedStep>, PlanError> {
    // tolerate ```json fences
    let t = text.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
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
                input: s.get("input").cloned().unwrap_or(json!({})),
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
