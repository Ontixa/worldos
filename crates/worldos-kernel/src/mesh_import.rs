//! Parsing external mesh files into kernel [`Mesh`] values — the read
//! half of [`crate::mesh`]'s writers, backing `geometry.import`.
//!
//! Two formats, parsed by hand with no external dependencies:
//!
//! - **Binary STL** — 80-byte header + u32 facet count + 50 bytes per
//!   facet. The file is a triangle soup; vertices are welded by exact
//!   coordinate equality into an indexed mesh (first-occurrence order,
//!   so the result is deterministic). Facet normals are ignored —
//!   winding is preserved as authored and consumers recompute normals.
//!   Files whose byte length exceeds `84 + count*50` are tolerated
//!   (trailing padding exists in the wild); shorter files are rejected
//!   as truncated. ASCII STL is detected and refused explicitly.
//! - **Wavefront OBJ** — `v` positions + `f` faces. Faces may carry
//!   `v/vt/vn` slash indices (only the vertex index is used) and
//!   negative indices relative to the current vertex count; polygons
//!   fan-triangulate deterministically. `vn`/`vt`/`o`/`g`/`s`/
//!   `usemtl`/`mtllib` and unknown records are ignored — positions and
//!   faces are the whole contract here.
//!
//! Decoded meshes are stored inline in the `geom:mesh` component
//! (project-file JSON), so imports are bounded: at most
//! [`MAX_IMPORT_TRIANGLES`] triangles, and non-finite coordinates,
//! out-of-range indices, and empty meshes all fail closed.

use crate::error::KernelError;
use crate::mesh::Mesh;
use std::collections::HashMap;
use std::path::Path;

/// Hard cap on decoded triangles, bounding the inline `geom:mesh`
/// component to roughly a few MB of JSON in the project file.
pub const MAX_IMPORT_TRIANGLES: usize = 100_000;

fn bad(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidInput(msg.into())
}

/// Mesh file formats `geometry.import` understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshFormat {
    /// Binary STL (the only STL dialect — ASCII STL is refused).
    Stl,
    /// Wavefront OBJ (positions + faces; normals/UVs ignored).
    Obj,
}

impl MeshFormat {
    /// Explicit `format` input wins; otherwise infer from the file
    /// extension (case-insensitive). Unknown formats/extensions fail
    /// closed — guessing a parser from content is not worth it.
    pub fn detect(format: Option<&str>, path: &str) -> Result<Self, KernelError> {
        if let Some(f) = format {
            return match f {
                "stl" => Ok(Self::Stl),
                "obj" => Ok(Self::Obj),
                other => Err(bad(format!(
                    "unknown mesh format `{other}` (supported: stl, obj)"
                ))),
            };
        }
        match Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("stl") => Ok(Self::Stl),
            Some("obj") => Ok(Self::Obj),
            other => Err(bad(format!(
                "cannot infer mesh format from extension `{}` — pass `format` (stl|obj)",
                other.unwrap_or("<none>")
            ))),
        }
    }
}

/// Decode `bytes` per `format`, enforcing the shared limits (non-empty,
/// at most [`MAX_IMPORT_TRIANGLES`] triangles).
pub fn parse(format: MeshFormat, bytes: &[u8]) -> Result<Mesh, KernelError> {
    let mesh = match format {
        MeshFormat::Stl => from_binary_stl(bytes),
        MeshFormat::Obj => from_obj(bytes),
    }?;
    if mesh.triangle_count() == 0 {
        return Err(bad("mesh has no triangles"));
    }
    if mesh.triangle_count() > MAX_IMPORT_TRIANGLES {
        return Err(bad(format!(
            "mesh has {} triangles — import is capped at {MAX_IMPORT_TRIANGLES}",
            mesh.triangle_count()
        )));
    }
    Ok(mesh)
}

/// Parse a binary STL buffer into a vertex-welded indexed [`Mesh`].
pub fn from_binary_stl(bytes: &[u8]) -> Result<Mesh, KernelError> {
    const HEADER_LEN: usize = 80;
    const FACET_LEN: usize = 50;
    if bytes.len() < HEADER_LEN + 4 {
        return Err(if looks_like_ascii_stl(bytes) {
            bad("ASCII STL is not supported — use binary STL or OBJ")
        } else {
            bad("file is too small to be a binary STL")
        });
    }
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    let needed = HEADER_LEN + 4 + count * FACET_LEN;
    if bytes.len() == needed {
        // Exact layout match — binary even when the header says "solid".
        return facets(bytes, count);
    }
    if looks_like_ascii_stl(bytes) {
        return Err(bad("ASCII STL is not supported — use binary STL or OBJ"));
    }
    if count > MAX_IMPORT_TRIANGLES {
        return Err(bad(format!(
            "STL declares {count} facets — import is capped at {MAX_IMPORT_TRIANGLES}"
        )));
    }
    if bytes.len() > needed {
        return facets(bytes, count); // trailing bytes tolerated
    }
    Err(bad(format!(
        "truncated binary STL: {count} facets need {needed} bytes, file has {}",
        bytes.len()
    )))
}

