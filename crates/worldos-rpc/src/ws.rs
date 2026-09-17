//! WebSocket JSON-RPC server (`worldos serve`) — the transport used by
//! the TypeScript SDK and, optionally, a future web client.

use crate::proto::{RpcRequest, RpcResponse, PARSE_ERROR};
use crate::service::RpcService;
use std::net::TcpListener;
use std::sync::Arc;
use tungstenite::{accept, Message};

/// Blocking WS server; one thread per connection, all sharing the engine.
pub fn serve_ws(service: Arc<RpcService>, addr: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr)?;
    tracing::info!(%addr, "worldos serve listening");
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let svc = service.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle_conn(s, svc) {
                        tracing::debug!("ws conn closed: {e}");
                    }
                });
            }
            Err(e) => tracing::warn!("accept failed: {e}"),
        }
    }
    Ok(())
}

fn handle_conn(
    stream: std::net::TcpStream,
    service: Arc<RpcService>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut ws = accept(stream)?;
    loop {
        match ws.read()? {
            Message::Text(text) => {
                let resp = match serde_json::from_str::<RpcRequest>(&text) {
                    Ok(req) => {
                        if req.id.is_none() {
                            let _ = service.handle(&req);
                            continue;
                        }
                        service.handle(&req)
                    }
                    Err(e) => RpcResponse::err(None, PARSE_ERROR, e.to_string()),
                };
                ws.send(Message::Text(serde_json::to_string(&resp)?.into()))?;
            }
            Message::Close(_) => return Ok(()),
            Message::Ping(p) => ws.send(Message::Pong(p))?,
            _ => {}
        }
    }
}
