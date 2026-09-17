//! `RulePlanner`: deterministic, offline planner for common goal phrasings.

use super::{PlanError, PlannedStep, Planner, SUPPORTED};
use serde_json::json;
use worldos_capability::CapabilityHost;
use worldos_kernel::known::types;

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
    host.project()
        .get(id)
        .and_then(|o| worldos_kernel::measure::object_dims(o).map(|d| d[0]))
        .unwrap_or(1.0)
}

fn object_position(host: &dyn CapabilityHost, id: worldos_kernel::ObjectId) -> [f64; 3] {
    host.project()
        .get(id)
        .map(worldos_kernel::measure::object_position)
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
