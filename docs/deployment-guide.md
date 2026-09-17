# Deployment Guide

## Build

```bash
# Core workspace (CLI, engine, RPC, agent)
cargo build --release -p worldos-cli
# binary: target/release/worldos(.exe)

# Desktop app
cd apps/desktop
npm install
npm run build
npm run tauri build        # produces native bundle in src-tauri/target

# SDKs
npm run build -w @worldos/sdk
pip install -e sdks/worldos-py
```

## Runtime requirements

- **CLI/server**: no runtime deps (SQLite bundled). `.worldos` files are
  single-file SQLite — keep them on real filesystems (not 9p/network
  mounts where locking is unreliable).
- **Desktop**: WebView2 on Windows (preinstalled on Win11/macOS WKWebView
  built-in).
- **MCP**: add to an AI client's config, e.g.
  `{"command": "worldos", "args": ["mcp", "path/to/project.worldos"]}`.

## Running services

```bash
worldos serve project.worldos --port 7799   # WebSocket JSON-RPC
worldos rpc project.worldos                 # stdio NDJSON (for SDKs/CI)
worldos mcp project.worldos                 # MCP over stdio
```

`serve` binds `127.0.0.1` only — there is no transport authentication
yet; front it with your own proxy/tunnel if remote access is needed.

## CI

`.github/workflows/ci.yml`: fmt + clippy + workspace tests
(windows-latest), TS SDK build, Python SDK compile, desktop frontend
build + `cargo check` of src-tauri.

## Versioning & migrations

`PRAGMA user_version` inside each `.worldos` file tracks the storage
schema; `SqliteStore` migrates on open. Component schemas carry their
own `version` field for semantic evolution independent of storage.
