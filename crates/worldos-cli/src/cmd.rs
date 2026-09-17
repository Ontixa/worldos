//! Subcommand implementations — all through `Engine`.

use crate::{out, Cmd, PluginCmd};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use worldos_agent::AgentRun;
use worldos_engine::{diff_projects, Engine};
use worldos_rpc::RpcService;

pub fn run(cmd: Cmd, json_out: bool) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        Cmd::New { name, path } => {
            let path = path.unwrap_or_else(|| Path::new(&format!("{name}.worldos")).to_path_buf());
            let mut e = Engine::create(&name, &path)?;
            register_extras(&mut e);
            e.save()?;
            print(json_out, &json!({"created": path.display().to_string(), "name": name}), |v| {
                println!("created `{}` at {}", v["name"], v["created"]);
            });
        }
        Cmd::Open { file } | Cmd::Inspect { file, object: None } => {
            let e = open(&file)?;
            let info = info_json(&e);
            print(json_out, &info, |v| {
                out::header(&format!("WorldOS project: {}", v["name"]));
                out::kv("id", v["id"].as_str().unwrap());
                out::kv("schema", &v["schema_version"]);
                out::kv("objects", &v["object_count"]);
                out::kv("relations", &v["relation_count"]);
                out::kv("undo/redo", format!("{}/{}", v["can_undo"], v["can_redo"]));
            });
        }
        Cmd::Inspect { file, object: Some(key) } => {
            let e = open(&file)?;
            let obj = resolve(&e, &key).ok_or(format!("object `{key}` not found"))?;
            print(json_out, &serde_json::to_value(obj)?, |o| {
                println!("{}  ({})", o["name"].as_str().unwrap(), o["type_id"].as_str().unwrap());
                out::kv("id", o["id"].as_str().unwrap());
                for (ctype, comp) in o["components"].as_object().into_iter().flatten() {
                    println!("  ▣ {ctype} v{}", comp["version"]);
                    println!("{}", indent(&serde_json::to_string_pretty(&comp["data"]).unwrap_or_default()));
                }
            });
        }
        Cmd::Graph { file } => {
            let e = open(&file)?;
            let p = e.project();
            let nodes: Vec<Value> = p.sorted_objects().iter().map(|o| {
                json!({"id": o.id.to_string(), "name": o.name, "type": o.type_id})
            }).collect();
            let edges: Vec<Value> = p.relations.values().map(|r| {
                json!({"type": r.type_id, "from": r.from.to_string(), "to": r.to.to_string()})
            }).collect();
            print(json_out, &json!({"nodes": nodes, "edges": edges}), |g| {
                out::header("Objects");
                for n in g["nodes"].as_array().unwrap() {
                    println!("  {}  [{}]", n["name"].as_str().unwrap(), n["type"].as_str().unwrap());
                }
                out::header("Relations");
                for e in g["edges"].as_array().unwrap() {
                    let f = name_of(p, e["from"].as_str().unwrap());
                    let t = name_of(p, e["to"].as_str().unwrap());
                    println!("  {} ─{}→ {}", f, e["type"].as_str().unwrap(), t);
                }
            });
        }
        Cmd::Validate { file } => {
            let e = open(&file)?;
            let rep = e.validate();
            print(json_out, &rep, |r| {
                out::header("Validation");
                for d in &r.diagnostics {
                    println!("  [{:?}] {} ({})", d.severity, d.message, d.code);
                }
                println!("passed: {}", r.passed);
            });
            if !rep.passed {
                return Err("validation failed".into());
            }
        }
        Cmd::Command { file, command, input } => {
            let mut e = open(&file)?;
            let input: Value = input.map(|s| serde_json::from_str(&s)).transpose()?.unwrap_or(json!({}));
            let receipt = e.execute(&command, input)?;
            e.save()?;
            print(json_out, &receipt, |r| {
                println!("✓ {} → {}", command, r.output);
            });
        }
        Cmd::Batch { file, script } => {
            let mut e = open(&file)?;
            let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&script)?)?;
            e.begin_transaction(format!("batch {}", script.display()))?;
            let mut outputs = Vec::new();
            let res = (|| -> Result<(), Box<dyn std::error::Error>> {
                for s in &steps {
                    let ty = s["type"].as_str().ok_or("missing type")?;
                    let out = e.execute(ty, s.get("input").cloned().unwrap_or(json!({})))?;
                    outputs.push(out.output);
                }
                Ok(())
            })();
            match res {
                Ok(()) => {
                    e.commit_transaction()?;
                }
                Err(err) => {
                    e.rollback_transaction()?;
                    return Err(err);
                }
            }
            e.save()?;
            print(json_out, &json!({"outputs": outputs}), |v| {
                println!("✓ {} command(s) committed", v["outputs"].as_array().unwrap().len());
            });
        }
        Cmd::Commands { file } => {
            let e = open(&file)?;
            let schemas = e.command_schemas();
            print(json_out, &schemas, |list| {
                for s in list {
                    println!("{:<28} {}", s.command_type, s.description);
                }
            });
        }
        Cmd::Capabilities { file } => {
            let e = open(&file)?;
            let caps = e.capability_descriptors();
            print(json_out, &caps, |list| {
                for c in list {
                    println!("{:<24} {} ({})", c.id, c.description, c.provider.id);
                }
            });
        }
        Cmd::History { file, limit } => {
            let e = open(&file)?;
            let recs: Vec<Value> = e.history().records.iter().rev().take(limit).map(|r| {
                json!({"index": r.index, "id": r.id.to_string(), "actor": r.actor,
                       "label": r.label, "undone": r.undone,
                       "commands": r.commands.len(),
                       "ops": r.ops.iter().map(|o| o.describe()).collect::<Vec<_>>()})
            }).collect();
            print(json_out, &json!({"transactions": recs}), |v| {
                out::header("History (newest first)");
                for t in v["transactions"].as_array().unwrap() {
                    let flag = if t["undone"].as_bool().unwrap() { " (undone)" } else { "" };
                    println!("  #{:<3} {:<20} {}{}", t["index"], t["actor"], t["label"].as_str().unwrap(), flag);
                    for op in t["ops"].as_array().unwrap() {
                        println!("        • {}", op.as_str().unwrap());
                    }
                }
            });
        }
        Cmd::Undo { file } => {
            let mut e = open(&file)?;
            let id = e.undo()?;
            e.save()?;
            print(json_out, &json!({"undone": id.map(|t| t.to_string())}), |v| {
                match v["undone"].as_str() {
                    Some(t) => println!("undone transaction {t}"),
                    None => println!("nothing to undo"),
                }
            });
        }
        Cmd::Redo { file } => {
            let mut e = open(&file)?;
            let id = e.redo()?;
            e.save()?;
            print(json_out, &json!({"redone": id.map(|t| t.to_string())}), |v| {
                match v["redone"].as_str() {
                    Some(t) => println!("redone transaction {t}"),
                    None => println!("nothing to redo"),
                }
            });
        }
        Cmd::Diff { a, b } => {
            let ea = open(&a)?;
            let eb = open(&b)?;
            let entries = diff_projects(ea.project(), eb.project());
            print(json_out, &entries, |list| {
                for d in list {
                    println!("{}", serde_json::to_string(d).unwrap_or_default());
                }
                if list.is_empty() {
                    println!("(identical)");
                }
            });
        }
        Cmd::Agent { file, goal, agent } => {
            let mut e = open(&file)?;
            let report = e.run_capability(
                "agent.run",
                json!({"goal": goal, "agent": agent}),
            )?;
            e.save()?;
            print(json_out, &report, |r| {
                out::header("Agent run");
                out::kv("status", r["status"].as_str().unwrap_or(""));
                out::kv("summary", r["summary"].as_str().unwrap_or(""));
                for s in r["steps"].as_array().into_iter().flatten() {
                    println!("  {} {} — {}", s["index"], s["command"].as_str().unwrap(), s["note"].as_str().unwrap());
                }
                for v in r["verification"].as_array().into_iter().flatten() {
                    println!("  ✓ {}", v.as_str().unwrap());
                }
            });
        }
        Cmd::Export { file, out: outp } => {
            let e = open(&file)?;
            let text = serde_json::to_string_pretty(e.project())?;
            std::fs::write(&outp, &text)?;
            print(json_out, &json!({"path": outp.display().to_string()}), |v| {
                println!("exported → {}", v["path"]);
            });
        }
        Cmd::Mcp { file } => {
            let e = open(&file)?;
            let svc = RpcService::new(e);
            worldos_rpc::mcp::serve_stdio(&svc)?;
        }
        Cmd::Rpc { file } => {
            let e = open(&file)?;
            let svc = RpcService::new(e);
            worldos_rpc::stdio::serve_stdio(&svc)?;
        }
        Cmd::Serve { file, port } => {
            let e = open(&file)?;
            let svc = std::sync::Arc::new(RpcService::new(e));
            println!("worldos serve ws://127.0.0.1:{port} ({})", file.display());
            worldos_rpc::ws::serve_ws(svc, &format!("127.0.0.1:{port}"))?;
        }
        Cmd::Doctor => doctor(json_out)?,
        Cmd::Plugin { sub: PluginCmd::List } => {
            print(json_out, &json!({"plugins": [], "note": "plugin loader lands with Forge"}), |_| {
                println!("no external plugins loaded (builtin domains only)");
            });
        }
        Cmd::Version => println!("worldos {}", env!("CARGO_PKG_VERSION")),
    }
    Ok(())
}

