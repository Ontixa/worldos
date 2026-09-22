//! Geometry commands: parametric primitives + transforms.
//!
//! Genesis uses analytic primitives (cube/sphere/cylinder/plane) whose
//! shape is fully described by component data — no B-rep kernel yet.
//! Forge plugs OCCT in behind the same command surface.

use crate::error::CommandError;
use crate::handler::{CommandContext, CommandHandler};
use crate::schema::{CommandSchema, props};
use serde_json::{Value, json};
use std::io::Write;
use worldos_kernel::actor::Permission;
use worldos_kernel::known::{components, permissions, types};

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

/// `geometry.export` — tessellate a `geom:*` object's analytic geometry
/// and write an external mesh file (binary STL by default, OBJ text).
///
/// The output file is an external effect, not graph state: the
/// transaction journal records this command (attributed, with the
/// content digest in the output), but undo does not remove the file —
/// same contract as `worldos artifact export`. Containment mirrors the
/// rest of the file-facing surface: `..` segments are refused, existing
/// files are never overwritten (`create_new`), and the actor needs
/// `filesystem.write` on top of the schema's `artifact.export` — the
/// default agent/plugin grants do not include it.
///
/// `cad:body` objects are B-reps: export them via `cad.export_stl` +
/// `worldos artifact export` instead.
pub struct GeometryExport;

impl CommandHandler for GeometryExport {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "geometry.export",
            "geometry",
            "Tessellate a primitive and write a mesh file (stl = binary STL, obj = Wavefront); never overwrites",
            props::object(
                &["path"],
                json!({
                    "id": {"type": "string"}, "name": {"type": "string"},
                    "object": {"type": "string", "description": "id or name"},
                    "path": {"type": "string", "description": "destination file — `..` rejected, existing files never overwritten"},
                    "format": {"type": "string", "enum": ["stl", "obj"], "description": "default: stl"}
                }),
            ),
        )
        .permission(permissions::ARTIFACT_EXPORT)
    }

    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        // Deny before touching the filesystem — cad.import_step uses the
        // same in-handler pattern for filesystem.read.
        if !ctx
            .actor
            .permissions
            .is_allowed(&Permission(permissions::FILESYSTEM_WRITE.into()))
        {
            return Err(CommandError::PermissionDenied {
                command: "geometry.export".into(),
                perm: permissions::FILESYSTEM_WRITE.into(),
            });
        }
        let path = input["path"]
            .as_str()
            .ok_or_else(|| CommandError::Failed("missing `path`".into()))?;
        if path.is_empty() {
            return Err(CommandError::Failed("empty `path`".into()));
        }
        // same traversal guard as the artifact.export capability
        if path.split(['/', '\\']).any(|seg| seg == "..") {
            return Err(CommandError::Failed(
                "path traversal (`..`) is not allowed".into(),
            ));
        }
        let format = input
            .get("format")
            .and_then(|f| f.as_str())
            .unwrap_or("stl");

        // `object` accepts id-or-name (cad.* convention); id/name work too.
        let lookup = match input.get("object").and_then(|v| v.as_str()) {
            Some(t) if t.parse::<worldos_kernel::ids::ObjectId>().is_ok() => json!({"id": t}),
            Some(t) => json!({"name": t}),
            None => input.clone(),
        };
        let id = super::object::resolve_object(ctx, &lookup)?;
        let obj = ctx
            .project
            .get(id)
            .ok_or_else(|| CommandError::Failed("object vanished mid-command".into()))?;
        if obj.type_id.0 == types::CAD_BODY {
            return Err(CommandError::Failed(
                "cad:body is a B-rep — use cad.export_stl + `worldos artifact export`".into(),
            ));
        }
        let mesh = worldos_kernel::mesh::object_mesh(obj).map_err(CommandError::Kernel)?;
        let bytes = match format {
            "stl" => worldos_kernel::mesh::to_binary_stl(&mesh),
            "obj" => worldos_kernel::mesh::to_obj(&mesh),
            other => {
                return Err(CommandError::Failed(format!("unknown format `{other}`")));
            }
        };

        // Verified bytes -> new file only. A mid-write failure keeps the
        // created path (may be partial), named in the error — the same
        // retained-partial contract as Engine::export_project_artifact.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| {
                CommandError::Failed(format!(
                    "cannot create `{path}` (existing files are never overwritten): {e}"
                ))
            })?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| {
                CommandError::Failed(format!(
                    "write `{path}` failed: {e}; created file retained and may be partial"
                ))
            })?;

        let digest = worldos_artifact::ArtifactRef::of(&bytes).to_string();
        Ok(json!({
            "id": id.to_string(),
            "name": obj.name,
            "path": path,
            "format": format,
            "triangles": mesh.triangle_count(),
            "bytes": bytes.len(),
            "sha256": digest,
        }))
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
