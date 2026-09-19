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

    /// Copy every blob from the current store into `store`
    /// (content-addressed copies are idempotent). Does NOT switch the
    /// binding — pair with [`Self::set_store`] once the caller's commit
    /// point has passed.
    pub fn migrate_to(
        &self,
        store: &ArtifactStore,
    ) -> Result<u64, worldos_artifact::ArtifactError> {
        let old = self.artifacts();
        let mut copied = 0u64;
        for r in old.list()? {
            let bytes = old.get(&r)?;
            if store.put(&bytes)?.written {
                copied += 1;
            }
        }
        Ok(copied)
    }

    /// Swap the bound store. Callers must ensure `store` already holds
    /// every live blob (see [`Self::migrate_to`]).
    pub fn set_store(&self, store: ArtifactStore) {
        *self.store.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(store);
    }

    /// Point `services` at `store`, migrating every blob from the
    /// current store first (content-addressed copies are idempotent).
    pub fn rebind(&self, store: ArtifactStore) -> Result<(), worldos_artifact::ArtifactError> {
        self.migrate_to(&store)?;
        self.set_store(store);
        Ok(())
    }

    /// Tessellate a stored BRep artifact for viewport display. The
    /// kernel handle exists only for the duration of this call.
    pub fn mesh_for_brep(
        &self,
        brep: &worldos_artifact::ArtifactRef,
        params: TessParams,
    ) -> Result<worldos_cad::MeshData, worldos_cad::CadError> {
        let bytes = self.artifacts().get(brep).map_err(|e| {
            worldos_cad::CadError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("brep artifact {brep}: {e}"),
            ))
        })?;
        let shape = self.kernel.import_brep(&bytes)?;
        let mesh = self.kernel.mesh(shape, params);
        self.kernel.drop_shape(shape);
        mesh
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

// ---- dependency graph helpers ------------------------------------------
//
// `core:derived-from` edges point dependent → source. They are the
// authoritative feature-tree lineage; recipe params (`a`, `b`, `source`)
// are kept in sync with them.

/// Object ids a recipe reads geometry from.
fn op_sources(op: &worldos_cad::CadOperation) -> Vec<worldos_kernel::ids::ObjectId> {
    let p = &op.params;
    let id = |k: &str| {
        p.get(k)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
    };
    match op.kind.as_str() {
        "boolean" => [id("a"), id("b")].into_iter().flatten().collect(),
        "fillet" | "chamfer" | "transform" => id("source").into_iter().collect(),
        _ => vec![],
    }
}

/// Recipe keys that carry object references — `cad.set_param` treats
/// patching them as a structural retarget (edges resync + cycle check),
/// not a plain value merge.
fn is_source_param_key(kind: &str, key: &str) -> bool {
    match kind {
        "boolean" => matches!(key, "a" | "b"),
        "fillet" | "chamfer" | "transform" => key == "source",
        _ => false,
    }
}

/// Direct source edges of `id` (dependent → source), sorted by relation
/// id for determinism.
fn source_edges_of(
    project: &worldos_kernel::project::Project,
    id: worldos_kernel::ids::ObjectId,
) -> Vec<worldos_kernel::model::Relation> {
    let mut v: Vec<_> = project
        .relations_from(id)
        .filter(|r| r.type_id == worldos_kernel::known::rel::DERIVED_FROM)
        .cloned()
        .collect();
    v.sort_by_key(|r| r.id);
    v
}

/// Every object downstream of `root` (transitive dependents), BFS in
/// deterministic order — cycle-safe via the visited set.
fn transitive_dependents(
    project: &worldos_kernel::project::Project,
    root: worldos_kernel::ids::ObjectId,
) -> Vec<worldos_kernel::ids::ObjectId> {
    use std::collections::BTreeSet;
    use worldos_kernel::ids::ObjectId;
    let mut seen: BTreeSet<ObjectId> = [root].into_iter().collect();
    let mut out = Vec::new();
    let mut frontier = vec![root];
    while !frontier.is_empty() {
        let mut next: BTreeSet<ObjectId> = BTreeSet::new();
        for id in &frontier {
            for r in project.relations_to(*id) {
                if r.type_id == worldos_kernel::known::rel::DERIVED_FROM && seen.insert(r.from) {
                    next.insert(r.from);
                    out.push(r.from);
                }
            }
        }
        frontier = next.into_iter().collect();
    }
    out
}

