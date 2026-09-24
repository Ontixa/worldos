//! WorldBench — deterministic task runner for WorldOS.
//!
//! A *task* is a YAML file describing a linear sequence of steps. Each
//! step is a real engine command (`cad.create_box`, …), an engine
//! lifecycle op (`engine.save`, `engine.reopen`, `engine.undo`,
//! `engine.redo`, `engine.save_as`), or a capability dispatch
//! (`capability.run` with `{id, input}` — covers `agent.run`,
//! `project.inspect`, `geometry.measure`, …).
//!
//! Steps carry `expect` checks evaluated against the command receipt
//! (or error) and live engine state. Beyond receipt assertions (`ok`,
//! `error`, `eq`, `approx`, `present`) the runner supports state-level
//! checks: `query_object` / `query_absent` / `field_absent`,
//! `object_count` / `relation_count`, `no_dangling_relations`, `valid`
//! (engine validators), `history` cursor invariants, `snapshot` state
//! digests (exact undo/redo, failure-purity and save/reopen equality),
//! `artifact_verified` (sidecar blob re-hash) and `file_digest`
//! (exported-file SHA-256 against the receipt's claim). See
//! [`checks::Check`] for the full vocabulary.
//!
//! `"${var.path}"` markers inside `input` (and inside check fields that
//! carry values, names or refs) resolve to earlier `save:`-bound
//! receipt outputs; `"${bench.dir}"` names the task's temp directory —
//! the only place file-producing steps should write.
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
//!       - {kind: object_count, count: 1}
//!   - command: engine.undo
//!     expect:
//!       - {kind: ok}
//!       - {kind: query_absent, name: memo}
//!       - {kind: history, records: 1, cursor: 0, can_redo: true}
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use worldos_engine::Engine;

mod checks;
mod perf;

pub use checks::{Check, project_digest};
pub use perf::{PerfReport, PerfRun, run_perf};

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
    pub steps: Vec<Step>,
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
        out.push((f, task));
    }
    Ok(out)
}

/// `output.foo.0.bar` → `foo.0.bar` — `output.*` reads naturally in
/// task files even though `v` already IS the receipt output. Numeric
/// segments index into arrays (`results.0.status`).
pub(crate) fn dig<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let path = path.strip_prefix("output.").unwrap_or(path);
    let mut cur = v;
    for seg in path.split('.') {
        cur = match cur {
            Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            _ => cur.get(seg)?,
        };
    }
    Some(cur)
}

/// Run every task in `dir` under a fresh engine per task (tempdir
/// project file, cadrum kernel attached, `agent.run` registered).
/// Deterministic: sorted task files, ordered steps, no clocks in the
/// graph.
pub fn run(dir: &Path) -> Result<Report, String> {
    let tasks = load_tasks(dir)?;
    let kernel = worldos_adapter_cadrum::CadrumKernel::new();
    let kernel_name = worldos_cad::CadKernel::name(&kernel).to_string();

    let mut records = Vec::new();
    for (file, task) in &tasks {
        records.push(run_task(file, task)?);
    }
    let passed = records.iter().filter(|r| r.passed).count();
    let report = Report {
        format_version: 2,
        generated_at: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
        kernel: kernel_name,
        summary: json!({
            "total": records.len(),
            "passed": passed,
            "failed": records.len() - passed,
            "duration_ms": records.iter().map(|r| r.duration_ms).sum::<u128>(),
        }),
        tasks: records,
    };
    Ok(report)
}

/// Attach the CAD kernel + the bench-registered capabilities every
/// interface shares (`agent.run` — same registration the CLI performs).
fn attach_services(engine: &mut Engine) -> Result<(), String> {
    engine
        .attach_cad(Arc::new(worldos_adapter_cadrum::CadrumKernel::new()))
        .map_err(|e| format!("attach_cad: {e}"))?;
    engine.register_capability(Arc::new(worldos_agent::AgentRun));
    Ok(())
}

