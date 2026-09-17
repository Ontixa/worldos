//! Newline-delimited JSON-RPC over stdio. Used by `worldos rpc` — the
//! raw endpoint SDKs/subprocesses speak — and shared parsing with MCP.

use crate::proto::{PARSE_ERROR, RpcRequest, RpcResponse};
use crate::service::RpcService;
use std::io::{BufRead, BufReader, Write};

/// Serve one request per line on `reader`, write responses to `writer`.
/// Requests without an `id` are notifications: executed, not answered.
pub fn serve<R: BufRead, W: Write>(
    service: &RpcService,
    reader: R,
    writer: &mut W,
) -> std::io::Result<()> {
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let parsed: Result<RpcRequest, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => {
                if req.id.is_none() {
                    let _ = service.handle(&req); // notification
                    continue;
                }
                service.handle(&req)
            }
            Err(e) => RpcResponse::err(None, PARSE_ERROR, e.to_string()),
        };
        let mut buf = serde_json::to_string(&resp).unwrap_or_default();
        buf.push('\n');
        writer.write_all(buf.as_bytes())?;
        writer.flush()?;
    }
    Ok(())
}

/// Convenience for `worldos rpc`: stdio in/out.
pub fn serve_stdio(service: &RpcService) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let reader = BufReader::new(stdin.lock());
    let mut stdout = std::io::stdout().lock();
    serve(service, reader, &mut stdout)
}
