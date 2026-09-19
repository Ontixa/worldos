//! WorldBench v0 — deterministic task runner for WorldOS.
//!
//! A *task* is a YAML file describing a linear sequence of steps. Each
//! step is either a real engine command (`cad.create_box`, …) or an
//! engine lifecycle op (`engine.save`, `engine.reopen`, `engine.undo`,
//! `engine.redo`, `engine.save_as`). Steps carry `expect` checks that
//! are evaluated against the command receipt (or error).
//!
//! The runner is deliberately not an agent: it is the non-LLM
//! baseline. Same engine, same commands, same receipts — the record
//! JSON is the evidence an agent run must later match or beat.
//!
//! Task file sketch:
//!
//! ```yaml
//! id: graph-crud
//! title: object + relation lifecycle
//! steps:
//!   - name: create
//!     command: object.create
//!     input: {type: core:note, name: memo}
//!     expect:
//!       - {kind: ok}
//!       - {kind: present, path: "output.id"}
//!   - command: engine.undo
//!     expect:
//!       - {kind: ok}
//!       - {kind: query_absent, name: memo}
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use worldos_engine::Engine;

/// One expect-check inside a step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Check {
    /// Step must succeed.
    Ok,
    /// Step must fail; `contains` optionally matches the error text.
    Error {
        #[serde(default)]
        contains: Option<String>,
    },
    /// `path` (dot-separated into receipt output) must equal `value`.
    Eq { path: String, value: Value },
    /// Numeric `path` must be within `rel` relative error of `value`.
    Approx {
        path: String,
        value: f64,
        #[serde(default = "default_rel")]
        rel: f64,
    },
    /// `path` must exist and be non-null in the output.
    Present { path: String },
    /// No object named `name` may exist in the project afterwards.
    QueryAbsent { name: String },
    /// An object named `name` must exist afterwards; `path` optionally
    /// dot-resolves into its component data and must equal `value`.
    QueryObject {
        name: String,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        value: Option<Value>,
    },
}

fn default_rel() -> f64 {
    1e-4
}

/// A single step in a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    #[serde(default)]
    pub name: Option<String>,
    /// Bind the step's receipt output to a variable usable later as
    /// `${name.path}` inside `input` strings.
    #[serde(default)]
    pub save: Option<String>,
    pub command: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub expect: Vec<Check>,
}

/// A parsed task file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// Intentional skip reason — the task is reported `skipped`, never
    /// counted as pass or fail. Absent ≠ skipped.
    #[serde(default)]
    pub skip: Option<String>,
    pub steps: Vec<Step>,
}

/// Static validation: reject tasks that could pass vacuously.
/// - every step must carry at least one `expect` check
/// - unknown check kinds are already rejected by serde's tagged enum
/// - a task must have steps and an id
fn validate_task(file: &Path, task: &Task) -> Result<(), String> {
    if task.id.trim().is_empty() {
        return Err(format!("{file:?}: task id is empty"));
    }
    if task.steps.is_empty() {
        return Err(format!("{file:?}: task has no steps"));
    }
    for (i, s) in task.steps.iter().enumerate() {
        if s.expect.is_empty() {
            return Err(format!(
                "{file:?}: step {i} ({}) has no `expect` checks — a step without assertions cannot detect regressions",
                s.command
            ));
        }
    }
    Ok(())
}

/// Evidence for one evaluated check.
#[derive(Debug, Clone, Serialize)]
pub struct CheckRecord {
    pub check: Check,
    pub passed: bool,
    pub detail: String,
}

/// Evidence for one executed step.
#[derive(Debug, Clone, Serialize)]
pub struct StepRecord {
    pub index: usize,
    pub name: Option<String>,
    pub command: String,
    pub input: Value,
    /// `"ok"` | `"error"` | `"check_failed"`
    pub status: String,
    /// Receipt output or error text.
    pub output: Value,
    pub duration_ms: u128,
    pub checks: Vec<CheckRecord>,
}

/// Per-task record.
#[derive(Debug, Clone, Serialize)]
pub struct TaskRecord {
    pub id: String,
    pub title: String,
    pub file: String,
    /// `passed` | `failed` | `skipped` | `setup_failed` — setup/kernel
    /// failures are NOT test failures and intentional skips are NOT
    /// passes; both are surfaced distinctly.
    pub status: String,
    #[serde(default)]
    pub skip_reason: Option<String>,
    pub passed: bool,
    pub duration_ms: u128,
    pub steps: Vec<StepRecord>,
}

