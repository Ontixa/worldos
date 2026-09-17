//! Requirement and decision commands — first-class engineering data.

use crate::error::CommandError;
use crate::handler::{CommandContext, CommandHandler};
use crate::schema::{CommandSchema, props};
use serde_json::{Value, json};
use worldos_kernel::known::{components, rel, types};

pub struct RequirementCreate;

impl CommandHandler for RequirementCreate {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "requirement.create",
            "requirement",
            "Create an evaluable requirement object",
            props::object(
                &["name", "expression"],
                json!({
                    "name": {"type": "string"},
                    "expression": {"type": "string",
                        "description": "e.g. exists(geom:cube), count(code:file) >= 2"},
                    "description": {"type": "string"},
                    "satisfied_by": {"type": "string", "description": "object name/id claimed to satisfy it"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let out = ctx.run_sub(
            "object.create",
            json!({
                "type": types::REQUIREMENT,
                "name": input["name"],
                "components": {
                    components::REQUIREMENT_EXPR: {
                        "expression": input["expression"],
                        "description": input.get("description").cloned().unwrap_or(Value::Null),
                    },
                    components::REQUIREMENT_STATUS: {"status": "unknown", "verdict": ""},
                },
            }),
        )?;
        if let Some(target) = input.get("satisfied_by").and_then(|t| t.as_str()) {
            let req_id = out["id"].as_str().unwrap().to_string();
            ctx.run_sub(
                "relation.add",
                json!({"type": rel::SATISFIES, "from": target, "to": req_id}),
            )?;
        }
        Ok(out)
    }
}

pub struct RequirementEvaluate;

impl CommandHandler for RequirementEvaluate {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "requirement.evaluate",
            "requirement",
            "Evaluate requirement expression(s) and write status components",
            props::object(
                &[],
                json!({
                    "id": {"type": "string"}, "name": {"type": "string"},
                    "all": {"type": "boolean", "description": "evaluate every requirement"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let all = input.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
        let targets: Vec<worldos_kernel::ObjectId> = if all {
            ctx.project
                .objects_of_type(types::REQUIREMENT)
                .map(|o| o.id)
                .collect()
        } else {
            vec![super::object::resolve_object(ctx, input)?]
        };
        let mut results = Vec::new();
        for id in targets {
            let ((status, verdict), refs) = {
                let req = ctx
                    .project
                    .get(id)
                    .ok_or_else(|| CommandError::Failed(format!("object {id} not found")))?;
                worldos_kernel::requirement::evaluate_traced(ctx.project, req)
            };
            let status_str = serde_json::to_value(status)?.as_str().unwrap().to_string();
            ctx.update_object(id, |o| {
                o.set_component(worldos_kernel::Component::new(
                    components::REQUIREMENT_STATUS,
                    json!({"status": status_str, "verdict": verdict}),
                ));
            })?;
            sync_dep_edges(ctx, id, &refs)?;
            results.push(json!({
                "id": id.to_string(), "status": status_str, "verdict": verdict,
                "depends_on": refs.iter().map(|r| r.to_string()).collect::<Vec<_>>(),
            }));
        }
        Ok(json!({"results": results}))
    }
}

/// Reconcile `core:depends-on` edges req→refs: remove stale ones, add
/// missing ones. Keeps the graph an accurate dependency trace of what
/// each requirement reads.
fn sync_dep_edges(
    ctx: &mut CommandContext,
    req: worldos_kernel::ObjectId,
    refs: &[worldos_kernel::ObjectId],
) -> Result<(), CommandError> {
    let want: std::collections::BTreeSet<worldos_kernel::ObjectId> = refs.iter().copied().collect();
    let existing: Vec<worldos_kernel::model::Relation> = ctx
        .project
        .relations_from(req)
        .filter(|r| r.type_id == rel::DEPENDS_ON)
        .cloned()
        .collect();
    for r in existing {
        if !want.contains(&r.to) {
            ctx.remove_relation(r.id)?;
        }
    }
    let have: std::collections::BTreeSet<worldos_kernel::ObjectId> = ctx
        .project
        .relations_from(req)
        .filter(|r| r.type_id == rel::DEPENDS_ON)
        .map(|r| r.to)
        .collect();
    for to in want {
        if !have.contains(&to) && ctx.project.get(to).is_some() {
            ctx.put_relation(worldos_kernel::model::Relation::new(
                rel::DEPENDS_ON,
                req,
                to,
                &ctx.actor.id,
            ))?;
        }
    }
    Ok(())
}

pub struct DecisionRecord;

impl CommandHandler for DecisionRecord {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "decision.record",
            "decision",
            "Record an engineering decision with rationale and alternatives",
            props::object(
                &["name", "choice"],
                json!({
                    "name": {"type": "string"},
                    "choice": {"type": "string"},
                    "rationale": {"type": "string"},
                    "alternatives": {"type": "array", "items": {"type": "string"}},
                    "affects": {"type": "array", "items": {"type": "string"},
                        "description": "object names/ids the decision affects"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let out = ctx.run_sub(
            "object.create",
            json!({
                "type": types::DECISION,
                "name": input["name"],
                "components": {
                    components::DECISION_INFO: {
                        "choice": input["choice"],
                        "rationale": input.get("rationale").cloned().unwrap_or(Value::Null),
                        "alternatives": input.get("alternatives").cloned().unwrap_or(json!([])),
                    },
                },
            }),
        )?;
        let decision_id = out["id"].as_str().unwrap().to_string();
        if let Some(affects) = input.get("affects").and_then(|a| a.as_array()) {
            for t in affects.iter().filter_map(|v| v.as_str()) {
                let _ = ctx.run_sub(
                    "relation.add",
                    json!({"type": rel::REFERENCES, "from": decision_id, "to": t}),
                );
            }
        }
        Ok(out)
    }
}
