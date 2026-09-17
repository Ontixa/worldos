//! WorldOS desktop backend — Tauri commands driving the same `Engine`
//! used by the CLI, RPC, MCP and SDK surfaces. No parallel logic: every
//! mutation goes through commands, every read through the engine facade.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::State;
use worldos_engine::{diff_projects, Engine};

/// The open project. `None` until the user creates or opens a file.
struct AppState(Mutex<Option<Engine>>);

fn make_engine(name: &str, path: &PathBuf) -> Result<Engine, String> {
    let mut e = Engine::create(name, path).map_err(|e| e.to_string())?;
    e.register_capability(Arc::new(worldos_agent::AgentRun));
    e.save().map_err(|e| e.to_string())?;
    Ok(e)
}

fn load_engine(path: &PathBuf) -> Result<Engine, String> {
    let mut e = Engine::open(path).map_err(|e| e.to_string())?;
    e.register_capability(Arc::new(worldos_agent::AgentRun));
    Ok(e)
}

fn info_json(e: &Engine) -> Value {
    let p = e.project();
    json!({
        "id": p.id.to_string(),
        "name": p.name,
        "path": e.path().map(|p| p.display().to_string()),
        "object_count": p.objects.len(),
        "relation_count": p.relations.len(),
        "dirty": e.is_dirty(),
        "can_undo": e.can_undo(),
        "can_redo": e.can_redo(),
        "actor": e.actor().id.to_string(),
    })
}

fn object_json(o: &worldos_kernel::model::Object) -> Value {
    serde_json::to_value(o).unwrap_or(json!({"id": o.id.to_string()}))
}

// ----- project lifecycle ---------------------------------------------

#[tauri::command]
fn project_new(state: State<AppState>, name: String, path: String) -> Result<Value, String> {
    let e = make_engine(&name, &PathBuf::from(&path))?;
    let info = info_json(&e);
    *state.0.lock().map_err(|e| e.to_string())? = Some(e);
    Ok(info)
}

#[tauri::command]
fn project_open(state: State<AppState>, path: String) -> Result<Value, String> {
    let e = load_engine(&PathBuf::from(&path))?;
    let info = info_json(&e);
    *state.0.lock().map_err(|e| e.to_string())? = Some(e);
    Ok(info)
}

#[tauri::command]
fn project_save(state: State<AppState>, path: Option<String>) -> Result<Value, String> {
    let mut g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_mut().ok_or("no project open")?;
    match path {
        Some(p) if !p.is_empty() => e.save_as(PathBuf::from(p)),
        _ => e.save(),
    }
    .map_err(|e| e.to_string())?;
    Ok(info_json(e))
}

#[tauri::command]
fn project_info(state: State<AppState>) -> Result<Value, String> {
    let g = state.0.lock().map_err(|e| e.to_string())?;
    match g.as_ref() {
        Some(e) => Ok(info_json(e)),
        None => Ok(Value::Null),
    }
}

// ----- reads ----------------------------------------------------------

#[tauri::command]
fn object_list(state: State<AppState>, type_id: Option<String>) -> Result<Value, String> {
    let g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_ref().ok_or("no project open")?;
    let mut objs: Vec<Value> = e
        .project()
        .objects
        .values()
        .filter(|o| type_id.as_ref().map_or(true, |t| o.type_id.0 == *t))
        .map(object_json)
        .collect();
    objs.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(json!(objs))
}

#[tauri::command]
fn object_get(state: State<AppState>, id_or_name: String) -> Result<Value, String> {
    let g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_ref().ok_or("no project open")?;
    let obj = match id_or_name.parse::<worldos_kernel::ObjectId>() {
        Ok(id) => e.get_object(id),
        Err(_) => e.find_object(&id_or_name),
    };
    obj.map(object_json).ok_or_else(|| "object not found".into())
}

#[tauri::command]
fn object_relations(state: State<AppState>, id: String) -> Result<Value, String> {
    let g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_ref().ok_or("no project open")?;
    let id: worldos_kernel::ObjectId = id.parse().map_err(|_| "bad id")?;
    let rels: Vec<Value> = e
        .object_relations(id)
        .iter()
        .map(|r| {
            json!({"id": r.id.to_string(), "type": r.type_id,
                   "from": r.from.to_string(), "to": r.to.to_string()})
        })
        .collect();
    Ok(json!(rels))
}

