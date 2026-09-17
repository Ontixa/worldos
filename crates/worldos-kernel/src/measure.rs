//! Geometry measurement over the semantic model.
//!
//! Primitives carry analytic shape data (`geom:geometry.size`,
//! `core:transform.scale`) so measurement needs no mesh kernel — the same
//! numbers back the `geometry.measure` capability, requirement terms like
//! `volume(x)` / `distance(a,b)`, and future validators.

use crate::known::components;
use crate::model::Object;
use serde_json::Value;

/// `[x,y,z]` effective dimensions of an object's geometry (size × scale).
/// `None` when the object has no geometry component.
pub fn object_dims(obj: &Object) -> Option<[f64; 3]> {
    let geom = obj.component_data(components::GEOMETRY)?;
    let xf = obj.component_data(components::TRANSFORM);
    let scale = xf
        .map(|t| vec3(&t["scale"], [1.0, 1.0, 1.0]))
        .unwrap_or([1.0, 1.0, 1.0]);
    Some(size_dims(&geom["size"], scale))
}

/// World-space position from `core:transform.position`.
pub fn object_position(obj: &Object) -> [f64; 3] {
    obj.component_data(components::TRANSFORM)
        .map(|t| vec3(&t["position"], [0.0, 0.0, 0.0]))
        .unwrap_or([0.0, 0.0, 0.0])
}

/// Euclidean distance between two objects' positions.
pub fn distance(a: &Object, b: &Object) -> f64 {
    let pa = object_position(a);
    let pb = object_position(b);
    ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2) + (pa[2] - pb[2]).powi(2)).sqrt()
}

/// `(volume, surface_area)` for a primitive kind at the given dimensions.
/// Unknown kinds are treated as boxes — permissive so custom `geom:*`
/// types still measure.
pub fn measure_primitive(kind: &str, d: [f64; 3]) -> (f64, f64) {
    match kind {
        "sphere" => {
            let r = d[0] / 2.0;
            (
                4.0 / 3.0 * std::f64::consts::PI * r.powi(3),
                4.0 * std::f64::consts::PI * r * r,
            )
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
        _ => (
            d[0] * d[1] * d[2],
            2.0 * (d[0] * d[1] + d[1] * d[2] + d[0] * d[2]),
        ),
    }
}

/// Axis-aligned bounding box `[min, max]` around the object's position.
pub fn object_bbox(obj: &Object) -> Option<([f64; 3], [f64; 3])> {
    let d = object_dims(obj)?;
    let p = object_position(obj);
    Some((
        [p[0] - d[0] / 2.0, p[1] - d[1] / 2.0, p[2] - d[2] / 2.0],
        [p[0] + d[0] / 2.0, p[1] + d[1] / 2.0, p[2] + d[2] / 2.0],
    ))
}

/// The `geom:geometry.kind` string, if present.
pub fn object_kind(obj: &Object) -> Option<&str> {
    obj.component_data(components::GEOMETRY)
        .and_then(|g| g.get("kind"))
        .and_then(|k| k.as_str())
}

pub fn vec3(v: &Value, default: [f64; 3]) -> [f64; 3] {
    let g = |i: usize| v.get(i).and_then(|x| x.as_f64());
    [
        g(0).unwrap_or(default[0]),
        g(1).unwrap_or(default[1]),
        g(2).unwrap_or(default[2]),
    ]
}

/// Interpret `size` (scalar or vec) × scale as `[x,y,z]` dims.
pub fn size_dims(size: &Value, scale: [f64; 3]) -> [f64; 3] {
    match size {
        Value::Number(n) => {
            let s = n.as_f64().unwrap_or(1.0);
            [s * scale[0], s * scale[1], s * scale[2]]
        }
        Value::Array(_) => {
            let v = vec3(size, [1.0, 1.0, 1.0]);
            [v[0] * scale[0], v[1] * scale[1], v[2] * scale[2]]
        }
        _ => scale,
    }
}