fn run_task(file: &Path, task: &Task) -> Result<TaskRecord, String> {
    let tmp = tempfile::tempdir().map_err(|e| format!("tempdir: {e}"))?;
    let project_path = tmp.path().join(format!("{}.worldos", task.id));
    let started = Instant::now();

    let mut engine =
        Engine::create(&task.id, &project_path).map_err(|e| format!("engine create: {e}"))?;
    attach_services(&mut engine)?;

    // `bench.dir` is the only sanctioned target for file-producing
    // steps (`geometry.export`, …): the task's own tempdir.
    let mut vars: HashMap<String, Value> = HashMap::new();
    vars.insert(
        "bench".into(),
        json!({"dir": tmp.path().to_string_lossy(), "task": task.id}),
    );
    // Named `snapshot` digests — live across steps, die with the task.
    let mut snapshots: HashMap<String, String> = HashMap::new();

    let mut steps = Vec::new();
    let mut task_ok = true;
    for (i, step) in task.steps.iter().enumerate() {
        let t0 = Instant::now();
        let input = interpolate(&step.input, &vars);
        let (ok, out, err) = exec_step(&mut engine, &project_path, &step.command, &input)?;
        let mut checks = Vec::new();
        let mut step_ok = true;
        for c in &step.expect {
            let (passed, detail) = eval_check(c, &engine, ok, &out, &err, &vars, &mut snapshots);
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

    Ok(TaskRecord {
        id: task.id.clone(),
        title: task.title.clone(),
        file: file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
        passed: task_ok,
        duration_ms: started.elapsed().as_millis(),
        steps,
    })
}

fn eval_check(
    check: &Check,
    engine: &Engine,
    ok: bool,
    out: &Value,
    err: &str,
    vars: &HashMap<String, Value>,
    snapshots: &mut HashMap<String, String>,
) -> (bool, String) {
    checks::eval_check(check, engine, ok, out, err, vars, snapshots)
}

/// Replace `"${var.path}"` markers in `input` with values captured by
/// earlier `save:` steps (or the seeded `bench` var). A string that is
/// exactly one marker resolves to the stored value verbatim — including
/// non-strings; a longer string with embedded markers is rendered
/// inline (strings raw, everything else as compact JSON).
pub(crate) fn interpolate(input: &Value, vars: &HashMap<String, Value>) -> Value {
    match input {
        Value::String(s) => interpolate_str(s, vars),
        Value::Array(a) => Value::Array(a.iter().map(|v| interpolate(v, vars)).collect()),
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| (k.clone(), interpolate(v, vars)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// String half of [`interpolate`]: whole-string marker → verbatim
/// value; embedded markers → inline substitution.
pub(crate) fn interpolate_str(s: &str, vars: &HashMap<String, Value>) -> Value {
    if let Some(inner) = s
        .strip_prefix("${")
        .and_then(|r| r.strip_suffix('}'))
        .filter(|inner| !inner.contains(['$', '{', '}']))
    {
        return resolve_var(inner, vars);
    }
    if !s.contains("${") {
        return Value::String(s.to_string());
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let body = &rest[start + 2..];
        match body.find('}') {
            Some(end) => {
                out.push_str(&render(&resolve_var(&body[..end], vars)));
                rest = &body[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    Value::String(out)
}

/// `var.path` → the stored `save:` output, dug by `path`.
fn resolve_var(expr: &str, vars: &HashMap<String, Value>) -> Value {
    let mut parts = expr.splitn(2, '.');
    let var = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    match vars.get(var) {
        Some(v) if path.is_empty() => v.clone(),
        Some(v) => dig(v, path).cloned().unwrap_or(Value::Null),
        None => Value::Null,
    }
}

/// Render a resolved value for inline `${…}` substitution: strings raw,
/// anything else as compact JSON.
pub(crate) fn render(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Interpolate a check field and render it as a plain string — used
/// for names, refs and file paths in state-level checks.
pub(crate) fn interp_s(s: &str, vars: &HashMap<String, Value>) -> String {
    render(&interpolate_str(s, vars))
}

/// Execute one step. `engine.*` commands are lifecycle ops handled by
/// the runner, `capability.run` dispatches `{id, input}` through the
/// capability registry; everything else goes through `Engine::execute`.
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
            match engine.save_as(&p) {
                Ok(()) => Ok((
                    true,
                    json!({"saved_as": p.to_string_lossy()}),
                    String::new(),
                )),
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
                    if let Err(err) = attach_services(&mut e) {
                        return Ok((false, Value::Null, err));
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
        "capability.run" => {
            let id = input.get("id").and_then(|v| v.as_str()).unwrap_or_default();
            if id.is_empty() {
                return Ok((false, Value::Null, "capability.run needs `id`".into()));
            }
            let cap_input = input.get("input").cloned().unwrap_or(Value::Null);
            match engine.run_capability(id, cap_input) {
                Ok(v) => Ok((true, v, String::new())),
                Err(e) => Ok((false, Value::Null, e.to_string())),
            }
        }
        _ => match engine.execute(command, input.clone()) {
            Ok(receipt) => Ok((true, receipt.output, String::new())),
            Err(e) => Ok((false, Value::Null, e.to_string())),
        },
    }
}
