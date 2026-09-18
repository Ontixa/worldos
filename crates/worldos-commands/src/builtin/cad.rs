//! CAD commands: real B-rep modeling through `CadKernel`.
//!
//! Every handler follows the same contract: build the shape in the
//! kernel, persist the BRep via the artifact store, read back kernel-
//! verified measures/topology, and write semantic components —
//! `cad:operation` (the regeneration recipe) + `cad:shape` (derived
//! state). The live `ShapeId` is dropped before returning; the graph
//! never stores kernel handles.

use std::sync::Arc;

use serde_json::{Value, json};
use worldos_artifact::ArtifactStore;
use worldos_cad::{CadKernel, ShapeId, TessParams};
use worldos_kernel::known::{components, types};

use crate::error::CommandError;
use crate::handler::{CommandContext, CommandHandler};
use crate::schema::{CommandSchema, props};

/// Kernel + artifact store a CAD command needs. Constructed once by the
/// engine (`Engine::attach_cad`) and shared by every cad handler.
///
/// The artifact store is behind a lock so the engine can rebind it when
/// the project is saved to a new location — `save_as` migrates every
/// blob to the new sidecar so reopening finds them.
pub struct CadServices {
    pub kernel: Arc<dyn CadKernel>,
    store: std::sync::RwLock<Arc<ArtifactStore>>,
}

impl CadServices {
    pub fn new(kernel: Arc<dyn CadKernel>, artifacts: ArtifactStore) -> Arc<Self> {
        Arc::new(Self {
            kernel,
            store: std::sync::RwLock::new(Arc::new(artifacts)),
        })
    }