fn open(file: &Path) -> Result<Engine, Box<dyn std::error::Error>> {
    let mut e = Engine::open(file)?;
    register_extras(&mut e);
    Ok(e)
}

/// Capabilities layered on the engine by this interface.
fn register_extras(e: &mut Engine) {
    e.register_capability(Arc::new(AgentRun));
}

fn resolve(e: &Engine, key: &str) -> Option<worldos_kernel::ObjectId> {
    if let Ok(id) = key.parse() {
        if e.get_object(id).is_some() {
            return Some(id);
        }
    }
    e.find_object(key).map(|o| o.id)
}

fn info_json(e: &Engine) -> Value {
    let p = e.project();
    json!({
        "id": p.id.to_string(), "name": p.name, "schema_version": p.schema_version,
        "object_count": p.objects.len(), "relation_count": p.relations.len(),
        "can_undo": e.can_undo(), "can_redo": e.can_redo(),
        "path": e.path().map(|p| p.display().to_string()),
    })
}

fn name_of(p: &worldos_kernel::Project, id: &str) -> String {
    id.parse()
        .ok()
        .and_then(|oid| p.get(oid))
        .map(|o| o.name.clone())
        .unwrap_or_else(|| id[..id.len().min(8)].to_string())
}

fn indent(s: &str) -> String {
    s.lines().map(|l| format!("      {l}")).collect::<Vec<_>>().join("\n")
}

