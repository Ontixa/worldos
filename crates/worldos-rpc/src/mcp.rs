//! MCP (Model Context Protocol) server over stdio — WorldOS as an MCP
//! server so external agents/tools can inspect and mutate projects
//! through the governed command layer.
//!
//! Implements the JSON-RPC 2.0 subset MCP needs: initialize, ping,
//! tools/list, tools/call, resources/list, resources/read.

use crate::proto::{RpcRequest, RpcResponse, PARSE_ERROR};
use crate::service::RpcService;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};

const PROTOCOL_VERSION: &str = "2025-06-18";

/// Tool name → (description, inputSchema, rpc method or custom mapping).
fn tools() -> Vec<Value> {
    let obj = |req: &[&str], props: Value| {
        json!({"type": "object", "required": req, "properties": props})
    };
    vec![
        json!({"name": "project_inspect", "description": "Project summary: counts, types, undo state",
               "inputSchema": obj(&[], json!({}))}),
        json!({"name": "project_search", "description": "Search objects by text/type/tag",
               "inputSchema": obj(&[], json!({"text": {"type":"string"}, "type_id": {"type":"string"}, "tag": {"type":"string"}}))}),
        json!({"name": "project_graph", "description": "Full graph: nodes + typed edges",
               "inputSchema": obj(&[], json!({}))}),
        json!({"name": "object_get", "description": "Get one object by id or name",
               "inputSchema": obj(&[], json!({"id": {"type":"string"}, "name": {"type":"string"}}))}),
        json!({"name": "object_create", "description": "Create an object (type, name, optional components/tags/parent)",
               "inputSchema": obj(&["type","name"], json!({
                   "type": {"type":"string"}, "name": {"type":"string"},
                   "components": {"type":"object"}, "tags": {"type":"array"}, "parent": {"type":"string"}}))}),
        json!({"name": "object_set_property", "description": "Set a component property path on an object",
               "inputSchema": obj(&["component","path","value"], json!({
                   "id": {"type":"string"}, "name": {"type":"string"},
                   "component": {"type":"string"}, "path": {"type":"string"}, "value": {}}))}),
        json!({"name": "object_rename", "description": "Rename an object",
               "inputSchema": obj(&["name"], json!({"object": {"type":"string"}, "name": {"type":"string"}}))}),
        json!({"name": "object_delete", "description": "Delete an object (cascade optional)",
               "inputSchema": obj(&[], json!({"id": {"type":"string"}, "name": {"type":"string"}, "cascade": {"type":"boolean"}}))}),
        json!({"name": "command_list", "description": "List all command schemas",
               "inputSchema": obj(&[], json!({}))}),
        json!({"name": "command_execute", "description": "Execute any registered command",
               "inputSchema": obj(&["type"], json!({"type": {"type":"string"}, "input": {"type":"object"}}))}),
        json!({"name": "capability_list", "description": "List capability descriptors",
               "inputSchema": obj(&[], json!({}))}),
        json!({"name": "capability_execute", "description": "Execute a capability",
               "inputSchema": obj(&["id"], json!({"id": {"type":"string"}, "input": {"type":"object"}}))}),
        json!({"name": "history_list", "description": "List committed transactions",
               "inputSchema": obj(&[], json!({"limit": {"type":"integer"}}))}),
        json!({"name": "history_undo", "description": "Undo the latest transaction",
               "inputSchema": obj(&[], json!({}))}),
        json!({"name": "history_redo", "description": "Redo the latest undone transaction",
               "inputSchema": obj(&[], json!({}))}),
        json!({"name": "validation_run", "description": "Run all validators",
               "inputSchema": obj(&[], json!({}))}),
        json!({"name": "agent_run", "description": "Run the builtin agent on a goal",
               "inputSchema": obj(&["goal"], json!({"goal": {"type":"string"}, "agent": {"type":"string"}}))}),
        json!({"name": "project_save", "description": "Persist the project",
               "inputSchema": obj(&[], json!({}))}),
    ]
}