    /// The artifact store currently bound to the project.
    pub fn artifacts(&self) -> Arc<ArtifactStore> {
        self.store.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Point `services` at `store`, migrating every blob from the
    /// current store first (content-addressed copies are idempotent).
    pub fn rebind(&self, store: ArtifactStore) -> Result<(), worldos_artifact::ArtifactError> {
        let old = self.artifacts();
        for r in old.list()? {
            let bytes = old.get(&r)?;
            store.put(&bytes)?;
        }
        *self.store.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(store);
        Ok(())
    }
}

/// Object metadata for a new `cad:body`.
struct ShapeMeta {
    name: String,
    position: Value,
    parent: Option<Value>,
    /// Source objects this body was derived from — emits
    /// `core:derived-from` edges (the feature-tree lineage).
    sources: Vec<worldos_kernel::ids::ObjectId>,
}

/// After the kernel produced `shape`: persist BRep, read verified
/// measures/topology, insert the `cad:body` object, drop the handle.
fn finalize_shape(
    ctx: &mut CommandContext,
    services: &CadServices,
    shape: ShapeId,
    op_kind: &str,
    mut params: Value,
    generator: &str,
    meta: ShapeMeta,
) -> Result<Value, CommandError> {
    let ShapeMeta {
        name,
        position,
        parent,
        sources,
    } = meta;
    let kernel = &services.kernel;
    // `position` is baked into the BRep: geometry is world-space truth,
    // not just transform metadata — booleans/measures must agree with
    // what the graph claims.
    let mut shape = shape;
    let delta = position.as_array().and_then(|a| {
        if a.len() != 3 {
            return None;
        }
        let d = [a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?];
        d.iter().any(|x| x.abs() > 0.0).then_some(d)
    });
    if let Some(delta) = delta {
        let moved = kernel
            .transform(
                shape,
                &[worldos_cad::TransformOp::Translate { delta_mm: delta }],
            )
            .map_err(|e| CommandError::Failed(format!("position bake failed: {e}")))?;
        kernel.drop_shape(shape);
        shape = moved;
        params["position"] = json!(delta);
    }
    let brep_bytes = kernel
        .export_brep(shape)
        .map_err(|e| CommandError::Failed(format!("brep export failed: {e}")))?;
    let put = services
        .artifacts()
        .put(&brep_bytes)
        .map_err(|e| CommandError::Failed(format!("artifact store failed: {e}")))?;
    let measures = kernel
        .measure(shape)
        .map_err(|e| CommandError::Failed(format!("measure failed: {e}")))?;
    let topology = kernel
        .topology(shape)
        .map_err(|e| CommandError::Failed(format!("topology failed: {e}")))?;
    kernel.drop_shape(shape);

    if !topology.is_valid {
        return Err(CommandError::Failed(
            "kernel produced an invalid shape (non-positive volume or no topology)".into(),
        ));
    }

    let brep_ref = put.artifact_ref.to_string();
    let operation = worldos_cad::CadOperation::new(op_kind, params, kernel.name());
    let shape_state = worldos_cad::CadShape::new(
        &brep_ref,
        kernel.name(),
        generator,
        measures,
        topology.clone(),
    );

    let mut args = json!({
        "type": types::CAD_BODY,
        "name": &name,
        "components": {
            components::TRANSFORM: {
                "position": position, "rotation": [0, 0, 0], "scale": [1, 1, 1]
            },
            components::CAD_OPERATION: serde_json::to_value(&operation)
                .map_err(|e| CommandError::Failed(e.to_string()))?,
            components::CAD_SHAPE: serde_json::to_value(&shape_state)
                .map_err(|e| CommandError::Failed(e.to_string()))?,
        },
    });
    if let Some(p) = parent {
        args["parent"] = p;
    }
    let id = ctx
        .run_sub("object.create", args)
        .map_err(|e| CommandError::Failed(format!("object.create failed: {e}")))?["id"]
        .clone();

    if !sources.is_empty() {
        let oid: worldos_kernel::ids::ObjectId = id
            .as_str()
            .unwrap_or_default()
            .parse()
            .map_err(|_| CommandError::Failed("bad id from object.create".into()))?;
        for src in sources {
            ctx.put_relation(worldos_kernel::model::Relation::new(
                worldos_kernel::known::rel::DERIVED_FROM,
                oid,
                src,
                &ctx.actor.id,
            ))?;
        }
    }

    Ok(json!({
        "id": id,
        "name": name,
        "type": types::CAD_BODY,
        "brep": brep_ref,
        "measures": serde_json::to_value(measures).unwrap_or(Value::Null),
        "topology": serde_json::to_value(&topology).unwrap_or(Value::Null),
    }))
}

fn auto_name(ctx: &CommandContext, prefix: &str) -> String {
    let seq = ctx.project.objects.len() + 1;
    format!("{prefix}-{seq}")
}

fn position_of(input: &Value) -> Value {
    input.get("position").cloned().unwrap_or(json!([0, 0, 0]))
}

macro_rules! cad_create {
    ($name:ident, $cmd:literal, $doc:literal, $kind:literal, $req:expr, $props:expr, $make:expr) => {
        pub struct $name {
            services: Arc<CadServices>,
        }

        impl $name {
            pub fn new(services: Arc<CadServices>) -> Self {
                Self { services }
            }
        }

        impl CommandHandler for $name {
            fn schema(&self) -> CommandSchema {
                CommandSchema::write($cmd, "cad", $doc, props::object($req, $props))
            }

            fn execute(
                &self,
                ctx: &mut CommandContext,
                input: &Value,
            ) -> Result<Value, CommandError> {
                let make: fn(&CadServices, &Value) -> Result<(ShapeId, Value), CommandError> =
                    $make;
                let (shape, params) = make(&self.services, input)?;
                let name = input
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| auto_name(ctx, $kind));
                finalize_shape(
                    ctx,
                    &self.services,
                    shape,
                    $kind,
                    params,
                    $cmd,
                    ShapeMeta {
                        name,
                        position: position_of(input),
                        parent: input.get("parent").cloned(),
                        sources: vec![],
                    },
                )
            }
        }
    };
}

fn vec3(input: &Value, key: &str) -> Result<[f64; 3], CommandError> {
    let v = input
        .get(key)
        .ok_or_else(|| CommandError::Failed(format!("missing `{key}`")))?;
    if let Some(n) = v.as_f64() {
        return Ok([n, n, n]);
    }
    let arr = v
        .as_array()
        .ok_or_else(|| CommandError::Failed(format!("`{key}` must be a number or [x,y,z]")))?;
    if arr.len() != 3 {
        return Err(CommandError::Failed(format!("`{key}` needs 3 elements")));
    }
    let mut out = [0.0; 3];
    for (i, x) in arr.iter().enumerate() {
        out[i] = x
            .as_f64()
            .ok_or_else(|| CommandError::Failed(format!("`{key}[{i}]` is not a number")))?;
    }
    Ok(out)
}