/// Ancestors of `id` — every object it transitively depends on.
fn ancestors(
    project: &worldos_kernel::project::Project,
    id: worldos_kernel::ids::ObjectId,
) -> std::collections::BTreeSet<worldos_kernel::ids::ObjectId> {
    use std::collections::BTreeSet;
    use worldos_kernel::ids::ObjectId;
    let mut seen = BTreeSet::new();
    let mut frontier = vec![id];
    while !frontier.is_empty() {
        let mut next = Vec::<ObjectId>::new();
        for cur in frontier {
            for r in source_edges_of(project, cur) {
                if seen.insert(r.to) {
                    next.push(r.to);
                }
            }
        }
        frontier = next;
    }
    seen
}

/// Name a body for error messages.
fn name_of(
    project: &worldos_kernel::project::Project,
    id: worldos_kernel::ids::ObjectId,
) -> String {
    project
        .get(id)
        .map(|o| o.name.clone())
        .unwrap_or_else(|| id.to_string())
}

/// Topological order over `scope` restricted to derived-from edges.
/// Deterministic (ids sorted per ready level); fails on a cycle.
fn topo_order(
    project: &worldos_kernel::project::Project,
    scope: &std::collections::BTreeSet<worldos_kernel::ids::ObjectId>,
) -> Result<Vec<worldos_kernel::ids::ObjectId>, CommandError> {
    use std::collections::BTreeSet;
    use worldos_kernel::ids::ObjectId;
    let mut emitted: BTreeSet<ObjectId> = BTreeSet::new();
    let mut out = Vec::new();
    loop {
        let ready: Vec<ObjectId> = scope
            .iter()
            .copied()
            .filter(|id| !emitted.contains(id))
            .filter(|id| {
                source_edges_of(project, *id)
                    .iter()
                    .all(|r| !scope.contains(&r.to) || emitted.contains(&r.to))
            })
            .collect();
        if ready.is_empty() {
            if out.len() == scope.len() {
                return Ok(out);
            }
            let names: Vec<String> = scope
                .iter()
                .filter(|id| !emitted.contains(id))
                .map(|id| name_of(project, *id))
                .collect();
            return Err(CommandError::Failed(format!(
                "dependency cycle among cad:bodies: {}",
                names.join(", ")
            )));
        }
        for id in ready {
            emitted.insert(id);
            out.push(id);
        }
    }
}

/// Is `id`'s `cad:shape` flagged stale?
fn is_stale(project: &worldos_kernel::project::Project, id: worldos_kernel::ids::ObjectId) -> bool {
    project
        .get(id)
        .and_then(|o| o.component_data(components::CAD_SHAPE))
        .and_then(|d| d.get("stale"))
        .and_then(|s| s.as_bool())
        .unwrap_or(false)
}

/// Parse + validate the `cad:operation` recipe of `id`.
fn recipe_of(
    ctx: &CommandContext,
    id: worldos_kernel::ids::ObjectId,
) -> Result<worldos_cad::CadOperation, CommandError> {
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
    let op: worldos_cad::CadOperation = serde_json::from_value(
        obj.component_data(components::CAD_OPERATION)
            .ok_or_else(|| CommandError::Failed("no cad:operation component".into()))?
            .clone(),
    )
    .map_err(|e| CommandError::Failed(format!("bad cad:operation: {e}")))?;
    if op.v != worldos_cad::CAD_SCHEMA_VERSION {
        return Err(CommandError::Failed(format!(
            "recipe schema v{} is unsupported (this build speaks v{})",
            op.v,
            worldos_cad::CAD_SCHEMA_VERSION
        )));
    }
    Ok(op)
}