fn tool_call(service: &RpcService, name: &str, args: &Value) -> Result<Value, String> {
    let (method, params) = match name {
        "project_inspect" => ("project.info", json!({})),
        "project_search" => ("project.search", args.clone()),
        "project_graph" => ("project.graph", json!({})),
        "object_get" => ("object.get", args.clone()),
        "object_create" => ("command.execute", json!({"type": "object.create", "input": args})),
        "object_set_property" => {
            ("command.execute", json!({"type": "object.set_property", "input": args}))
        }
        "object_rename" => ("command.execute", json!({"type": "object.rename", "input": args})),
        "object_delete" => ("command.execute", json!({"type": "object.delete", "input": args})),
        "command_list" => ("command.list", json!({})),
        "command_execute" => ("command.execute", args.clone()),
        "capability_list" => ("capability.list", json!({})),
        "capability_execute" => ("capability.execute", args.clone()),
        "history_list" => ("history.list", args.clone()),
        "history_undo" => ("history.undo", json!({})),
        "history_redo" => ("history.redo", json!({})),
        "validation_run" => ("validation.run", json!({})),
        "agent_run" => ("agent.run", args.clone()),
        "project_save" => ("project.save", json!({})),
        _ => return Err(format!("unknown tool `{name}`")),
    };
    let req = RpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: method.into(),
        params,
    };
    let resp = service.handle(&req);
    if let Some(err) = resp.error {
        return Err(err.message);
    }
    Ok(resp.result.unwrap_or(Value::Null))
}

/// Serve MCP over the given reader/writer until EOF.
pub fn serve_mcp<R: BufRead, W: Write>(
    service: &RpcService,
    reader: R,
    writer: &mut W,
) -> std::io::Result<()> {
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req: RpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                write_resp(writer, &RpcResponse::err(None, PARSE_ERROR, e.to_string()))?;
                continue;
            }
        };
        let is_notification = req.id.is_none();
        let resp = match req.method.as_str() {
            "initialize" => RpcResponse::ok(
                req.id.clone(),
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {"tools": {"listChanged": false}, "resources": {}},
                    "serverInfo": {"name": "worldos", "version": env!("CARGO_PKG_VERSION")},
                }),
            ),
            "notifications/initialized" | "initialized" => continue,
            "ping" => RpcResponse::ok(req.id.clone(), json!({})),
            "tools/list" => RpcResponse::ok(req.id.clone(), json!({"tools": tools()})),
            "tools/call" => {
                let name = req.params["name"].as_str().unwrap_or("");
                let args = req.params.get("arguments").cloned().unwrap_or(json!({}));
                match tool_call(service, name, &args) {
                    Ok(v) => RpcResponse::ok(
                        req.id.clone(),
                        json!({"content": [{"type": "text",
                               "text": serde_json::to_string_pretty(&v).unwrap_or_default()}],
                               "isError": false}),
                    ),
                    Err(m) => RpcResponse::ok(
                        req.id.clone(),
                        json!({"content": [{"type": "text", "text": m}], "isError": true}),
                    ),
                }
            }
            "resources/list" => RpcResponse::ok(
                req.id.clone(),
                json!({"resources": [
                    {"uri": "worldos://project", "name": "Project graph", "mimeType": "application/json"},
                    {"uri": "worldos://history", "name": "Transaction history", "mimeType": "application/json"},
                ]}),
            ),
            "resources/read" => {
                let uri = req.params["uri"].as_str().unwrap_or("");
                let (method, ok) = match uri {
                    "worldos://project" => ("project.graph", true),
                    "worldos://history" => ("history.list", true),
                    _ => ("", false),
                };
                if !ok {
                    RpcResponse::err(req.id.clone(), -32602, format!("unknown resource {uri}"))
                } else {
                    let inner = RpcRequest {
                        jsonrpc: "2.0".into(), id: Some(json!(1)),
                        method: method.into(), params: json!({}),
                    };
                    let r = service.handle(&inner);
                    let text = serde_json::to_string_pretty(&r.result.unwrap_or(Value::Null))
                        .unwrap_or_default();
                    RpcResponse::ok(req.id.clone(), json!({"contents": [
                        {"uri": uri, "mimeType": "application/json", "text": text}
                    ]}))
                }
            }
            _ => {
                if is_notification {
                    continue;
                }
                RpcResponse::err(req.id.clone(), -32601, format!("unknown method {}", req.method))
            }
        };
        if !is_notification {
            write_resp(writer, &resp)?;
        }
    }
    Ok(())
}

fn write_resp<W: Write>(w: &mut W, resp: &RpcResponse) -> std::io::Result<()> {
    let mut s = serde_json::to_string(resp).unwrap_or_default();
    s.push('\n');
    w.write_all(s.as_bytes())?;
    w.flush()
}

/// `worldos mcp` entry: stdio MCP server bound to the open project.
pub fn serve_stdio(service: &RpcService) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let reader = BufReader::new(stdin.lock());
    let mut stdout = std::io::stdout().lock();
    serve_mcp(service, reader, &mut stdout)
}