fn scalar(input: &Value, key: &str) -> Result<f64, CommandError> {
    input
        .get(key)
        .and_then(|v| v.as_f64())
        .ok_or_else(|| CommandError::Failed(format!("`{key}` must be a number")))
}

cad_create!(
    CadCreateBox,
    "cad.create_box",
    "Create a B-rep box (OCCT kernel) with cad:operation recipe + cad:shape derived state",
    "create_box",
    &["size_mm"],
    json!({
        "size_mm": {"description": "scalar or [x,y,z] extents in mm"},
        "name": {"type": "string"},
        "position": {"type": "array", "items": {"type": "number"}},
        "parent": {"type": "string"}
    }),
    |services, input| {
        let [x, y, z] = vec3(input, "size_mm")?;
        let shape = services
            .kernel
            .make_box(x, y, z)
            .map_err(|e| CommandError::Failed(format!("kernel: {e}")))?;
        Ok((shape, json!({"size_mm": [x, y, z]})))
    }
);

cad_create!(
    CadCreateCylinder,
    "cad.create_cylinder",
    "Create a B-rep cylinder along +Z (OCCT kernel)",
    "create_cylinder",
    &[],
    json!({
        "size_mm": {"description": "scalar radius or [radius,height] in mm"},
        "radius_mm": {"type": "number"},
        "height_mm": {"type": "number"},
        "name": {"type": "string"},
        "position": {"type": "array", "items": {"type": "number"}},
        "parent": {"type": "string"}
    }),
    |services, input| {
        let (r, h) = match (input.get("radius_mm"), input.get("height_mm")) {
            (Some(_), Some(_)) => (scalar(input, "radius_mm")?, scalar(input, "height_mm")?),
            _ => {
                let v = input.get("size_mm").ok_or_else(|| {
                    CommandError::Failed("need radius_mm+height_mm or size_mm [r,h]".into())
                })?;
                match v.as_array().map(|a| a.len()) {
                    Some(2) => (
                        v[0].as_f64()
                            .ok_or_else(|| CommandError::Failed("radius not a number".into()))?,
                        v[1].as_f64()
                            .ok_or_else(|| CommandError::Failed("height not a number".into()))?,
                    ),
                    _ => {
                        return Err(CommandError::Failed(
                            "size_mm for a cylinder must be [radius,height]".into(),
                        ));
                    }
                }
            }
        };
        let shape = services
            .kernel
            .make_cylinder(r, h)
            .map_err(|e| CommandError::Failed(format!("kernel: {e}")))?;
        Ok((shape, json!({"radius_mm": r, "height_mm": h})))
    }
);

cad_create!(
    CadCreateSphere,
    "cad.create_sphere",
    "Create a B-rep sphere centered at origin (OCCT kernel)",
    "create_sphere",
    &[],
    json!({
        "size_mm": {"description": "scalar radius or [radius] in mm"},
        "radius_mm": {"type": "number"},
        "name": {"type": "string"},
        "position": {"type": "array", "items": {"type": "number"}},
        "parent": {"type": "string"}
    }),
    |services, input| {
        let r = if let Ok(r) = scalar(input, "radius_mm") {
            r
        } else {
            match input.get("size_mm") {
                Some(v) if v.is_number() => v.as_f64().unwrap_or(0.0),
                Some(v) if v.is_array() && v.as_array().map(|a| a.len()) == Some(1) => {
                    v[0].as_f64().unwrap_or(0.0)
                }
                _ => {
                    return Err(CommandError::Failed(
                        "need radius_mm or scalar size_mm".into(),
                    ));
                }
            }
        };
        let shape = services
            .kernel
            .make_sphere(r)
            .map_err(|e| CommandError::Failed(format!("kernel: {e}")))?;
        Ok((shape, json!({"radius_mm": r})))
    }
);

/// Load a `cad:body` object's shape into the kernel: resolve the
/// object, read `cad:shape`, fetch the BRep blob, import it.
/// Caller owns the returned `ShapeId` and must drop it.
fn load_shape(
    ctx: &CommandContext,
    services: &CadServices,
    input: &Value,
) -> Result<
    (
        worldos_kernel::ids::ObjectId,
        ShapeId,
        worldos_cad::CadShape,
    ),
    CommandError,