/// Fail when any direct source of `id`'s recipe is itself stale — the
/// input version no longer matches what the recipe was authored against.
fn check_sources_fresh(
    ctx: &CommandContext,
    op: &worldos_cad::CadOperation,
) -> Result<(), CommandError> {
    let stale: Vec<String> = op_sources(op)
        .iter()
        .filter(|s| is_stale(ctx.project, **s))
        .map(|s| name_of(ctx.project, *s))
        .collect();
    if stale.is_empty() {
        Ok(())
    } else {
        Err(CommandError::Failed(format!(
            "stale upstream input(s): {} — regenerate them first (or run cad.regenerate with all_stale)",
            stale.join(", ")
        )))
    }
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
    let (sid, state) = load_shape_id(ctx, services, id)?;
    Ok((id, sid, state))
}

/// Load a `cad:body` by id into the kernel (the resolution-agnostic
/// half of [`load_shape`], used by recipe replay).
fn load_shape_id(
    ctx: &CommandContext,
    services: &CadServices,
    id: worldos_kernel::ids::ObjectId,
) -> Result<(ShapeId, worldos_cad::CadShape), CommandError> {
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
    Ok((sid, state))
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

        // STEP source is persisted as an artifact so the recipe is
        // regenerable even when the original was a filesystem path.
        let step_ref = self
            .services
            .artifacts()
            .put(&bytes)
            .map_err(|e| CommandError::Failed(format!("step artifact store: {e}")))?
            .artifact_ref
            .to_string();
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
            json!({
                "step": step_ref,
                "source_file": input.get("file").cloned().unwrap_or(Value::Null),
            }),
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

/// Replay a `cad:operation` recipe through the kernel. Caller owns
/// the returned `ShapeId`. Regeneration reads CURRENT source BReps —
/// if a source is itself stale, the result reflects that stale
/// geometry (deep topological replay is v2).
fn regen(
    ctx: &CommandContext,
    services: &CadServices,
    op: &worldos_cad::CadOperation,
) -> Result<ShapeId, CommandError> {
    if op.kernel != services.kernel.name() {
        return Err(CommandError::Failed(format!(
            "recipe requires kernel `{}`, attached is `{}` — refusing to guess",
            op.kernel,
            services.kernel.name()
        )));
    }
    let k = &*services.kernel;
    let p = &op.params;
    let shape = match op.kind.as_str() {
        "create_box" => {
            let [x, y, z] = vec3(p, "size_mm")?;
            k.make_box(x, y, z)
        }
        "create_cylinder" => {
            let r = scalar(p, "radius_mm")?;
            let h = scalar(p, "height_mm")?;
            k.make_cylinder(r, h)
        }
        "create_sphere" => k.make_sphere(scalar(p, "radius_mm")?),
        "boolean" => {
            let a_id: worldos_kernel::ids::ObjectId = p["a"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| CommandError::Failed("boolean recipe missing `a`".into()))?;
            let b_id: worldos_kernel::ids::ObjectId = p["b"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| CommandError::Failed("boolean recipe missing `b`".into()))?;
            let bool_op = match p["op"].as_str().unwrap_or_default() {
                "union" => worldos_cad::BoolOp::Union,
                "subtract" => worldos_cad::BoolOp::Subtract,
                "intersect" => worldos_cad::BoolOp::Intersect,
                o => return Err(CommandError::Failed(format!("bad boolean op `{o}`"))),
            };
            let (a, _) = load_shape_id(ctx, services, a_id)?;
            let (b, _) = load_shape_id(ctx, services, b_id)?;
            let out = k.boolean(a, b, bool_op);
            k.drop_shape(a);
            k.drop_shape(b);
            out
        }
        "fillet" | "chamfer" => {
            let src: worldos_kernel::ids::ObjectId = p["source"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| CommandError::Failed("recipe missing `source`".into()))?;
            let (sid, src_state) = load_shape_id(ctx, services, src)?;
            let edge_ids: Vec<u64> = p
                .get("edge_ids")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_u64()).collect())
                .unwrap_or_default();
            // Edge ids are kernel topology handles, not semantic names:
            // after the source regenerated, a stored id that no longer
            // resolves is a STALE SELECTION — refuse instead of silently
            // picking whatever edge now sits at that ordinal.
            let live: std::collections::HashSet<u64> =
                src_state.topology.edge_ids.iter().copied().collect();
            let gone: Vec<u64> = edge_ids.iter().copied().filter(|e| !live.contains(e)).collect();
            if !gone.is_empty() {
                k.drop_shape(sid);
                return Err(CommandError::Failed(format!(
                    "stale edge selection on `{}`: id(s) {} no longer exist on the regenerated source — reselect edges",
                    name_of(ctx.project, src),
                    gone.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ")
                )));
            }
            let out = if op.kind == "fillet" {
                k.fillet(sid, scalar(p, "radius_mm")?, &edge_ids)
            } else {
                k.chamfer(sid, scalar(p, "distance_mm")?, &edge_ids)
            };
            k.drop_shape(sid);
            out
        }
        "transform" => {
            let src: worldos_kernel::ids::ObjectId = p["source"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| CommandError::Failed("recipe missing `source`".into()))?;
            let ops: Vec<worldos_cad::TransformOp> = serde_json::from_value(p["ops"].clone())
                .map_err(|e| CommandError::Failed(format!("bad transform ops: {e}")))?;
            let (sid, _) = load_shape_id(ctx, services, src)?;
            let out = k.transform(sid, &ops);
            k.drop_shape(sid);
            out
        }
        "import_step" => {
            let r = p["step"]
                .as_str()
                .ok_or_else(|| CommandError::Failed("import recipe missing `step` ref".into()))?;
            let aref: worldos_artifact::ArtifactRef =
                r.parse().map_err(|e: worldos_artifact::ArtifactError| {
                    CommandError::Failed(format!("bad step ref: {e}"))
                })?;
            let bytes = services
                .artifacts()
                .get(&aref)
                .map_err(|e| CommandError::Failed(format!("step artifact: {e}")))?;
            k.import_step(&bytes)
        }
        other => {
            return Err(CommandError::Failed(format!(
                "recipe kind `{other}` is not regenerable"
            )));
        }
    }
    .map_err(|e| CommandError::Failed(format!("regen `{}` failed: {e}", op.kind)))?;

    // position bake (create recipes carry it in params)
    let delta = p.get("position").and_then(|v| v.as_array()).and_then(|a| {
        if a.len() != 3 {
            return None;
        }
        let d = [a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?];
        d.iter().any(|x| x.abs() > 0.0).then_some(d)
    });
    if let Some(delta) = delta {
        let moved = k
            .transform(
                shape,
                &[worldos_cad::TransformOp::Translate { delta_mm: delta }],
            )
            .map_err(|e| CommandError::Failed(format!("position bake failed: {e}")))?;
        k.drop_shape(shape);
        return Ok(moved);
    }
    Ok(shape)
}

