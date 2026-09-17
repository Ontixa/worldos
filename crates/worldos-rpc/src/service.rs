//! `RpcService`: the single method-dispatch surface every transport and
//! every SDK talks to. Maps JSON-RPC methods onto `Engine` operations —
//! no separate business logic lives here.

use crate::proto::{RpcRequest, RpcResponse, INVALID_PARAMS, METHOD_NOT_FOUND};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;
use worldos_engine::{diff_projects, Engine};

pub struct RpcService {
    engine: Mutex<Engine>,
}

impl RpcService {
    pub fn new(mut engine: Engine) -> Self {
        // interfaces share one engine; agent.run is registered here so RPC
        // clients (MCP, SDK, CLI) all reach the same agent capability
        engine.register_capability(std::sync::Arc::new(worldos_agent::AgentRun));
        Self { engine: Mutex::new(engine) }
    }
    /// Open or create the project bound at startup.
    pub fn for_project(path: PathBuf) -> Result<Self, worldos_engine::EngineError> {
        Ok(Self::new(Engine::open(path)?))
    }

    pub fn handle(&self, req: &RpcRequest) -> RpcResponse {
        let id = req.id.clone();
        match self.dispatch(&req.method, &req.params) {
            Ok(v) => RpcResponse::ok(id, v),
            Err(DispatchError::UnknownMethod) => {
                RpcResponse::err(id, METHOD_NOT_FOUND, format!("unknown method {}", req.method))
            }
            Err(DispatchError::BadParams(m)) => RpcResponse::err(id, INVALID_PARAMS, m),
            Err(DispatchError::App(m)) => RpcResponse::app_err(id, m),
        }
    }