> {
    // `object` accepts an id or a name — translate to resolver shape.
    let target = input
        .get("object")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CommandError::Failed("missing `object` (id or name)".into()))?;
    let lookup = if target.parse::<worldos_kernel::ids::ObjectId>().is_ok() {
        json!({"id": target})
    } else {
        json!({"name": target})
    };
    let id = crate::builtin::resolve_object(ctx, &lookup)?;
    let obj = ctx
        .project
        .get(id)
        .ok_or_else(|| CommandError::Failed("object vanished mid-command".into()))?;
    if obj.type_id.0 != types::CAD_BODY {
        return Err(CommandError::Failed(format!(
            "object `{}` is a `{}`, not a cad:body",
            obj.name, obj.type_id
        )));
    }
    let state: worldos_cad::CadShape = serde_json::from_value(
        obj.component_data(components::CAD_SHAPE)
            .ok_or_else(|| CommandError::Failed("no cad:shape component".into()))?
            .clone(),
    )
    .map_err(|e| CommandError::Failed(format!("bad cad:shape payload: {e}")))?;
    if state.kernel != services.kernel.name() {
        return Err(CommandError::Failed(format!(
            "shape was built by kernel `{}`, attached kernel is `{}` — refusing to guess",
            state.kernel,
            services.kernel.name()
        )));
    }
    let brep_ref: worldos_artifact::ArtifactRef =
        state
            .brep
            .parse()
            .map_err(|e: worldos_artifact::ArtifactError| {
                CommandError::Failed(format!("bad brep ref: {e}"))
            })?;
    let bytes = services
        .artifacts()
        .get(&brep_ref)
        .map_err(|e| CommandError::Failed(format!("brep artifact missing/corrupt: {e}")))?;
    let sid = services
        .kernel
        .import_brep(&bytes)
        .map_err(|e| CommandError::Failed(format!("brep import failed: {e}")))?;
    Ok((id, sid, state))
}

/// `cad.measure` — kernel-verified measures on demand. Rewrites the
/// derived `cad:shape` component (undoable write).
pub struct CadMeasure {
    services: Arc<CadServices>,
}

impl CadMeasure {
    pub fn new(services: Arc<CadServices>) -> Self {
        Self { services }
    }
}

impl CommandHandler for CadMeasure {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "cad.measure",
            "cad",
            "Kernel-verified volume/area/bbox/center for a cad:body (mm, mm3, mm2)",
            props::object(
                &[],
                json!({
                    "object": {"type": "string", "description": "id or name"}
                }),
            ),
        )
    }

    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let (id, sid, mut state) = load_shape(ctx, &self.services, input)?;
        let measures = self
            .services
            .kernel
            .measure(sid)
            .map_err(|e| CommandError::Failed(format!("measure: {e}")))?;
        let topology = self
            .services
            .kernel
            .topology(sid)
            .map_err(|e| CommandError::Failed(format!("topology: {e}")))?;
        self.services.kernel.drop_shape(sid);

        state.measures = measures;
        state.topology = topology.clone();
        ctx.update_object(id, |obj| {
            if let Ok(v) = serde_json::to_value(&state) {
                obj.set_component(worldos_kernel::model::Component::new(
                    components::CAD_SHAPE,
                    v,
                ));
            }
        })?;

        Ok(json!({
            "id": id.to_string(),
            "measures": serde_json::to_value(measures).unwrap_or(Value::Null),
            "topology": serde_json::to_value(&topology).unwrap_or(Value::Null),
        }))
    }
}