/// Regenerate `id` from its stored recipe and rewrite `cad:shape` +
/// `cad:operation` in place. All kernel work happens BEFORE any graph
/// write — a kernel failure leaves the object untouched. Old STEP/STL
/// refs are dropped (they describe stale geometry). Every transitive
/// dependent (`core:derived-from` chains) is flagged stale.
fn regen_object(
    ctx: &mut CommandContext,
    services: &CadServices,
    id: worldos_kernel::ids::ObjectId,
    op: worldos_cad::CadOperation,
) -> Result<worldos_cad::CadShape, CommandError> {
    check_sources_fresh(ctx, &op)?;
    let shape = regen(ctx, services, &op)?;
    let k = &services.kernel;
    let topology = k
        .topology(shape)
        .map_err(|e| CommandError::Failed(format!("topology: {e}")))?;
    let measures = k
        .measure(shape)
        .map_err(|e| CommandError::Failed(format!("measure: {e}")))?;
    let brep_bytes = k
        .export_brep(shape)
        .map_err(|e| CommandError::Failed(format!("brep export: {e}")))?;
    k.drop_shape(shape);
    if !topology.is_valid {
        return Err(CommandError::Failed(
            "regenerated shape is invalid (non-positive volume or no topology)".into(),
        ));
    }
    let put = services
        .artifacts()
        .put(&brep_bytes)
        .map_err(|e| CommandError::Failed(format!("artifact store: {e}")))?;

    let mut state = worldos_cad::CadShape::new(
        put.artifact_ref.to_string(),
        k.name(),
        "cad.regenerate",
        measures,
        topology,
    );

    // everything below is graph writes — kernel work is done
    ctx.update_object(id, |obj| {
        if let Ok(v) = serde_json::to_value(&op) {
            obj.set_component(worldos_kernel::model::Component::new(
                components::CAD_OPERATION,
                v,
            ));
        }
        if let Ok(v) = serde_json::to_value(&state) {
            obj.set_component(worldos_kernel::model::Component::new(
                components::CAD_SHAPE,
                v,
            ));
        }
    })?;

    // Transitive staleness: every downstream dependent is flagged, not
    // just direct children — a grandchild built on a regenerated parent
    // is stale even if its own recipe params did not change.
    let dependents = transitive_dependents(ctx.project, id);
    for dep in dependents {
        ctx.update_object(dep, |obj| {
            if let Some(data) = obj.component_data(components::CAD_SHAPE)
                && let Ok(mut s) = serde_json::from_value::<worldos_cad::CadShape>(data.clone())
            {
                s.stale = true;
                if let Ok(v) = serde_json::to_value(&s) {
                    obj.set_component(worldos_kernel::model::Component::new(
                        components::CAD_SHAPE,
                        v,
                    ));
                }
            }
        })?;
    }
    state.stale = false; // report the post-write truth
    Ok(state)
}