    fn dispatch(&self, method: &str, params: &Value) -> Result<Value, DispatchError> {
        let mut engine = self.engine.lock().map_err(|_| DispatchError::app("poisoned"))?;
        let p = |k: &str| params.get(k).cloned().unwrap_or(Value::Null);
        match method {
            // ----- project --------------------------------------------
            "project.info" => {
                let pr = engine.project();
                Ok(json!({
                    "id": pr.id.to_string(), "name": pr.name,
                    "schema_version": pr.schema_version,
                    "object_count": pr.objects.len(),
                    "relation_count": pr.relations.len(),
                    "path": engine.path().map(|p| p.display().to_string()),
                    "can_undo": engine.can_undo(), "can_redo": engine.can_redo(),
                    "dirty": engine.is_dirty(),
                }))
            }
            "project.create" => {
                let name = p("name").as_str().unwrap_or("untitled").to_string();
                let path = p("path").as_str().map(PathBuf::from);
                let mut e = match &path {
                    Some(path) => Engine::create(&name, path).map_err(DispatchError::app)?,
                    None => Engine::new(&name),
                };
                std::mem::swap(&mut *engine, &mut e);
                Ok(json!({"ok": true}))
            }
            "project.open" => {
                let path = p("path");
                let path = path.as_str().ok_or(DispatchError::bad("missing path"))?;
                let mut e = Engine::open(path).map_err(DispatchError::app)?;
                std::mem::swap(&mut *engine, &mut e);
                Ok(json!({"ok": true, "name": engine.project().name}))
            }
            "project.save" => {
                match p("path").as_str() {
                    Some(path) => engine.save_as(path).map_err(DispatchError::app)?,
                    None => engine.save().map_err(DispatchError::app)?,
                }
                Ok(json!({"ok": true}))
            }
            "project.snapshot" => {
                Ok(serde_json::to_value(engine.snapshot()).map_err(DispatchError::app)?)
            }
            "project.graph" => {
                let pr = engine.project();
                let nodes: Vec<Value> = pr
                    .sorted_objects()
                    .iter()
                    .map(|o| {
                        json!({"id": o.id.to_string(), "name": o.name, "type": o.type_id,
                               "components": o.components.keys().collect::<Vec<_>>()})
                    })
                    .collect();
                let edges: Vec<Value> = pr
                    .relations
                    .values()
                    .map(|r| json!({"id": r.id.to_string(), "type": r.type_id,
                                    "from": r.from.to_string(), "to": r.to.to_string()}))
                    .collect();
                Ok(json!({"nodes": nodes, "edges": edges}))
            }
            "project.search" => {
                let q: worldos_kernel::SearchQuery =
                    serde_json::from_value(params.clone()).unwrap_or_default();
                let out: Vec<Value> = engine
                    .search(&q)
                    .iter()
                    .map(|o| json!({"id": o.id.to_string(), "name": o.name,
                                    "type": o.type_id, "tags": o.tags}))
                    .collect();
                Ok(json!({"results": out}))
            }
            "project.diff" => {
                // diff current project vs another .worldos file
                let other = p("other");
                let other = other.as_str().ok_or(DispatchError::bad("missing other"))?;
                let b = Engine::open(other).map_err(DispatchError::app)?;
                let entries = diff_projects(b.project(), engine.project());
                Ok(serde_json::to_value(entries).map_err(DispatchError::app)?)
            }

            // ----- objects ---------------------------------------------
            "object.get" => {
                let oid = resolve(&engine, params).ok_or(DispatchError::bad("object not found"))?;
                let o = engine.get_object(oid).ok_or(DispatchError::bad("gone"))?;
                Ok(serde_json::to_value(o).map_err(DispatchError::app)?)
            }
            "object.list" => {
                let ty = p("type").as_str().map(String::from);
                let out: Vec<Value> = engine
                    .project()
                    .sorted_objects()
                    .iter()
                    .filter(|o| ty.as_ref().map(|t| o.type_id.0 == *t).unwrap_or(true))
                    .map(|o| json!({"id": o.id.to_string(), "name": o.name, "type": o.type_id}))
                    .collect();
                Ok(json!({"objects": out}))
            }

            // ----- commands / capabilities ------------------------------
            "command.list" => Ok(serde_json::to_value(engine.command_schemas())
                .map_err(DispatchError::app)?),
            "command.execute" => {
                let tyv = p("type");
                let ty = tyv.as_str().ok_or(DispatchError::bad("missing type"))?;
                let input = p("input");
                let receipt = engine.execute(ty, input).map_err(DispatchError::app)?;
                Ok(serde_json::to_value(receipt).map_err(DispatchError::app)?)
            }
            "capability.list" => Ok(serde_json::to_value(engine.capability_descriptors())
                .map_err(DispatchError::app)?),
            "capability.execute" => {
                let capv = p("id");
                let cap = capv.as_str().ok_or(DispatchError::bad("missing id"))?;
                let input = p("input");
                Ok(engine.run_capability(cap, input).map_err(DispatchError::app)?)
            }

            // ----- history -------------------------------------------------
            "history.list" => {
                let limit = p("limit").as_u64().unwrap_or(50) as usize;
                let recs: Vec<Value> = engine
                    .history()
                    .records
                    .iter()
                    .rev()
                    .take(limit)
                    .map(|r| {
                        json!({
                            "id": r.id.to_string(), "index": r.index, "actor": r.actor,
                            "label": r.label, "committed_at": r.committed_at,
                            "undone": r.undone,
                            "commands": r.commands.iter().map(|c| json!({
                                "type": c.envelope.command_type, "ok": c.ok,
                                "id": c.envelope.id.to_string(),
                            })).collect::<Vec<_>>(),
                            "ops": r.ops.iter().map(|o| o.describe()).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                Ok(json!({"transactions": recs, "cursor": engine.history().cursor}))
            }
            "history.undo" => Ok(json!({"undone": engine.undo().map_err(DispatchError::app)?
                .map(|t| t.to_string())})),
            "history.redo" => Ok(json!({"redone": engine.redo().map_err(DispatchError::app)?
                .map(|t| t.to_string())})),

            // ----- validation / agent / events -------------------------------
            "validation.run" => {
                Ok(serde_json::to_value(engine.validate()).map_err(DispatchError::app)?)
            }
            "agent.run" => {
                let out = engine
                    .run_capability("agent.run", params.clone())
                    .map_err(DispatchError::app)?;
                Ok(out)
            }
            "events.drain" => {
                let evs = engine.drain_events();
                Ok(serde_json::to_value(evs).map_err(DispatchError::app)?)
            }
            _ => Err(DispatchError::UnknownMethod),
        }
    }
}

fn resolve(engine: &Engine, params: &Value) -> Option<worldos_kernel::ObjectId> {
    if let Some(id) = params.get("id").and_then(|v| v.as_str()) {
        if let Ok(oid) = id.parse() {
            return Some(oid);
        }
    }
    params
        .get("name")
        .and_then(|v| v.as_str())
        .and_then(|n| engine.find_object(n))
        .map(|o| o.id)
}

enum DispatchError {
    UnknownMethod,
    BadParams(String),
    App(String),
}
impl DispatchError {
    fn bad(m: impl Into<String>) -> Self {
        Self::BadParams(m.into())
    }
    fn app(e: impl std::fmt::Display) -> Self {
        Self::App(e.to_string())
    }
}