macro_rules! cad_export {
    ($name:ident, $cmd:literal, $doc:literal, $ext:literal, $export:expr, $field:ident) => {
        pub struct $name {
            services: Arc<CadServices>,
        }

        impl $name {
            pub fn new(services: Arc<CadServices>) -> Self {
                Self { services }
            }
        }

        impl CommandHandler for $name {
            fn schema(&self) -> CommandSchema {
                CommandSchema::write(
                    $cmd,
                    "cad",
                    $doc,
                    props::object(
                        &[],
                        json!({
                            "object": {"type": "string", "description": "id or name"}
                        }),
                    ),
                )
                .permission(worldos_kernel::known::permissions::ARTIFACT_EXPORT)
            }

            fn execute(
                &self,
                ctx: &mut CommandContext,
                input: &Value,
            ) -> Result<Value, CommandError> {
                let (id, sid, mut state) = load_shape(ctx, &self.services, input)?;
                let export: fn(&dyn CadKernel, ShapeId) -> Result<Vec<u8>, worldos_cad::CadError> =
                    $export;
                let bytes = export(&*self.services.kernel, sid)
                    .map_err(|e| CommandError::Failed(format!("export: {e}")))?;
                self.services.kernel.drop_shape(sid);
                let put = self
                    .services
                    .artifacts()
                    .put(&bytes)
                    .map_err(|e| CommandError::Failed(format!("artifact store: {e}")))?;
                let r = put.artifact_ref.to_string();
                state.$field = Some(r.clone());
                ctx.update_object(id, |obj| {
                    if let Ok(v) = serde_json::to_value(&state) {
                        obj.set_component(worldos_kernel::model::Component::new(
                            components::CAD_SHAPE,
                            v,
                        ));
                    }
                })?;
                Ok(json!({
                    "id": id.to_string(),
                    $ext: r,
                    "size_bytes": put.size,
                }))
            }
        }
    };
}

cad_export!(
    CadExportStep,
    "cad.export_step",
    "Export a cad:body to STEP (AP242) and store it as an artifact; records the ref in cad:shape.step",
    "step",
    |k: &dyn CadKernel, sid| k.export_step(sid),
    step
);

cad_export!(
    CadExportStl,
    "cad.export_stl",
    "Tessellate a cad:body and export STL as an artifact; records the ref in cad:shape.stl",
    "stl",
    |k: &dyn CadKernel, sid| k.export_stl(sid, TessParams::default()),
    stl
);

/// `cad.boolean` — union/subtract/intersect of two bodies → new body.
pub struct CadBoolean {
    services: Arc<CadServices>,
}

impl CadBoolean {
    pub fn new(services: Arc<CadServices>) -> Self {
        Self { services }
    }
}

impl CommandHandler for CadBoolean {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "cad.boolean",
            "cad",
            "Boolean union/subtract/intersect of two cad:body objects -> new derived body",
            props::object(
                &["a", "b", "op"],
                json!({
                    "a": {"type": "string", "description": "id or name"},
                    "b": {"type": "string", "description": "id or name"},
                    "op": {"type": "string", "enum": ["union", "subtract", "intersect"]},
                    "name": {"type": "string"},
                    "parent": {"type": "string"}
                }),
            ),
        )
    }

    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let op = match input["op"].as_str().unwrap_or_default() {
            "union" => worldos_cad::BoolOp::Union,
            "subtract" => worldos_cad::BoolOp::Subtract,
            "intersect" => worldos_cad::BoolOp::Intersect,
            other => {
                return Err(CommandError::Failed(format!(
                    "unknown boolean op `{other}`"
                )));
            }
        };
        let (a_id, a_shape, _) = load_shape(ctx, &self.services, &json!({"object": input["a"]}))?;
        let (b_id, b_shape, _) = load_shape(ctx, &self.services, &json!({"object": input["b"]}))?;
        let out = self
            .services
            .kernel
            .boolean(a_shape, b_shape, op)
            .map_err(|e| CommandError::Failed(format!("boolean failed: {e}")))?;
        self.services.kernel.drop_shape(a_shape);
        self.services.kernel.drop_shape(b_shape);

        let name = input
            .get("name")
            .and_then(|n| n.as_str())
            .map(String::from)
            .unwrap_or_else(|| auto_name(ctx, "boolean"));
        finalize_shape(
            ctx,
            &self.services,
            out,
            "boolean",
            json!({
                "a": a_id.to_string(),
                "b": b_id.to_string(),
                "op": input["op"].as_str().unwrap_or_default(),
            }),
            "cad.boolean",
            ShapeMeta {
                name,
                position: json!([0, 0, 0]),
                parent: input.get("parent").cloned(),
                sources: vec![a_id, b_id],
            },
        )
    }
}