/// `cad.set_param` — merge params into the `cad:operation` recipe and
/// regenerate the shape in place (object identity preserved).
pub struct CadSetParam {
    services: Arc<CadServices>,
}

impl CadSetParam {
    pub fn new(services: Arc<CadServices>) -> Self {
        Self { services }
    }
}

impl CommandHandler for CadSetParam {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "cad.set_param",
            "cad",
            "Merge params into a cad:body's recipe and regenerate in place; dependents go stale",
            props::object(
                &["object", "params"],
                json!({
                    "object": {"type": "string", "description": "id or name"},
                    "params": {"type": "object", "description": "recipe fields to merge, e.g. {\"size_mm\":[60,40,20]}"}
                }),
            ),
        )
    }

    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        let target = input
            .get("object")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CommandError::Failed("missing `object`".into()))?;
        let lookup = if target.parse::<worldos_kernel::ids::ObjectId>().is_ok() {
            json!({"id": target})
        } else {
            json!({"name": target})
        };
        let id = crate::builtin::resolve_object(ctx, &lookup)?;
        let mut op = recipe_of(ctx, id)?;

        // shallow-merge provided params into the recipe
        let patch = input["params"]
            .as_object()
            .ok_or_else(|| CommandError::Failed("`params` must be an object".into()))?;
        let retargeted = patch.keys().any(|k| is_source_param_key(&op.kind, k));
        {
            let base = op
                .params
                .as_object_mut()
                .ok_or_else(|| CommandError::Failed("recipe params not an object".into()))?;
            for (key, val) in patch {
                base.insert(key.clone(), val.clone());
            }
        }

        // A patch touching source refs is a structural retarget: resync
        // the `core:derived-from` edges (the authoritative lineage) and
        // reject retargets that would close a dependency cycle.
        if retargeted {
            sync_derived_edges(ctx, id, &op)?;
        }

        let state = regen_object(ctx, &self.services, id, op)?;
        Ok(json!({
            "id": id.to_string(),
            "brep": state.brep,
            "measures": serde_json::to_value(state.measures).unwrap_or(Value::Null),
            "topology": serde_json::to_value(&state.topology).unwrap_or(Value::Null),
        }))
    }
}

