//! Hosted plugin runtime.
//!
//! A plugin is any executable named `worldos-plugin-*`. The host spawns it
//! and serves a line-delimited JSON-RPC channel over the plugin's own
//! stdin/stdout — the plugin is the *client*: it writes requests on
//! stdout, reads responses on stdin. Everything the plugin does flows
//! through commands as the `plugin:<name>` actor, inside ONE transaction:
//! a clean exit commits, any failure rolls back.
//!
//! Protocol (v1): one JSON object per line.
//!   → `{"id": 1, "method": "command.execute", "params": {...}}`
//!   ← `{"id": 1, "result": ...}` or `{"id": 1, "error": {"message": ...}}`
//! Plugin signals completion with `session.end` or by closing stdout.
//! Diagnostics go to the plugin's stderr — never stdout.
//!
//! Env handed to the child: `WORLDOS_PROJECT`, `WORLDOS_PLUGIN_NAME`,
//! `WORLDOS_PROTOCOL=1`.

use crate::error::CapabilityError;
use crate::host::CapabilityHost;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use worldos_kernel::actor::Actor;

pub const PROTOCOL_VERSION: u32 = 1;
const DEFAULT_TIMEOUT_MS: u64 = 120_000;

/// A discovered or explicitly-given plugin invocation.
#[derive(Debug, Clone)]
pub struct PluginSpec {
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PluginOutcome {
    pub plugin: String,
    pub requests: usize,
    pub committed: bool,
    pub exit_code: Option<i32>,
}

fn fail(e: impl std::fmt::Display) -> CapabilityError {
    CapabilityError::Failed(e.to_string())
}

// ------------------------------------------------------------------ spawn

/// Spawn the plugin and serve its request channel until it exits or the
/// timeout kills it. Commits on clean exit, rolls back otherwise.
pub fn run_plugin(
    host: &mut dyn CapabilityHost,
    spec: &PluginSpec,
) -> Result<PluginOutcome, CapabilityError> {
    let mut cmd = spawn_command(spec);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .env("WORLDOS_PLUGIN_NAME", &spec.name)
        .env("WORLDOS_PROTOCOL", PROTOCOL_VERSION.to_string());
    if let Some(p) = host.project_path() {
        cmd.env("WORLDOS_PROJECT", p);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| fail(format!("spawn `{}`: {e}", spec.program.display())))?;
    let stdout = child.stdout.take().unwrap();
    let stdin = child.stdin.take().unwrap();

    // Pump child stdout on a thread so we can enforce a timeout.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line).is_err() {
                return;
            }
        }
    });

    let actor = Actor::plugin(&spec.name);
    let own_txn = !host.in_transaction();
    if own_txn {
        host.begin_transaction_as(&actor, &format!("plugin:{} session", spec.name))?;
    }
    let deadline = Instant::now() + Duration::from_millis(spec.timeout_ms);
    let mut requests = 0usize;
    let outcome = pump(
        host,
        &actor,
        &rx,
        stdin,
        &mut child,
        deadline,
        &mut requests,
    );

    let exit_code = match child.try_wait() {
        Ok(Some(s)) => s.code(),
        _ => {
            let _ = child.wait();
            child.try_wait().ok().flatten().and_then(|s| s.code())
        }
    };
    let ok = outcome.is_ok() && exit_code.unwrap_or(0) == 0;
    if own_txn {
        if ok {
            host.commit_transaction()?;
        } else {
            let _ = host.rollback_transaction();
        }
    }
    outcome?;
    Ok(PluginOutcome {
        plugin: spec.name.clone(),
        requests,
        committed: ok,
        exit_code,
    })
}

/// Blocking session over any reader/writer pair — the reference loop used
/// by in-process tests and by embedders hosting a plugin in-thread (no
/// subprocess). Runs until the plugin sends `session.end` or closes the
/// stream. The caller owns transaction control.
pub fn serve_stream<R: BufRead, W: Write>(
    host: &mut dyn CapabilityHost,
    actor: &Actor,
    reader: R,
    mut writer: W,
) -> Result<usize, CapabilityError> {
    let mut requests = 0usize;
    let mut ended = false;
    for line in reader.lines() {
        let line = line.map_err(fail)?;
        requests += 1;
        if let Some(resp) = handle_line(host, actor, &line, &mut ended) {
            writeln!(writer, "{resp}").map_err(fail)?;
            writer.flush().map_err(fail)?;
        }
        if ended {
            break;
        }
    }
    Ok(requests)
}

