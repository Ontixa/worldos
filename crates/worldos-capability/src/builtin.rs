//! Builtin capabilities: inspection, search, validation, export, measure.

use crate::descriptor::CapabilityDescriptor;
use crate::error::CapabilityError;
use crate::host::CapabilityHost;
use crate::registry::Capability;
use serde_json::{json, Value};
use worldos_kernel::known::{components, permissions};

fn fail(e: impl std::fmt::Display) -> CapabilityError {
    CapabilityError::Failed(e.to_string())
}

// ---------------------------------------------------------------- inspect

pub struct ProjectInspect;

impl Capability for ProjectInspect {
    fn descriptor(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::new(
            "project.inspect",
            "Project summary: counts, type histogram, roots",
            json!({"type": "object"}),
        )
        .requires(&[permissions::PROJECT_READ])
        .deterministic()
    }
    fn execute(&self, host: &mut dyn CapabilityHost, _i: &Value) -> Result<Value, CapabilityError> {
        let p = host.project();
        let mut types = serde_json::Map::new();
        for o in p.objects.values() {
            *types.entry(o.type_id.0.clone()).or_insert(json!(0)) =
                json!(types.get(&o.type_id.0).and_then(|v| v.as_i64()).unwrap_or(0) + 1);
        }
        Ok(json!({
            "id": p.id.to_string(),
            "name": p.name,
            "schema_version": p.schema_version,
            "object_count": p.objects.len(),
            "relation_count": p.relations.len(),
            "types": types,
            "roots": p.roots().iter().map(|o| json!({"id": o.id.to_string(), "name": o.name, "type": o.type_id})).collect::<Vec<_>>(),
        }))
    }
}

// ---------------------------------------------------------------- search

pub struct ProjectSearch;

impl Capability for ProjectSearch {
    fn descriptor(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::new(
            "project.search",
            "Structured search over objects (text, type, tag, component)",
            json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string"}, "type_id": {"type": "string"},
                    "tag": {"type": "string"}, "has_component": {"type": "string"},
                    "limit": {"type": "integer"}
                }
            }),
        )
        .requires(&[permissions::PROJECT_SEARCH])
        .deterministic()
    }
    fn execute(&self, host: &mut dyn CapabilityHost, input: &Value) -> Result<Value, CapabilityError> {
        let q: worldos_kernel::SearchQuery =
            serde_json::from_value(input.clone()).map_err(fail)?;
        let results: Vec<Value> = worldos_kernel::search(host.project(), &q)
            .into_iter()
            .map(|o| {
                json!({
                    "id": o.id.to_string(), "name": o.name, "type": o.type_id,
                    "tags": o.tags, "components": o.components.keys().collect::<Vec<_>>(),
                })
            })
            .collect();
        Ok(json!({"results": results, "count": results.len()}))
    }
}

// ---------------------------------------------------------------- validate

pub struct ValidationRun;

impl Capability for ValidationRun {
    fn descriptor(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::new(
            "validation.run",
            "Run all validators, return structured diagnostics",
            json!({"type": "object"}),
        )
        .requires(&[permissions::VALIDATION_RUN])
        .deterministic()
    }
    fn execute(&self, host: &mut dyn CapabilityHost, _i: &Value) -> Result<Value, CapabilityError> {
        Ok(serde_json::to_value(host.validate()?).map_err(fail)?)
    }
}

// ---------------------------------------------------------------- export

pub struct ArtifactExport;

impl Capability for ArtifactExport {
    fn descriptor(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::new(
            "artifact.export",
            "Export the project model to a JSON file",
            json!({
                "type": "object",
                "required": ["path"],
                "properties": {"path": {"type": "string"}}
            }),
        )
        .requires(&[permissions::ARTIFACT_EXPORT, permissions::FILESYSTEM_WRITE])
    }
    fn execute(&self, host: &mut dyn CapabilityHost, input: &Value) -> Result<Value, CapabilityError> {
        let path = input["path"].as_str().ok_or_else(|| fail("missing path"))?;
        // Path traversal guard: refuse absolute escapes and `..` segments
        // when a sandbox root is configured later; for now require the
        // caller-supplied path verbatim but block obvious traversal.
        if path.split(['/', '\\']).any(|seg| seg == "..") {
            return Err(fail("path traversal (`..`) is not allowed"));
        }
        let json = serde_json::to_string_pretty(host.project()).map_err(fail)?;
        std::fs::write(path, &json)?;
        Ok(json!({"path": path, "bytes": json.len()}))
    }
}

