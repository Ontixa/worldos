#![no_main]

//! Fuzz the JSON-RPC dispatch surface: arbitrary bytes decode to an
//! `RpcRequest`, then run through `RpcService::handle` on a fresh
//! in-memory engine. File-IO and process-spawning methods are skipped —
//! on-disk behavior is covered by `store_migration`; this target stays
//! pure/deterministic so it can run anywhere.

use libfuzzer_sys::fuzz_target;
use worldos_engine::Engine;
use worldos_rpc::{RpcRequest, RpcService};

/// Methods with host-side effects outside the in-memory graph.
const SKIP_METHODS: &[&str] = &[
    "project.create", // creates a file when params.path is set
    "project.open",
    "project.save",
    "project.diff",
    "agent.run", // planner loop; deterministic and covered by property tests
];

/// Commands/capabilities with filesystem or subprocess effects.
const SKIP_COMMANDS: &[&str] = &[
    "geometry.export",
    "geometry.import",
    "cad.export_step",
    "cad.export_stl",
    "cad.import_step",
];
const SKIP_CAPABILITIES: &[&str] = &[
    "artifact.export",
    "geometry.export",
    "geometry.import",
    "plugin.run",
];

fuzz_target!(|data: &[u8]| {
    let Ok(req) = serde_json::from_slice::<RpcRequest>(data) else {
        return;
    };
    if SKIP_METHODS.contains(&req.method.as_str()) {
        return;
    }
    match req.method.as_str() {
        "command.execute"
            if SKIP_COMMANDS.contains(
                &req.params
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or(""),
            ) =>
        {
            return;
        }
        "capability.execute"
            if SKIP_CAPABILITIES.contains(
                &req.params
                    .get("id")
                    .and_then(|t| t.as_str())
                    .unwrap_or(""),
            ) =>
        {
            return;
        }
        _ => {}
    }
    let service = RpcService::new(Engine::new("fuzz"));
    let _ = service.handle(&req);
});