/// Request pump: read request lines, dispatch, write responses.
fn pump(
    host: &mut dyn CapabilityHost,
    actor: &Actor,
    rx: &mpsc::Receiver<std::io::Result<String>>,
    mut stdin: impl Write,
    child: &mut Child,
    deadline: Instant,
    requests: &mut usize,
) -> Result<(), CapabilityError> {
    let mut ended = false;
    loop {
        if ended {
            return Ok(());
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err(fail("plugin timed out"));
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(line)) => {
                *requests += 1;
                if let Some(resp) = handle_line(host, actor, &line, &mut ended) {
                    writeln!(stdin, "{resp}").map_err(fail)?;
                    stdin.flush().map_err(fail)?;
                }
            }
            Ok(Err(e)) => return Err(fail(format!("read plugin: {e}"))),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if child.try_wait().ok().flatten().is_some() {
                    // exited; drain whatever is left then finish
                    while let Ok(Ok(line)) = rx.try_recv() {
                        *requests += 1;
                        if let Some(resp) = handle_line(host, actor, &line, &mut ended) {
                            let _ = writeln!(stdin, "{resp}");
                            let _ = stdin.flush();
                        }
                    }
                    return Ok(());
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

/// Parse + dispatch one request line. `ended` flips on `session.end`.
fn handle_line(
    host: &mut dyn CapabilityHost,
    actor: &Actor,
    line: &str,
    ended: &mut bool,
) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return Some(
                json!({"id": null, "error": {"message": format!("bad json: {e}")}}).to_string(),
            );
        }
    };
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(json!({}));
    if method == "session.end" {
        *ended = true;
    }
    match dispatch(host, actor, method, &params) {
        Ok(v) => Some(json!({"id": id, "result": v}).to_string()),
        Err(e) => Some(json!({"id": id, "error": {"message": e}}).to_string()),
    }
}

/// The plugin-visible method surface — reads plus `command.execute`.
fn dispatch(
    host: &mut dyn CapabilityHost,
    actor: &Actor,
    method: &str,
    params: &Value,
) -> Result<Value, String> {
    match method {
        "session.end" => Ok(json!({"bye": true})),
        "project.info" => {
            let p = host.project();
            Ok(json!({
                "id": p.id.to_string(), "name": p.name,
                "object_count": p.objects.len(), "relation_count": p.relations.len(),
            }))
        }
        "object.list" => {
            let ty = params.get("type").and_then(|t| t.as_str());
            let out: Vec<Value> = host
                .project()
                .sorted_objects()
                .iter()
                .filter(|o| ty.map(|t| o.type_id.0 == *t).unwrap_or(true))
                .map(|o| json!({"id": o.id.to_string(), "name": o.name, "type": o.type_id}))
                .collect();
            Ok(json!({"objects": out}))
        }
        "object.get" => {
            let key = params
                .get("id")
                .or_else(|| params.get("name"))
                .and_then(|v| v.as_str())
                .ok_or("provide id or name")?;
            let oid = host.resolve_object(key).ok_or("object not found")?;
            serde_json::to_value(host.project().get(oid)).map_err(|e| e.to_string())
        }
        "project.search" => {
            let q: worldos_kernel::SearchQuery =
                serde_json::from_value(params.clone()).map_err(|e| e.to_string())?;
            let out: Vec<Value> = worldos_kernel::search(host.project(), &q)
                .into_iter()
                .map(|o| json!({"id": o.id.to_string(), "name": o.name, "type": o.type_id}))
                .collect();
            Ok(json!({"results": out}))
        }
        "command.list" => serde_json::to_value(host.command_schemas()).map_err(|e| e.to_string()),
        "command.execute" => {
            let ty = params
                .get("type")
                .and_then(|t| t.as_str())
                .ok_or("missing type")?;
            let input = params.get("input").cloned().unwrap_or(json!({}));
            host.run_command_as(actor, ty, input)
                .map_err(|e| e.to_string())
        }
        "validation.run" => serde_json::to_value(host.validate().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string()),
        _ => Err(format!("unknown method `{method}`")),
    }
}

// ------------------------------------------------------------------ discovery