/// Whole-run report — the WorldBench artifact.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub format_version: u32,
    pub generated_at: String,
    pub kernel: String,
    /// Machine/config + provenance metadata for reproducibility.
    pub meta: Value,
    /// Task ids + step counts actually executed.
    pub inputs: Value,
    pub tasks: Vec<TaskRecord>,
    pub summary: Value,
}

/// Load every `*.yaml`/`*.yml` task in `dir`, sorted by file name for
/// determinism.
pub fn load_tasks(dir: &Path) -> Result<Vec<(PathBuf, Task)>, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read task dir {dir:?}: {e}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("yaml") | Some("yml")
            )
        })
        .collect();
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).map_err(|e| format!("cannot read {f:?}: {e}"))?;
        let task: Task = serde_yaml::from_str(&text).map_err(|e| format!("bad task {f:?}: {e}"))?;
        validate_task(&f, &task)?;
        out.push((f, task));
    }
    Ok(out)
}

fn dig<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    // `output.*` reads naturally in task files — strip the prefix
    // since `v` already IS the receipt output.
    let path = path.strip_prefix("output.").unwrap_or(path);
    let mut cur = v;
    for seg in path.split('.') {
        cur = match cur.get(seg) {
            Some(next) => next,
            // numeric segment indexes into arrays ("regenerated.0.name")
            None => cur
                .as_array()
                .and_then(|a| seg.parse::<usize>().ok().and_then(|i| a.get(i)))?,
        };
    }
    Some(cur)
}

fn eval_check(check: &Check, engine: &Engine, ok: bool, out: &Value, err: &str) -> (bool, String) {
    match check {
        Check::Ok => (
            ok,
            if ok {
                "ok".into()
            } else {
                format!("failed: {err}")
            },
        ),
        Check::Error { contains } => {
            if ok {
                (false, "expected error, step succeeded".into())
            } else if let Some(c) = contains {
                (
                    err.contains(c.as_str()),
                    format!("error `{err}` contains `{c}`"),
                )
            } else {
                (true, format!("failed as expected: {err}"))
            }
        }
        Check::Eq { path, value } => {
            if !ok {
                return (false, format!("step failed: {err}"));
            }
            let got = dig(out, path);
            (
                got == Some(value),
                format!("{path} = {:?} (want {:?})", got, value),
            )
        }
        Check::Approx { path, value, rel } => {
            if !ok {
                return (false, format!("step failed: {err}"));
            }
            match dig(out, path).and_then(|v| v.as_f64()) {
                Some(got) => {
                    let ok = worldos_cad::approx_relative(got, *value)
                        || (got - value).abs() <= rel * value.abs().max(1.0);
                    (ok, format!("{path} = {got} (want ~{value} rel {rel})"))
                }
                None => (false, format!("{path} not a number")),
            }
        }
        Check::Present { path } => {
            if !ok {
                return (false, format!("step failed: {err}"));
            }
            (
                dig(out, path).is_some(),
                format!("{path} present = {}", dig(out, path).is_some()),
            )
        }
        Check::QueryAbsent { name } => {
            let absent = engine.project().find_by_name(name).is_none();
            (absent, format!("object `{name}` absent = {absent}"))
        }
        Check::QueryObject { name, path, value } => {
            let Some(obj) = engine.project().find_by_name(name) else {
                return (false, format!("object `{name}` not found"));
            };
            if let (Some(p), Some(want)) = (path, value) {
                // path is "component.field.subfield": resolve into the
                // object's component map
                let mut parts = p.splitn(2, '.');
                let comp = parts.next().unwrap_or("");
                let rest = parts.next().unwrap_or("");
                let found = obj.components.get(comp).and_then(|c| {
                    if rest.is_empty() {
                        Some(&c.data)
                    } else {
                        dig(&c.data, rest)
                    }
                });
                let matched = found == Some(want);
                (
                    matched,
                    format!("`{name}` {p} = {:?} (want {:?})", found, want),
                )
            } else {
                (true, format!("object `{name}` exists"))
            }
        }
    }
}

