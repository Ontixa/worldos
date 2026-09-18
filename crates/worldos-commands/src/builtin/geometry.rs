//! Geometry commands: parametric primitives + transforms.
//!
//! Genesis uses analytic primitives (cube/sphere/cylinder/plane) whose
//! shape is fully described by component data — no B-rep kernel yet.
//! Forge plugs OCCT in behind the same command surface.

use crate::error::CommandError;
use crate::handler::{CommandContext, CommandHandler};
use crate::schema::{CommandSchema, props};
use serde_json::{Value, json};
use worldos_kernel::known::{components, types};

pub struct GeometryCreatePrimitive;

impl CommandHandler for GeometryCreatePrimitive {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "geometry.create_primitive",
            "geometry",
            "Create a 3D primitive (cube, sphere, cylinder, cone, torus, plane) with transform + material",
            props::object(
                &["kind"],
                json!({
                    "kind": {"type": "string", "enum": ["cube", "sphere", "cylinder", "cone", "torus", "plane"]},
                    "name": {"type": "string"},
                    "size": {"description": "scalar or [x,y,z]"},
                    "position": {"type": "array", "items": {"type": "number"}},
                    "color": {"type": "string", "description": "css color e.g. #8ab4f8"},
                    "parent": {"type": "string"}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let kind = input["kind"].as_str().unwrap();
        let type_id = format!("geom:{kind}");
        if !types::is_primitive(&type_id) {
            return Err(CommandError::Failed(format!("unknown primitive `{kind}`")));
        }
        let name = input
            .get("name")
            .and_then(|n| n.as_str())
            .map(String::from)
            .unwrap_or_else(|| format!("{kind}-{}", short_seq(ctx)));
        let size = normalize_size(kind, input.get("size").cloned().unwrap_or(json!(1.0)));
        let position = input.get("position").cloned().unwrap_or(json!([0, 0, 0]));
        let color = input.get("color").cloned().unwrap_or(json!("#9aa7b8"));
        let mut args = json!({
            "type": type_id,
            "name": name,
            "components": {
                components::TRANSFORM: {
                    "position": position, "rotation": [0, 0, 0], "scale": [1, 1, 1]
                },
                components::GEOMETRY: {"kind": kind, "size": size},
                components::MATERIAL: {"color": color, "roughness": 0.7, "metallic": 0.0}
            },
        });
        if let Some(p) = input.get("parent") {
            args["parent"] = p.clone();
        }
        ctx.run_sub("object.create", args)
    }
}

/// Torus size is stored normalized as `[ring_d, tube_d, ring_d]` (flat in
/// the xz ground plane) so bounding boxes stay meaningful: scalar `s` →
/// `[s, s/3, s]`; array `[D, d, …]` → `[D, d, D]`. Other kinds keep
/// `size` verbatim.
fn normalize_size(kind: &str, size: Value) -> Value {
    if kind != "torus" {
        return size;
    }
    match size {
        Value::Number(n) => {
            let s = n.as_f64().unwrap_or(1.0);
            json!([s, s / 3.0, s])
        }
        Value::Array(a) => {
            let big_d = a.first().and_then(|v| v.as_f64()).unwrap_or(1.0);
            let tube_d = a.get(1).and_then(|v| v.as_f64()).unwrap_or(big_d / 3.0);
            json!([big_d, tube_d, big_d])
        }
        other => other,
    }
}

/// Count existing primitives to make default names deterministic-ish.
fn short_seq(ctx: &CommandContext) -> usize {
    ctx.project
        .objects
        .values()
        .filter(|o| types::is_primitive(&o.type_id.0))
        .count()
        + 1
}

pub struct GeometryTransform;

impl CommandHandler for GeometryTransform {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "geometry.transform",
            "geometry",
            "Move/rotate/scale an object; `translate` is relative, `position`/`rotation`/`scale` absolute",
            props::object(
                &[],
                json!({
                    "id": {"type": "string"}, "name": {"type": "string"},
                    "translate": {"type": "array", "items": {"type": "number"}},
                    "position": {"type": "array", "items": {"type": "number"}},
                    "rotation": {"type": "array", "items": {"type": "number"}, "description": "degrees [x,y,z]"},
                    "scale": {"type": "array", "items": {"type": "number"}}
                }),
            ),
        )
    }
    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let id = super::object::resolve_object(ctx, input)?;
        let translate = input.get("translate").cloned();
        let position = input.get("position").cloned();
        let rotation = input.get("rotation").cloned();
        let scale = input.get("scale").cloned();
        ctx.update_object(id, |o| {
            let entry = o
                .components
                .entry(components::TRANSFORM.to_string())
                .or_insert_with(|| {
                    worldos_kernel::Component::new(
                        components::TRANSFORM,
                        json!({"position": [0,0,0], "rotation": [0,0,0], "scale": [1,1,1]}),
                    )
                });
            if let Some(t) = &translate {
                let cur = entry.data["position"].clone();
                entry.data["position"] = add_vec3(&cur, t);
            }
            if let Some(p) = &position {
                entry.data["position"] = p.clone();
            }
            if let Some(r) = &rotation {
                entry.data["rotation"] = r.clone();
            }
            if let Some(s) = &scale {
                entry.data["scale"] = s.clone();
            }
        })?;
        let t = ctx
            .project
            .get(id)
            .and_then(|o| o.component_data(components::TRANSFORM))
            .cloned();
        Ok(json!({"id": id.to_string(), "transform": t}))
    }
}

fn add_vec3(a: &Value, b: &Value) -> Value {
    let get = |v: &Value, i: usize| v.get(i).and_then(|x| x.as_f64()).unwrap_or(0.0);
    json!([
        get(a, 0) + get(b, 0),
        get(a, 1) + get(b, 1),
        get(a, 2) + get(b, 2)
    ])
}