/// Build the spawn command for a plugin file, picking an interpreter for
/// script extensions (`.py` → python, `.ps1` → powershell, `.cmd`/`.bat`
/// → cmd). Native executables and extensionless files run directly.
pub fn spawn_command(spec: &PluginSpec) -> Command {
    let ext = spec
        .program
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let (prog, mut pre): (String, Vec<String>) = match ext.as_str() {
        "py" => ("python".into(), vec![spec.program.display().to_string()]),
        "ps1" => (
            "powershell".into(),
            vec![
                "-NoProfile".into(),
                "-File".into(),
                spec.program.display().to_string(),
            ],
        ),
        "cmd" | "bat" => (
            "cmd".into(),
            vec!["/c".into(), spec.program.display().to_string()],
        ),
        _ => (spec.program.display().to_string(), vec![]),
    };
    pre.extend(spec.args.iter().cloned());
    let mut c = Command::new(prog);
    c.args(pre);
    c
}

/// Directories searched for plugins, in priority order.
pub fn plugin_dirs(project_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = project_dir {
        dirs.push(d.join("plugins"));
    }
    dirs.push(PathBuf::from("plugins"));
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        dirs.push(PathBuf::from(home).join(".worldos").join("plugins"));
    }
    if let Some(extra) = std::env::var_os("WORLDOS_PLUGIN_PATH") {
        dirs.extend(std::env::split_paths(&extra));
    }
    dirs
}

/// Scan a directory for `worldos-plugin-*` files.
pub fn discover(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return found;
    };
    for e in rd.flatten() {
        let path = e.path();
        let Some(stem) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if path.is_file() && stem.starts_with("worldos-plugin-") {
            let name = stem
                .trim_end_matches(".exe")
                .trim_end_matches(".py")
                .trim_end_matches(".ps1")
                .trim_end_matches(".cmd")
                .trim_end_matches(".bat")
                .trim_start_matches("worldos-plugin-")
                .to_string();
            found.push((name, path));
        }
    }
    found.sort();
    found
}

/// Resolve `name-or-path` to a plugin file: literal path first, then
/// `worldos-plugin-<name>` across the discovery dirs and PATH.
pub fn resolve(name_or_path: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let p = PathBuf::from(name_or_path);
    if (p.components().count() > 1 || p.extension().is_some()) && p.is_file() {
        return Some(p);
    }
    let want = format!("worldos-plugin-{name_or_path}");
    for d in dirs {
        for (n, path) in discover(d) {
            if n == name_or_path {
                return Some(path);
            }
        }
    }
    // PATH lookup for the executable name (any extension).
    which(&want)
}

fn which(name: &str) -> Option<PathBuf> {
    std::process::Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg(name)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.lines().next().map(|l| PathBuf::from(l.trim())))
}

// ------------------------------------------------------------------ capability

/// `plugin.run` capability — agents and RPC clients invoke hosted plugins
/// through this, so the same session host serves every surface.
pub struct PluginRun;

impl crate::registry::Capability for PluginRun {
    fn descriptor(&self) -> crate::descriptor::CapabilityDescriptor {
        crate::descriptor::CapabilityDescriptor::new(
            "plugin.run",
            "Run a hosted plugin: subprocess speaking JSON-RPC, one attributed transaction",
            json!({
                "type": "object",
                "required": ["plugin"],
                "properties": {
                    "plugin": {"type": "string", "description": "plugin name or path"},
                    "args": {"type": "array", "items": {"type": "string"}},
                    "timeout_ms": {"type": "integer"}
                }
            }),
        )
        .requires(&[
            worldos_kernel::known::permissions::SHELL_EXECUTE,
            worldos_kernel::known::permissions::COMMAND_EXECUTE,
        ])
    }
    fn execute(
        &self,
        host: &mut dyn CapabilityHost,
        input: &Value,
    ) -> Result<Value, CapabilityError> {
        let name = input["plugin"]
            .as_str()
            .ok_or_else(|| fail("missing `plugin`"))?;
        let dirs = plugin_dirs(host.project_path().as_deref().and_then(|p| p.parent()));
        let program =
            resolve(name, &dirs).ok_or_else(|| fail(format!("plugin `{name}` not found")))?;
        let spec = PluginSpec {
            name: PathBuf::from(name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(name)
                .trim_start_matches("worldos-plugin-")
                .to_string(),
            program,
            args: input
                .get("args")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            timeout_ms: input
                .get("timeout_ms")
                .and_then(|t| t.as_u64())
                .unwrap_or(DEFAULT_TIMEOUT_MS),
        };
        let out = run_plugin(host, &spec)?;
        serde_json::to_value(out).map_err(CapabilityError::Serde)
    }
}