/// Make the `core:derived-from` edges of `id` match the recipe's source
/// refs after a `set_param` retarget. Rejects dangling targets, non-body
/// sources, and cycles — all inside the same transaction, so a failure
/// leaves the graph untouched.
fn sync_derived_edges(
    ctx: &mut CommandContext,
    id: worldos_kernel::ids::ObjectId,
    op: &worldos_cad::CadOperation,
) -> Result<(), CommandError> {
    use std::collections::BTreeSet;
    use worldos_kernel::ids::ObjectId;
    let want: BTreeSet<ObjectId> = op_sources(op).into_iter().collect();
    for s in &want {
        let obj = ctx
            .project
            .get(*s)
            .ok_or_else(|| CommandError::Failed(format!("retarget source {s} does not exist")))?;
        if obj.type_id.0 != types::CAD_BODY {
            return Err(CommandError::Failed(format!(
                "retarget source `{}` is a `{}`, not a cad:body",
                obj.name, obj.type_id
            )));
        }
        if *s == id || ancestors(ctx.project, *s).contains(&id) {
            return Err(CommandError::Failed(format!(
                "retarget would close a dependency cycle: `{}` already depends on `{}`",
                name_of(ctx.project, *s),
                name_of(ctx.project, id)
            )));
        }
    }
    let cur: Vec<worldos_kernel::model::Relation> = source_edges_of(ctx.project, id);
    let cur_targets: BTreeSet<ObjectId> = cur.iter().map(|r| r.to).collect();
    for r in cur.iter().filter(|r| !want.contains(&r.to)) {
        ctx.remove_relation(r.id)?;
    }
    for s in want.iter().filter(|s| !cur_targets.contains(*s)) {
        ctx.put_relation(worldos_kernel::model::Relation::new(
            worldos_kernel::known::rel::DERIVED_FROM,
            id,
            *s,
            &ctx.actor.id,
        ))?;
    }
    Ok(())
}

/// `cad.regenerate` — replay a body's stored recipe (used after a
/// source changed and the object was flagged stale).
pub struct CadRegenerate {
    services: Arc<CadServices>,
}

impl CadRegenerate {
    pub fn new(services: Arc<CadServices>) -> Self {
        Self { services }
    }
}

impl CommandHandler for CadRegenerate {
    fn schema(&self) -> CommandSchema {
        CommandSchema::write(
            "cad.regenerate",
            "cad",
            "Replay cad:body recipes — single object, cascade over its dependents, or all stale bodies (topological order, one atomic transaction)",
            props::object(
                &[],
                json!({
                    "object": {"type": "string", "description": "id or name"},
                    "cascade": {"type": "boolean", "description": "also regenerate every transitive dependent of `object`"},
                    "all_stale": {"type": "boolean", "description": "regenerate every stale cad:body in topological order"}
                }),
            ),
        )
    }

    fn execute(&self, ctx: &mut CommandContext, input: &Value) -> Result<Value, CommandError> {
        use std::collections::BTreeSet;
        use worldos_kernel::ids::ObjectId;

        let all_stale = input
            .get("all_stale")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let cascade = input
            .get("cascade")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Compute the deterministic topological scope to rebuild.
        let scope: BTreeSet<ObjectId> = if all_stale {
            ctx.project
                .objects_of_type(types::CAD_BODY)
                .filter(|o| is_stale(ctx.project, o.id))
                .map(|o| o.id)
                .collect()
        } else {
            let target = input
                .get("object")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    CommandError::Failed("missing `object` (or pass `all_stale: true`)".into())
                })?;
            let lookup = if target.parse::<ObjectId>().is_ok() {
                json!({"id": target})
            } else {
                json!({"name": target})
            };
            let id = crate::builtin::resolve_object(ctx, &lookup)?;
            let mut scope = BTreeSet::from([id]);
            if cascade {
                scope.extend(transitive_dependents(ctx.project, id));
            }
            scope
        };

        // Whole batch verified before any mutation commits: this command
        // runs inside one transaction — a mid-chain failure rolls back
        // every graph write, so no body reports fresh on stale inputs.
        let order = topo_order(ctx.project, &scope)?;
        let mut results = Vec::new();
        for id in order {
            let op = recipe_of(ctx, id)?;
            let state = regen_object(ctx, &self.services, id, op)?;
            results.push(json!({
                "id": id.to_string(),
                "name": name_of(ctx.project, id),
                "brep": state.brep,
                "measures": serde_json::to_value(state.measures).unwrap_or(Value::Null),
                "topology": serde_json::to_value(&state.topology).unwrap_or(Value::Null),
            }));
        }
        let single = results.len() == 1 && !all_stale && !cascade;
        Ok(if single {
            results.pop().unwrap_or(json!({}))
        } else {
            json!({"regenerated": results, "count": results.len()})
        })
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
        Arc::new(CadImportStep::new(services.clone())),
        Arc::new(CadSetParam::new(services.clone())),
        Arc::new(CadRegenerate::new(services)),
    ]
}