macro_rules! cad_feature {
    ($name:ident, $cmd:literal, $doc:literal, $kind:literal, $param_key:literal, $unit:literal, $call:expr) => {
        pub struct $name {
            services: Arc<CadServices>,
        }

        impl $name {
            pub fn new(services: Arc<CadServices>) -> Self {
                Self { services }
            }
        }

        impl CommandHandler for $name {
            fn schema(&self) -> CommandSchema {
                CommandSchema::write(
                    $cmd,
                    "cad",
                    $doc,
                    props::object(
                        &["object", $param_key],
                        json!({
                            "object": {"type": "string", "description": "id or name"},
                            $param_key: {"type": "number", "description": $unit},
                            "edge_ids": {
                                "type": "array",
                                "items": {"type": "integer"},
                                "description": "kernel edge ids from cad:shape.topology; empty = all edges"
                            },
                            "name": {"type": "string"},
                            "parent": {"type": "string"}
                        }),
                    ),
                )
            }

            fn execute(
                &self,
                ctx: &mut CommandContext,
                input: &Value,
            ) -> Result<Value, CommandError> {
                let amount = scalar(input, $param_key)?;
                let edge_ids: Vec<u64> = input
                    .get("edge_ids")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_u64()).collect())
                    .unwrap_or_default();
                let (src_id, src_shape, _) = load_shape(ctx, &self.services, input)?;
                let apply: fn(&dyn CadKernel, ShapeId, f64, &[u64]) -> Result<ShapeId, worldos_cad::CadError> =
                    $call;
                let out = apply(&*self.services.kernel, src_shape, amount, &edge_ids)
                    .map_err(|e| CommandError::Failed(format!("{} failed: {e}", $kind)))?;
                self.services.kernel.drop_shape(src_shape);

                let name = input
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| auto_name(ctx, $kind));
                finalize_shape(
                    ctx,
                    &self.services,
                    out,
                    $kind,
                    json!({
                        "source": src_id.to_string(),
                        $param_key: amount,
                        "edge_ids": edge_ids,
                    }),
                    $cmd,
                    ShapeMeta {
                        name,
                        position: json!([0, 0, 0]),
                        parent: input.get("parent").cloned(),
                        sources: vec![src_id],
                    },
                )
            }
        }
    };
}

cad_feature!(
    CadFillet,
    "cad.fillet",
    "Fillet edges of a cad:body -> new derived body",
    "fillet",
    "radius_mm",
    "fillet radius in mm",
    |k: &dyn CadKernel, s, r, e| k.fillet(s, r, e)
);

cad_feature!(
    CadChamfer,
    "cad.chamfer",
    "Chamfer edges of a cad:body -> new derived body",
    "chamfer",
    "distance_mm",
    "chamfer distance in mm",
    |k: &dyn CadKernel, s, d, e| k.chamfer(s, d, e)
);

/// `cad.transform` — rigid/affine transform chain -> new derived body.
pub struct CadTransform {
    services: Arc<CadServices>,
}

impl CadTransform {
    pub fn new(services: Arc<CadServices>) -> Self {
        Self { services }
    }
}

impl CommandHandler for CadTransform {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "cad.transform",
            "cad",
            "Apply translate/rotate_axis/scale steps to a cad:body -> new derived body",
            props::object(
                &["object", "ops"],
                json!({
                    "object": {"type": "string"},
                    "ops": {
                        "type": "array",
                        "items": {"type": "object"},
                        "description": "TransformOp list, internally tagged: {kind:translate,delta_mm:[x,y,z]} | {kind:rotate_axis,origin_mm:[x,y,z],dir:[x,y,z],angle_rad:f} | {kind:scale,center_mm:[x,y,z],factor:f}"
                    },
                    "name": {"type": "string"},
                    "parent": {"type": "string"}
                }),
            ),
        )
    }

    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let ops: Vec<worldos_cad::TransformOp> = serde_json::from_value(input["ops"].clone())
            .map_err(|e| CommandError::Failed(format!("bad ops: {e}")))?;
        if ops.is_empty() {
            return Err(CommandError::Failed("ops must not be empty".into()));
        }
        let (src_id, src_shape, _) = load_shape(ctx, &self.services, input)?;
        let out = self
            .services
            .kernel
            .transform(src_shape, &ops)
            .map_err(|e| CommandError::Failed(format!("transform failed: {e}")))?;
        self.services.kernel.drop_shape(src_shape);

        let name = input
            .get("name")
            .and_then(|n| n.as_str())
            .map(String::from)
            .unwrap_or_else(|| auto_name(ctx, "transform"));
        finalize_shape(
            ctx,
            &self.services,
            out,
            "transform",
            json!({
                "source": src_id.to_string(),
                "ops": serde_json::to_value(&ops).unwrap_or(Value::Null),
            }),
            "cad.transform",
            ShapeMeta {
                name,
                position: json!([0, 0, 0]),
                parent: input.get("parent").cloned(),
                sources: vec![src_id],
            },
        )
    }
}