fn print<T: serde::Serialize>(json: bool, v: &T, human: impl FnOnce(&T)) {
    if json {
        println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
    } else {
        human(v);
    }
}

fn doctor(json_out: bool) -> Result<(), Box<dyn std::error::Error>> {
    let checks = vec![
        ("rustc", which("rustc"), "Rust toolchain for building WorldOS"),
        ("cargo", which("cargo"), "Rust package manager"),
        ("node", which("node"), "Node.js for the desktop UI + TS SDK"),
        ("npm", which("npm"), "Node package manager"),
        ("sqlite3", which("sqlite3"), "optional: inspect .worldos files directly"),
    ];
    let rows: Vec<Value> = checks.iter().map(|(name, path, why)| {
        json!({"tool": name, "found": path.is_some(), "path": path, "purpose": why})
    }).collect();
    print(json_out, &json!({"checks": rows}), |v| {
        out::header("worldos doctor");
        for c in v["checks"].as_array().unwrap() {
            let mark = if c["found"].as_bool().unwrap() { "✓" } else { "✗" };
            println!("  {mark} {:<8} {}", c["tool"].as_str().unwrap(), c["purpose"].as_str().unwrap());
        }
    });
    Ok(())
}

fn which(tool: &str) -> Option<String> {
    std::process::Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg(tool)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.lines().next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
}