// ---------------------------------------------------------------- measure

pub struct GeometryMeasure;

impl Capability for GeometryMeasure {
    fn descriptor(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::new(
            "geometry.measure",
            "Compute bounding box, volume and surface area of a primitive",
            json!({
                "type": "object",
                "properties": {"id": {"type": "string"}, "name": {"type": "string"}}
            }),
        )
        .requires(&[permissions::PROJECT_READ])
        .deterministic()
    }
    fn execute(&self, host: &mut dyn CapabilityHost, input: &Value) -> Result<Value, CapabilityError> {
        let key = input
            .get("id")
            .or_else(|| input.get("name"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| fail("provide id or name"))?;
        let oid = host
            .resolve_object(key)
            .ok_or_else(|| fail(format!("object `{key}` not found")))?;
        let obj = host.project().get(oid).ok_or_else(|| fail("gone"))?;
        let geom = obj.component_data(components::GEOMETRY).cloned().unwrap_or(json!({}));
        let xf = obj.component_data(components::TRANSFORM).cloned().unwrap_or(json!({}));
        let kind = geom.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        let scale = vec3(&xf["scale"], [1.0, 1.0, 1.0]);
        let dims = size_dims(&geom["size"], scale);
        let (volume, area) = measure(kind, dims);
        Ok(json!({
            "id": oid.to_string(),
            "kind": kind,
            "dimensions": dims,
            "bbox": {"min": bbox_min(&xf, dims), "max": bbox_max(&xf, dims)},
            "volume": volume,
            "surface_area": area,
        }))
    }
}

fn vec3(v: &Value, default: [f64; 3]) -> [f64; 3] {
    let g = |i: usize| v.get(i).and_then(|x| x.as_f64());
    [g(0).unwrap_or(default[0]), g(1).unwrap_or(default[1]), g(2).unwrap_or(default[2])]
}

/// Interpret `size` (scalar or vec) × scale as [x,y,z] dims.
fn size_dims(size: &Value, scale: [f64; 3]) -> [f64; 3] {
    match size {
        Value::Number(n) => {
            let s = n.as_f64().unwrap_or(1.0);
            [s * scale[0], s * scale[1], s * scale[2]]
        }
        Value::Array(_) => {
            let v = vec3(size, [1.0, 1.0, 1.0]);
            [v[0] * scale[0], v[1] * scale[1], v[2] * scale[2]]
        }
        _ => [scale[0], scale[1], scale[2]],
    }
}

fn measure(kind: &str, d: [f64; 3]) -> (f64, f64) {
    match kind {
        "sphere" => {
            let r = d[0] / 2.0;
            (4.0 / 3.0 * std::f64::consts::PI * r.powi(3), 4.0 * std::f64::consts::PI * r * r)
        }
        "cylinder" => {
            let r = d[0] / 2.0;
            let h = d[2];
            (
                std::f64::consts::PI * r * r * h,
                2.0 * std::f64::consts::PI * r * (r + h),
            )
        }
        "plane" => (0.0, d[0] * d[1]),
        _ => (d[0] * d[1] * d[2], 2.0 * (d[0] * d[1] + d[1] * d[2] + d[0] * d[2])),
    }
}

fn bbox_min(xf: &Value, d: [f64; 3]) -> [f64; 3] {
    let p = vec3(&xf["position"], [0.0, 0.0, 0.0]);
    [p[0] - d[0] / 2.0, p[1] - d[1] / 2.0, p[2] - d[2] / 2.0]
}
fn bbox_max(xf: &Value, d: [f64; 3]) -> [f64; 3] {
    let p = vec3(&xf["position"], [0.0, 0.0, 0.0]);
    [p[0] + d[0] / 2.0, p[1] + d[1] / 2.0, p[2] + d[2] / 2.0]
}

/// All builtin capabilities.
pub fn builtins() -> Vec<std::sync::Arc<dyn Capability>> {
    vec![
        std::sync::Arc::new(ProjectInspect),
        std::sync::Arc::new(ProjectSearch),
        std::sync::Arc::new(ValidationRun),
        std::sync::Arc::new(ArtifactExport),
        std::sync::Arc::new(GeometryMeasure),
    ]
}