#[tauri::command]
fn graph(state: State<AppState>) -> Result<Value, String> {
    let g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_ref().ok_or("no project open")?;
    let p = e.project();
    let nodes: Vec<Value> = p
        .objects
        .values()
        .map(|o| {
            json!({"id": o.id.to_string(), "name": o.name, "type": o.type_id.0,
                   "components": o.components.keys().collect::<Vec<_>>()})
        })
        .collect();
    let edges: Vec<Value> = p
        .relations
        .values()
        .map(|r| json!({"id": r.id.to_string(), "type": r.type_id,
                        "from": r.from.to_string(), "to": r.to.to_string()}))
        .collect();
    Ok(json!({"nodes": nodes, "edges": edges}))
}

#[tauri::command]
fn search(state: State<AppState>, query: Value) -> Result<Value, String> {
    let g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_ref().ok_or("no project open")?;
    let q: worldos_kernel::SearchQuery =
        serde_json::from_value(query).map_err(|e| e.to_string())?;
    let out: Vec<Value> = e
        .search(&q)
        .iter()
        .map(|o| json!({"id": o.id.to_string(), "name": o.name, "type": o.type_id.0}))
        .collect();
    Ok(json!(out))
}

// ----- governed mutations ----------------------------------------------

#[tauri::command]
fn command_execute(state: State<AppState>, command_type: String, input: Value) -> Result<Value, String> {
    let mut g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_mut().ok_or("no project open")?;
    let receipt = e
        .execute(&command_type, input)
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "command_id": receipt.command_id.to_string(),
        "transaction_id": receipt.transaction_id.to_string(),
        "output": receipt.output,
    }))
}

#[tauri::command]
fn command_list(state: State<AppState>) -> Result<Value, String> {
    let g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_ref().ok_or("no project open")?;
    Ok(serde_json::to_value(e.command_schemas()).map_err(|e| e.to_string())?)
}

#[tauri::command]
fn capability_list(state: State<AppState>) -> Result<Value, String> {
    let g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_ref().ok_or("no project open")?;
    Ok(serde_json::to_value(e.capability_descriptors()).map_err(|e| e.to_string())?)
}

#[tauri::command]
fn capability_execute(state: State<AppState>, id: String, input: Value) -> Result<Value, String> {
    let mut g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_mut().ok_or("no project open")?;
    e.run_capability(&id, input).map_err(|e| e.to_string())
}

// ----- history ---------------------------------------------------------

#[tauri::command]
fn history(state: State<AppState>, limit: Option<usize>) -> Result<Value, String> {
    let g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_ref().ok_or("no project open")?;
    let txns: Vec<Value> = e
        .history()
        .records
        .iter()
        .rev()
        .take(limit.unwrap_or(100))
        .map(|t| {
            json!({
                "id": t.id.to_string(), "index": t.index, "actor": t.actor.to_string(),
                "label": t.label, "committed_at": t.committed_at, "undone": t.undone,
                "commands": t.commands.iter().map(|c| json!({
                    "type": c.envelope.command_type, "ok": c.ok,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!(txns))
}

#[tauri::command]
fn undo(state: State<AppState>) -> Result<Value, String> {
    let mut g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_mut().ok_or("no project open")?;
    let id = e.undo().map_err(|e| e.to_string())?;
    Ok(json!({"undone": id.map(|i| i.to_string())}))
}

#[tauri::command]
fn redo(state: State<AppState>) -> Result<Value, String> {
    let mut g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_mut().ok_or("no project open")?;
    let id = e.redo().map_err(|e| e.to_string())?;
    Ok(json!({"redone": id.map(|i| i.to_string())}))
}

#[tauri::command]
fn validate(state: State<AppState>) -> Result<Value, String> {
    let g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_ref().ok_or("no project open")?;
    Ok(serde_json::to_value(e.validate()).map_err(|e| e.to_string())?)
}

#[tauri::command]
fn diff_with(state: State<AppState>, other: String) -> Result<Value, String> {
    let g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_ref().ok_or("no project open")?;
    let b = Engine::open(&other).map_err(|e| e.to_string())?;
    Ok(serde_json::to_value(diff_projects(e.project(), b.project()))
        .map_err(|e| e.to_string())?)
}

// ----- agent -----------------------------------------------------------

#[tauri::command]
fn agent_run(state: State<AppState>, goal: String) -> Result<Value, String> {
    let mut g = state.0.lock().map_err(|e| e.to_string())?;
    let e = g.as_mut().ok_or("no project open")?;
    e.run_capability("agent.run", json!({"goal": goal, "agent": "desktop-agent"}))
        .map_err(|e| e.to_string())
}

// ----- app -------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            project_new,
            project_open,
            project_save,
            project_info,
            object_list,
            object_get,
            object_relations,
            graph,
            search,
            command_execute,
            command_list,
            capability_list,
            capability_execute,
            history,
            undo,
            redo,
            validate,
            diff_with,
            agent_run,
        ])
        .run(tauri::generate_context!())
        .expect("error while running WorldOS desktop");
}
