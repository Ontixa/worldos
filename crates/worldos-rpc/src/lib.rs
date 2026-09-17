//! # worldos-rpc
//!
//! One JSON-RPC method surface (`RpcService`) exposed over stdio (SDKs,
//! subprocesses), WebSocket (`worldos serve`) and MCP (`worldos mcp`).

pub mod mcp;
pub mod proto;
pub mod service;
pub mod stdio;
pub mod ws;

pub use proto::{RpcRequest, RpcResponse};
pub use service::RpcService;
