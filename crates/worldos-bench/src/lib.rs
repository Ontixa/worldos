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

fn dig<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    // `output.*` reads naturally in task files — strip the prefix
    // since `v` already IS the receipt output.
    let path = path.strip_prefix("output.").unwrap_or(path);
    let mut cur = v;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
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

/// Run every task in `dir` under a fresh engine per task (tempdir
/// project file, cadrum kernel attached). Deterministic: sorted task
/// files, ordered steps, no clocks in the graph.
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
        format_version: 1,
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

fn run_task(file: &Path, task: &Task) -> Result<TaskRecord, String> {
    let tmp = tempfile::tempdir().map_err(|e| format!("tempdir: {e}"))?;
    let project_path = tmp.path().join(format!("{}.worldos", task.id));
    let started = Instant::now();

    let mut engine =
        Engine::create(&task.id, &project_path).map_err(|e| format!("engine create: {e}"))?;
    engine
        .attach_cad(Arc::new(worldos_adapter_cadrum::CadrumKernel::new()))
        .map_err(|e| format!("attach_cad: {e}"))?;

    let mut steps = Vec::new();
    let mut task_ok = true;
    let mut vars: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    for (i, step) in task.steps.iter().enumerate() {
        let t0 = Instant::now();
        let input = interpolate(&step.input, &vars);
        let (ok, out, err) = exec_step(&mut engine, &project_path, &step.command, &input)?;
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
