//! Builtin capabilities: inspection, search, validation, export, measure.

use crate::descriptor::CapabilityDescriptor;
use crate::error::CapabilityError;
use crate::host::CapabilityHost;
use crate::registry::Capability;
use serde_json::{Value, json};
use worldos_kernel::known::permissions;

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
            *types.entry(o.type_id.0.clone()).or_insert(json!(0)) = json!(
                types
                    .get(&o.type_id.0)
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
                    + 1
            );
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
    fn execute(
        &self,
        host: &mut dyn CapabilityHost,
        input: &Value,
    ) -> Result<Value, CapabilityError> {
        let q: worldos_kernel::SearchQuery = serde_json::from_value(input.clone()).map_err(fail)?;
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
        serde_json::to_value(host.validate()?).map_err(fail)
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
    fn execute(
        &self,
        host: &mut dyn CapabilityHost,
        input: &Value,
    ) -> Result<Value, CapabilityError> {
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
    fn execute(
        &self,
        host: &mut dyn CapabilityHost,
        input: &Value,
    ) -> Result<Value, CapabilityError> {
        let key = input
            .get("id")
            .or_else(|| input.get("name"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| fail("provide id or name"))?;
        let oid = host
            .resolve_object(key)
            .ok_or_else(|| fail(format!("object `{key}` not found")))?;
        let obj = host.project().get(oid).ok_or_else(|| fail("gone"))?;
        use worldos_kernel::measure as m;
        let kind = m::object_kind(obj).unwrap_or("").to_string();
        let dims = m::object_dims(obj).unwrap_or([1.0, 1.0, 1.0]);
        let (volume, area) = m::measure_primitive(&kind, dims);
        let (bmin, bmax) = m::object_bbox(obj).unwrap_or(([0.0; 3], [0.0; 3]));
        Ok(json!({
            "id": oid.to_string(),
            "kind": kind,
            "dimensions": dims,
            "bbox": {"min": bmin, "max": bmax},
            "volume": volume,
            "surface_area": area,
        }))
    }
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