/// `cad.import_step` — load a STEP file (artifact ref or filesystem
/// path) into a new cad:body.
pub struct CadImportStep {
    services: Arc<CadServices>,
}

impl CadImportStep {
    pub fn new(services: Arc<CadServices>) -> Self {
        Self { services }
    }
}

impl CommandHandler for CadImportStep {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "cad.import_step",
            "cad",
            "Import STEP into a new cad:body (from artifact ref `step` or `file` path)",
            props::object(
                &[],
                json!({
                    "step": {"type": "string", "description": "artifact ref sha256:<hex>"},
                    "file": {"type": "string", "description": "filesystem path (needs filesystem.read)"},
                    "name": {"type": "string"},
                    "parent": {"type": "string"}
                }),
            ),
        )
    }

    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let bytes = if let Some(r) = input.get("step").and_then(|v| v.as_str()) {
            let aref: worldos_artifact::ArtifactRef =
                r.parse().map_err(|e: worldos_artifact::ArtifactError| {
                    CommandError::Failed(format!("bad artifact ref: {e}"))
                })?;
            self.services
                .artifacts()
                .get(&aref)
                .map_err(|e| CommandError::Failed(format!("step artifact: {e}")))?
        } else if let Some(f) = input.get("file").and_then(|v| v.as_str()) {
            if !ctx
                .actor
                .permissions
                .is_allowed(&worldos_kernel::actor::Permission(
                    worldos_kernel::known::permissions::FILESYSTEM_READ.into(),
                ))
            {
                return Err(CommandError::PermissionDenied {
                    command: "cad.import_step".into(),
                    perm: worldos_kernel::known::permissions::FILESYSTEM_READ.into(),
                });
            }
            std::fs::read(f).map_err(|e| CommandError::Failed(format!("cannot read `{f}`: {e}")))?
        } else {
            return Err(CommandError::Failed(
                "provide `step` (artifact ref) or `file` (path)".into(),
            ));
        };

        let shape = self
            .services
            .kernel
            .import_step(&bytes)
            .map_err(|e| CommandError::Failed(format!("step import failed: {e}")))?;
        let name = input
            .get("name")
            .and_then(|n| n.as_str())
            .map(String::from)
            .unwrap_or_else(|| auto_name(ctx, "import"));
        finalize_shape(
            ctx,
            &self.services,
            shape,
            "import_step",
            json!({"bytes": bytes.len(), "source": input.get("step").cloned().or_else(|| input.get("file").cloned()).unwrap_or(Value::Null)}),
            "cad.import_step",
            ShapeMeta {
                name,
                position: json!([0, 0, 0]),
                parent: input.get("parent").cloned(),
                sources: vec![],
            },
        )
    }
}

/// CAD handlers that need kernel+artifact services, for
/// `Engine::attach_cad`.
pub fn cad_handlers(services: Arc<CadServices>) -> Vec<Arc<dyn CommandHandler>> {
    vec![
        Arc::new(CadCreateBox::new(services.clone())),
        Arc::new(CadCreateCylinder::new(services.clone())),
        Arc::new(CadCreateSphere::new(services.clone())),
        Arc::new(CadBoolean::new(services.clone())),
        Arc::new(CadFillet::new(services.clone())),
        Arc::new(CadChamfer::new(services.clone())),
        Arc::new(CadTransform::new(services.clone())),
        Arc::new(CadMeasure::new(services.clone())),
        Arc::new(CadExportStep::new(services.clone())),
        Arc::new(CadExportStl::new(services.clone())),
        Arc::new(CadImportStep::new(services)),
    ]
}