/// Provenance + machine metadata for the report. The commit is read
/// from `WORLDOS_COMMIT` or `git rev-parse HEAD`; `unknown` is honest.
fn run_meta(kernel_name: &str, tasks: &[(PathBuf, Task)]) -> (Value, Value) {
    let commit = std::env::var("WORLDOS_COMMIT").ok().or_else(|| {
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
    });
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".into());
    let meta = json!({
        "commit": commit.unwrap_or_else(|| "unknown".into()),
        "worldos_version": env!("CARGO_PKG_VERSION"),
        "kernel": kernel_name,
        "cadrum_version": "0.8.20",
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        "host": host,
    });
    let inputs = json!(
        tasks
            .iter()
            .map(|(f, t)| json!({
                "file": f.file_name().unwrap_or_default().to_string_lossy(),
                "id": t.id,
                "steps": t.steps.len(),
                "skipped": t.skip.is_some(),
            }))
            .collect::<Vec<_>>()
    );
    (meta, inputs)
}

/// Run every task in `dir` under a fresh engine per task (tempdir
/// project file, cadrum kernel attached). Deterministic: sorted task
/// files, ordered steps, no clocks in the graph. A task whose SETUP
/// fails (engine create / kernel attach) is recorded `setup_failed` —
/// the rest of the suite still runs.
pub fn run(dir: &Path) -> Result<Report, String> {
    let tasks = load_tasks(dir)?;
    let kernel = worldos_adapter_cadrum::CadrumKernel::new();
    let kernel_name = worldos_cad::CadKernel::name(&kernel).to_string();
    let (meta, inputs) = run_meta(&kernel_name, &tasks);

    let mut records = Vec::new();
    for (file, task) in &tasks {
        records.push(run_task(file, task));
    }
    let count = |s: &str| records.iter().filter(|r| r.status == s).count();
    let report = Report {
        format_version: 2,
        generated_at: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
        kernel: kernel_name,
        meta,
        inputs,
        summary: json!({
            "total": records.len(),
            "passed": count("passed"),
            "failed": count("failed"),
            "skipped": count("skipped"),
            "setup_failed": count("setup_failed"),
            "duration_ms": records.iter().map(|r| r.duration_ms).sum::<u128>(),
        }),
        tasks: records,
    };
    Ok(report)
}

fn empty_record(task: &Task, file: &Path, status: &str, reason: Option<String>) -> TaskRecord {
    TaskRecord {
        id: task.id.clone(),
        title: task.title.clone(),
        file: file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
        status: status.into(),
        skip_reason: reason,
        passed: false,
        duration_ms: 0,
        steps: Vec::new(),
    }
}

fn run_task(file: &Path, task: &Task) -> TaskRecord {
    let started = Instant::now();
    if let Some(reason) = &task.skip {
        let mut r = empty_record(task, file, "skipped", Some(reason.clone()));
        r.duration_ms = started.elapsed().as_millis();
        return r;
    }
    let tmp = match tempfile::tempdir() {
        Ok(t) => t,
        Err(e) => {
            return empty_record(task, file, "setup_failed", Some(format!("tempdir: {e}")));
        }
    };
    let project_path = tmp.path().join(format!("{}.worldos", task.id));

    let mut engine = match Engine::create(&task.id, &project_path) {
        Ok(e) => e,
        Err(e) => {
            return empty_record(
                task,
                file,
                "setup_failed",
                Some(format!("engine create: {e}")),
            );
        }
    };
    if let Err(e) = engine.attach_cad(Arc::new(worldos_adapter_cadrum::CadrumKernel::new())) {
        return empty_record(task, file, "setup_failed", Some(format!("attach_cad: {e}")));
    }

    let mut steps = Vec::new();
    let mut task_ok = true;
    let mut vars: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    for (i, step) in task.steps.iter().enumerate() {
        let t0 = Instant::now();
        let input = interpolate(&step.input, &vars);
        let (ok, out, err) = match exec_step(&mut engine, &project_path, &step.command, &input) {
            Ok(t) => t,
            Err(e) => (false, Value::Null, format!("runner: {e}")),
        };
        let mut checks = Vec::new();
        let mut step_ok = true;
        for c in &step.expect {
            let (passed, detail) = eval_check(c, &engine, ok, &out, &err);
            step_ok &= passed;
            checks.push(CheckRecord {
                check: c.clone(),
                passed,
                detail,
            });
        }
        let status = if !ok {
            if step.expect.iter().any(|c| matches!(c, Check::Error { .. })) && step_ok {
                "ok" // expected failure that matched
            } else {
                "error"
            }
        } else if step_ok {
            "ok"
        } else {
            "check_failed"
        };
        if status != "ok" {
            task_ok = false;
        }
        if let Some(var) = &step.save
            && ok
        {
            vars.insert(var.clone(), out.clone());
        }
        steps.push(StepRecord {
            index: i,
            name: step.name.clone(),
            command: step.command.clone(),
            input: input.clone(),
            status: status.into(),
            output: if ok { out } else { json!({"error": err}) },
            duration_ms: t0.elapsed().as_millis(),
            checks,
        });
    }

    TaskRecord {
        id: task.id.clone(),
        title: task.title.clone(),
        file: file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
        status: if task_ok { "passed" } else { "failed" }.into(),
        skip_reason: None,
        passed: task_ok,
        duration_ms: started.elapsed().as_millis(),
        steps,
    }
}

