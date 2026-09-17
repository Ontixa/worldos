//! In-process tests for the JSON-RPC service and MCP server — no sockets.

use serde_json::{Value, json};
use worldos_rpc::proto::RpcRequest;
use worldos_rpc::service::RpcService;

fn svc() -> RpcService {
    let mut e = worldos_engine::Engine::new("rpc-test");
    e.register_capability(std::sync::Arc::new(worldos_agent::AgentRun));
    RpcService::new(e)
}

fn call(svc: &RpcService, method: &str, params: Value) -> Value {
    let req = RpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: method.into(),
        params,
    };
    let resp = svc.handle(&req);
    serde_json::to_value(resp).unwrap()
}

#[test]
fn unknown_method_returns_method_not_found() {
    let resp = call(&svc(), "does.not.exist", json!({}));
    assert_eq!(resp["error"]["code"], -32601);
}

#[test]
fn malformed_params_return_error_not_panic() {
    let resp = call(&svc(), "object.get", json!({"bogus": true}));
    assert!(resp["error"].is_object());
    // string where object expected
    let resp = call(&svc(), "command.execute", json!("nope"));
    assert!(resp["error"].is_object());
}

#[test]
fn command_execute_then_object_get_roundtrip() {
    let svc = svc();
    let r = call(
        &svc,
        "command.execute",
        json!({"type": "object.create",
               "input": {"type": "core:note", "name": "rpc-note",
                         "components": {"doc:text": {"text": "hi"}}}}),
    );
    assert!(r["result"]["transaction_id"].is_string(), "{r}");
    let got = call(&svc, "object.get", json!({"name": "rpc-note"}));
    assert_eq!(got["result"]["name"], "rpc-note");
    assert!(got["result"]["components"]["doc:text"].is_object());
}

#[test]
fn undo_redo_via_rpc() {
    let svc = svc();
    call(
        &svc,
        "command.execute",
        json!({"type": "object.create", "input": {"type": "core:note", "name": "u"}}),
    );
    let u = call(&svc, "history.undo", json!({}));
    assert!(u["result"]["undone"].is_string());
    let r = call(&svc, "history.redo", json!({}));
    assert!(r["result"]["redone"].is_string());
}

#[test]
fn mcp_initialize_list_and_call() {
    let svc = svc();
    let session = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"object_create","arguments":{"type":"core:note","name":"mcp-note"}}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"project_search","arguments":{"text":"mcp"}}}"#,
        "\n",
    );
    let mut out = Vec::new();
    worldos_rpc::mcp::serve_mcp(&svc, session.as_bytes(), &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let resps: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(resps.len(), 4, "{text}");

    assert!(resps[0]["result"]["capabilities"]["tools"].is_object());
    let tools = resps[1]["result"]["tools"].as_array().unwrap();
    assert!(
        tools.len() >= 10,
        "expected tool catalog, got {}",
        tools.len()
    );
    // object_create succeeded (content is JSON text or structured result)
    let call_out = &resps[2]["result"];
    assert!(call_out.get("isError") != Some(&json!(true)), "{call_out}");
    // search found the created object
    let search = resps[3]["result"].to_string();
    assert!(search.contains("mcp-note"), "{search}");
}

#[test]
fn mcp_parse_error_line_returns_error_and_continues() {
    let svc = svc();
    let session = "garbage\n{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}\n";
    let mut out = Vec::new();
    worldos_rpc::mcp::serve_mcp(&svc, session.as_bytes(), &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let resps: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(resps[0]["error"]["code"], -32700);
    assert_eq!(resps[1]["id"], 9); // ping still answered
}
