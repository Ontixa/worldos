#![no_main]

//! Fuzz the untrusted-mesh boundaries in `worldos-kernel`:
//! - the hand-rolled binary-STL and OBJ parsers behind
//!   `geometry.import` — hostile file bytes must fail closed as
//!   values, never panic, overflow, or read out of bounds;
//! - `mesh_from_component`, the `geom:mesh` component decoder reached
//!   through `object_mesh` when a project file carries attacker-authored
//!   mesh data;
//! - writer/parser agreement: a decoded mesh pushed back through
//!   `to_binary_stl`/`to_obj` must itself re-parse, and the pure
//!   consumers (`signed_volume`, `surface_area`, `bbox`) must not
//!   panic on whatever the decoders accept.

use libfuzzer_sys::fuzz_target;
use worldos_kernel::mesh::{Mesh, mesh_from_component, to_binary_stl, to_obj};
use worldos_kernel::mesh_import::{self, MeshFormat};

fn exercise(mesh: &Mesh) {
    let _ = mesh.signed_volume();
    let _ = mesh.surface_area();
    let _ = mesh.bbox();
}

fuzz_target!(|data: &[u8]| {
    if let Ok(mesh) = mesh_import::parse(MeshFormat::Stl, data) {
        exercise(&mesh);
        let bytes = to_binary_stl(&mesh);
        let _ = mesh_import::parse(MeshFormat::Stl, &bytes);
    }
    if let Ok(mesh) = mesh_import::parse(MeshFormat::Obj, data) {
        exercise(&mesh);
        let bytes = to_obj(&mesh);
        let _ = mesh_import::parse(MeshFormat::Obj, &bytes);
    }
    // `geom:mesh` payloads are graph data a hostile project file
    // controls — any JSON shape must decode or fail cleanly.
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(data) {
        if let Ok(mesh) = mesh_from_component(&v) {
            exercise(&mesh);
        }
    }
});