/// Replace `"${var.path}"` strings in `input` with values captured by
/// earlier `save:` steps.
fn interpolate(input: &Value, vars: &std::collections::HashMap<String, Value>) -> Value {
    match input {
        Value::String(s) => {
            if let Some(rest) = s.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
                let mut parts = rest.splitn(2, '.');
                let var = parts.next().unwrap_or("");
                let path = parts.next().unwrap_or("");
                if let Some(v) = vars.get(var) {
                    if path.is_empty() {
                        return v.clone();
                    }
                    if let Some(found) = dig(v, path) {
                        return found.clone();
                    }
                }
                return Value::Null;
            }
            input.clone()
        }
        Value::Array(a) => Value::Array(a.iter().map(|v| interpolate(v, vars)).collect()),
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| (k.clone(), interpolate(v, vars)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Execute one step. `engine.*` commands are lifecycle ops handled by
/// the runner; everything else goes through `Engine::execute`.
fn exec_step(
    engine: &mut Engine,
    project_path: &Path,
    command: &str,
    input: &Value,
) -> Result<(bool, Value, String), String> {
    match command {
        "engine.save" => match engine.save() {
            Ok(()) => Ok((true, json!({"saved": true}), String::new())),
            Err(e) => Ok((false, Value::Null, e.to_string())),
        },
        "engine.save_as" => {
            let name = input
                .get("file")
                .and_then(|f| f.as_str())
                .unwrap_or("moved.worldos");
            let p = project_path.parent().unwrap_or(project_path).join(name);
            let res = if input
                .get("overwrite")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                engine.save_as_opts(&p, worldos_engine::SaveOptions { overwrite: true })
            } else {
                engine.save_as(&p)
            };
            match res {
                Ok(()) => Ok((
                    true,
                    json!({"saved_as": p.to_string_lossy()}),
                    String::new(),
                )),
                Err(e) => Ok((false, Value::Null, e.to_string())),
            }
        }
        // ----- failure injection for persistence regression tasks -----
        // `engine.touch` creates an occupied file so save_as hits the
        // destination-exists guard.
        "engine.touch" => {
            let name = input
                .get("file")
                .and_then(|f| f.as_str())
                .ok_or("engine.touch: missing file")?;
            let p = project_path.parent().unwrap_or(project_path).join(name);
            let content = input
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("occupied")
                .as_bytes();
            match std::fs::write(&p, content) {
                Ok(()) => Ok((true, json!({"touched": p.to_string_lossy()}), String::new())),
                Err(e) => Ok((false, Value::Null, e.to_string())),
            }
        }
        // Corrupt / delete a blob in the project sidecar: pick `sha`
        // prefix if given, else the lexically-first stored blob.
        "engine.corrupt_artifact" | "engine.delete_artifact" => {
            let store = match engine.cad() {
                Some(c) => c.artifacts(),
                None => return Ok((false, Value::Null, "no cad services".into())),
            };
            let want = input.get("sha").and_then(|s| s.as_str()).map(String::from);
            match store.list() {
                Err(e) => Ok((false, Value::Null, e.to_string())),
                Ok(refs) => {
                    let pick = refs.iter().find(|r| {
                        want.as_ref()
                            .map(|w| r.to_string().contains(w.as_str()))
                            .unwrap_or(true)
                    });
                    let Some(r) = pick else {
                        return Ok((false, Value::Null, "no artifacts stored".into()));
                    };
                    let hex = r.to_string().replace("sha256:", "");
                    let path = store.dir().join("objects").join(&hex[..2]).join(&hex);
                    let res = if command == "engine.delete_artifact" {
                        std::fs::remove_file(&path)
                    } else {
                        std::fs::write(&path, b"corrupted-by-bench")
                    };
                    match res {
                        Ok(()) => Ok((true, json!({command: r.to_string()}), String::new())),
                        Err(e) => Ok((false, Value::Null, e.to_string())),
                    }
                }
            }
        }
        // A restricted actor for permission-denied regression steps.
        "engine.set_actor" => {
            let id = input
                .get("id")
                .and_then(|s| s.as_str())
                .unwrap_or("bench-actor");
            let mut a = worldos_kernel::actor::Actor::human(id);
            if let Some(perms) = input.get("permissions").and_then(|p| p.as_array()) {
                let mut set = worldos_kernel::actor::PermissionSet {
                    grants: Default::default(),
                };
                for p in perms.iter().filter_map(|p| p.as_str()) {
                    set.grant(worldos_kernel::actor::Permission::new(p));
                }
                a.permissions = set;
            }
            engine.set_actor(a);
            Ok((true, json!({"actor": id}), String::new()))
        }
        "engine.reset_actor" => {
            engine.set_actor(worldos_kernel::actor::Actor::human("local-user"));
            Ok((true, json!({"actor": "local-user"}), String::new()))
        }
        // Partial component edit through the governed `object.set_component`
        // command — used to plant stale selections / bad recipes honestly.
        "engine.set_component_field" => {
            let obj_name = input
                .get("object")
                .and_then(|s| s.as_str())
                .ok_or("set_component_field: missing object")?;
            let comp = input
                .get("component")
                .and_then(|s| s.as_str())
                .ok_or("set_component_field: missing component")?;
            let path = input
                .get("path")
                .and_then(|s| s.as_str())
                .ok_or("set_component_field: missing path")?;
            let value = input.get("value").cloned().unwrap_or(Value::Null);
            let Some(obj) = engine.project().find_by_name(obj_name) else {
                return Ok((false, Value::Null, format!("object `{obj_name}` not found")));
            };
            let Some(c) = obj.components.get(comp) else {
                return Ok((
                    false,
                    Value::Null,
                    format!("object `{obj_name}` has no {comp}"),
                ));
            };
            let mut data = c.data.clone();
            // dot-path write
            let mut cur = &mut data;
            let segs: Vec<&str> = path.split('.').collect();
            for s in &segs[..segs.len().saturating_sub(1)] {
                if !cur.is_object() {
                    *cur = json!({});
                }
                cur = cur.get_mut(s).unwrap();
            }
            cur[segs[segs.len() - 1]] = value;
            match engine.execute(
                "object.set_component",
                json!({"id": obj.id.to_string(), "component": comp, "data": data}),
            ) {
                Ok(r) => Ok((true, r.output, String::new())),
                Err(e) => Ok((false, Value::Null, e.to_string())),
            }
        }
        "engine.reopen" => {
            let path = input
                .get("file")
                .and_then(|f| f.as_str())
                .map(|f| project_path.parent().unwrap_or(project_path).join(f))
                .unwrap_or_else(|| project_path.to_path_buf());
            match Engine::open(&path) {
                Ok(mut e) => {
                    if let Err(err) =
                        e.attach_cad(Arc::new(worldos_adapter_cadrum::CadrumKernel::new()))
                    {
                        return Ok((false, Value::Null, format!("attach_cad: {err}")));
                    }
                    *engine = e;
                    Ok((
                        true,
                        json!({"reopened": path.to_string_lossy()}),
                        String::new(),
                    ))
                }
                Err(e) => Ok((false, Value::Null, e.to_string())),
            }
        }
        "engine.undo" => match engine.undo() {
            Ok(Some(tx)) => Ok((true, json!({"undone": tx.to_string()}), String::new())),
            Ok(None) => Ok((false, Value::Null, "nothing to undo".into())),
            Err(e) => Ok((false, Value::Null, e.to_string())),
        },
        "engine.redo" => match engine.redo() {
            Ok(Some(tx)) => Ok((true, json!({"redone": tx.to_string()}), String::new())),
            Ok(None) => Ok((false, Value::Null, "nothing to redo".into())),
            Err(e) => Ok((false, Value::Null, e.to_string())),
        },
        _ => match engine.execute(command, input.clone()) {
            Ok(receipt) => Ok((true, receipt.output, String::new())),
            Err(e) => Ok((false, Value::Null, e.to_string())),
        },
    }
}