/// An ASCII STL starts with `solid` and contains `facet` records. That
/// heuristic misfires only on a binary file whose payload happens to
/// contain the bytes "facet" — and only when its declared length is
/// already inconsistent, which is itself malformed.
fn looks_like_ascii_stl(bytes: &[u8]) -> bool {
    bytes.starts_with(b"solid") && bytes.windows(5).any(|w| w == b"facet")
}

/// Read `count` 50-byte facets starting at offset 84, welding identical
/// vertex coordinates (keyed on the raw f32 bit patterns, so welding is
/// exact and order-deterministic by first occurrence).
fn facets(bytes: &[u8], count: usize) -> Result<Mesh, KernelError> {
    let mut mesh = Mesh::default();
    let mut welded: HashMap<[u32; 3], u32> = HashMap::new();
    for i in 0..count {
        let base = 84 + i * 50;
        // facet layout: normal (12B) then three vertices (12B each)
        for v in 0..3 {
            let off = base + 12 + v * 12;
            let f = |k: usize| {
                f32::from_le_bytes(bytes[off + 4 * k..off + 4 * k + 4].try_into().unwrap())
            };
            let p = [f(0), f(1), f(2)];
            if !p.iter().all(|x| x.is_finite()) {
                return Err(bad(format!("STL facet {i} has a non-finite vertex")));
            }
            let key = [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
            let idx = *welded.entry(key).or_insert_with(|| {
                mesh.positions.push([p[0] as f64, p[1] as f64, p[2] as f64]);
                (mesh.positions.len() - 1) as u32
            });
            mesh.indices.push(idx);
        }
    }
    Ok(mesh)
}

/// Parse a Wavefront OBJ text file into an indexed [`Mesh`]: the `v`
/// list verbatim (f64), `f` faces fan-triangulated into `indices`.
pub fn from_obj(bytes: &[u8]) -> Result<Mesh, KernelError> {
    let text = std::str::from_utf8(bytes).map_err(|_| bad("OBJ file is not valid UTF-8 text"))?;
    let mut mesh = Mesh::default();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut tok = line.split_whitespace();
        let fail = |m: String| bad(format!("OBJ line {}: {m}", n + 1));
        match tok.next() {
            Some("v") => {
                let mut p = [0.0; 3];
                for v in p.iter_mut() {
                    *v = tok
                        .next()
                        .and_then(|s| s.parse::<f64>().ok())
                        .filter(|f| f.is_finite())
                        .ok_or_else(|| fail("v needs three finite numbers".into()))?;
                }
                mesh.positions.push(p);
            }
            Some("f") => {
                let mut face = Vec::with_capacity(4);
                for t in tok {
                    // `v/vt/vn` — only the vertex index matters here
                    let head = t.split('/').next().unwrap_or("");
                    let idx: i64 = head
                        .parse()
                        .map_err(|_| fail(format!("bad face index `{t}`")))?;
                    let resolved = if idx > 0 {
                        idx - 1
                    } else if idx < 0 {
                        mesh.positions.len() as i64 + idx
                    } else {
                        return Err(fail("face index 0 is invalid (1-based)".into()));
                    };
                    if resolved < 0 || resolved >= mesh.positions.len() as i64 {
                        return Err(fail(format!(
                            "face index {idx} out of range ({} vertices)",
                            mesh.positions.len()
                        )));
                    }
                    face.push(resolved as u32);
                }
                if face.len() < 3 {
                    return Err(fail("face needs at least 3 vertices".into()));
                }
                // fan triangulation — deterministic for convex polygons
                for k in 1..face.len() - 1 {
                    mesh.indices
                        .extend_from_slice(&[face[0], face[k], face[k + 1]]);
                }
                if mesh.triangle_count() > MAX_IMPORT_TRIANGLES {
                    return Err(bad(format!(
                        "OBJ exceeds the {MAX_IMPORT_TRIANGLES}-triangle import cap"
                    )));
                }
            }
            // vn/vt/o/g/s/usemtl/mtllib/l/p/… — ignored records
            _ => {}
        }
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{tessellate, to_binary_stl, to_obj};

    #[test]
    fn stl_roundtrip_preserves_facet_geometry() {
        // a curved primitive exercises normals recomputed from
        // f32-quantized vertices — still byte-identical
        let orig = tessellate("cylinder", [2.0, 2.0, 5.0]).unwrap();
        let bytes = to_binary_stl(&orig);
        let parsed = parse(MeshFormat::Stl, &bytes).unwrap();
        assert_eq!(parsed.triangle_count(), orig.triangle_count());
        assert_eq!(parsed.positions.len(), orig.positions.len());
        // welding may reorder vertices, but facet coordinates + winding
        // are identical → byte-identical re-export
        assert_eq!(to_binary_stl(&parsed), bytes);
        assert!((parsed.signed_volume() / (std::f64::consts::PI * 5.0) - 1.0).abs() < 0.02);
    }

    #[test]
    fn obj_roundtrip_is_exact() {
        let orig = tessellate("cylinder", [2.0, 2.0, 5.0]).unwrap();
        let bytes = to_obj(&orig);
        let parsed = parse(MeshFormat::Obj, &bytes).unwrap();
        assert_eq!(parsed, orig, "f64 shortest-roundtrip text is exact");
        assert_eq!(to_obj(&parsed), bytes);
    }

    #[test]
    fn obj_slashes_negatives_and_polygons() {
        let text = b"
# comment
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
vn 0 0 1
f 1/1/1 2/1/1 3/1/1 4/1/1
f -4 -3 -2
";
        let m = parse(MeshFormat::Obj, text).unwrap();
        assert_eq!(m.positions.len(), 4);
        assert_eq!(m.triangle_count(), 3); // quad fans to 2 + one tri
        // negative face: verts (0,0,0)(1,0,0)(1,1,0)
        assert_eq!(&m.indices[6..9], &[0, 1, 2]);
    }

    #[test]
    fn stl_failures_fail_closed() {
        // too small
        assert!(from_binary_stl(b"solid").is_err());
        // ascii
        assert!(
            parse(
                MeshFormat::Stl,
                b"solid x\n facet normal 0 0 1\n endsolid x\n"
            )
            .unwrap_err()
            .to_string()
            .contains("ASCII")
        );
        // truncated: declares 2 facets, carries 1
        let mut bytes = to_binary_stl(&tessellate("cube", [1.0; 3]).unwrap());
        bytes[80..84].copy_from_slice(&2u32.to_le_bytes());
        bytes.truncate(84 + 50);
        assert!(
            parse(MeshFormat::Stl, &bytes)
                .unwrap_err()
                .to_string()
                .contains("truncated")
        );
        // trailing bytes tolerated
        let mut padded = to_binary_stl(&tessellate("cube", [1.0; 3]).unwrap());
        padded.extend_from_slice(&[0u8; 7]);
        assert!(parse(MeshFormat::Stl, &padded).is_ok());
    }

    #[test]
    fn obj_failures_fail_closed() {
        for bad_obj in [
            &b"v 0 0 0\nf 1 2 3"[..],              // index out of range
            b"v 0 0\nf 1 2 3",                     // short vertex
            b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2",   // 2-vertex face
            b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2", // index 0
            b"v a 0 0\nf 1 2 3",                   // non-numeric vertex
        ] {
            assert!(parse(MeshFormat::Obj, bad_obj).is_err(), "{bad_obj:?}");
        }
        assert!(
            parse(MeshFormat::Obj, b"# empty\nv 1 2 3\n")
                .unwrap_err()
                .to_string()
                .contains("no triangles")
        );
    }

    #[test]
    fn format_detection() {
        assert_eq!(
            MeshFormat::detect(None, "a/b/c.STL").unwrap(),
            MeshFormat::Stl
        );
        assert_eq!(MeshFormat::detect(None, "m.obj").unwrap(), MeshFormat::Obj);
        assert_eq!(
            MeshFormat::detect(Some("obj"), "m.stl").unwrap(),
            MeshFormat::Obj,
            "explicit format wins over extension"
        );
        assert!(MeshFormat::detect(None, "m.step").is_err());
        assert!(MeshFormat::detect(Some("step"), "m.step").is_err());
    }
}
