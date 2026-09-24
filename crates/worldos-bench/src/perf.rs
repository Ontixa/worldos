//! Performance baselines (NEXT.md item 10): measured wall-time for
//! create, save, reopen, query and undo at increasing object counts.
//!
//! Deliberately simple and honest: each size runs once against a real
//! `.worldos` file on disk through the same command path users hit —
//! every `object.create` is its own committed transaction, so history
//! grows with the graph and `save`/`reopen` pay for it. Numbers carry
//! an environment note and are regression baselines, not statistical
//! claims or SLAs.

use std::time::Instant;

use serde::Serialize;
use serde_json::{Value, json};
use worldos_engine::Engine;
use worldos_kernel::SearchQuery;

/// Largest size a run may request — a sanity bound, not a scale claim:
/// 100k+ object files carry real serialization/fsync cost here.
pub const MAX_PERF_OBJECTS: usize = 1_000_000;

/// One measured size point.
#[derive(Debug, Clone, Serialize)]
pub struct PerfRun {
    /// Objects created by the run.
    pub objects: usize,
    /// `Engine::create` — the fsync-bound first save of an empty file.
    pub project_create_ms: u128,
    /// `objects` × `object.create`, each its own committed transaction
    /// (the real command path — history grows with every call).
    pub create_ms: u128,
    /// `create_ms` amortized per object, µs.
    pub create_us_per_object: u128,
    /// `Engine::save` of the full project + transaction history.
    pub save_ms: u128,
    /// `Engine::open` of the saved file.
    pub reopen_ms: u128,
    /// Average `find_by_name` over `query_lookups` probes — an O(n)
    /// scan in the kernel today, measured on the most recent objects.
    pub find_by_name_us: u128,
    /// One `search` (type filter + full sort + limit) over all objects.
    pub search_ms: u128,
    /// Undo then redo of the last transaction (one-op transactions —
    /// these measure cursor mechanics, not state size).
    pub undo_ms: u128,
    pub redo_ms: u128,
    /// `.worldos` file size on disk after save.
    pub file_bytes: u64,
    /// Committed transaction records — one per `object.create` here.
    pub history_records: usize,
    /// Probe count behind `find_by_name_us`.
    pub query_lookups: usize,
}

/// Whole perf run — the `worldos-bench perf` report artifact.
#[derive(Debug, Serialize)]
pub struct PerfReport {
    pub format_version: u32,
    pub generated_at: String,
    /// Build profile the numbers came from (`dev` builds still compile
    /// dependencies at `opt-level = 2` per the workspace profile).
    pub profile: String,
    /// Host context + the honest caveat — these are single-run numbers.
    pub environment: Value,
    pub runs: Vec<PerfRun>,
    pub summary: Value,
}

/// Measure every requested size (sorted, deduplicated).
pub fn run_perf(sizes: &[usize]) -> Result<PerfReport, String> {
    let mut sizes = sizes.to_vec();
    sizes.sort_unstable();
    sizes.dedup();
    if sizes.is_empty() {
        return Err("no sizes given".into());
    }
    let mut runs = Vec::new();
    for &n in &sizes {
        if n == 0 || n > MAX_PERF_OBJECTS {
            return Err(format!("size {n} out of range 1..={MAX_PERF_OBJECTS}"));
        }
        runs.push(measure(n)?);
    }
    let total_ms: u128 = runs
        .iter()
        .map(|r| r.project_create_ms + r.create_ms + r.save_ms + r.reopen_ms)
        .sum();
    Ok(PerfReport {
        format_version: 1,
        generated_at: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
        profile: if cfg!(debug_assertions) {
            "dev (deps at opt-level=2)".into()
        } else {
            "release".into()
        },
        environment: json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "caveat": "single-run wall times on a Windows dev host (NTFS, antivirus scanning); `synchronous=FULL` makes create/save fsync-bound — treat as regression baselines, not SLAs",
        }),
        summary: json!({
            "sizes": sizes,
            "duration_ms": total_ms,
        }),
        runs,
    })
}

fn measure(n: usize) -> Result<PerfRun, String> {
    let tmp = tempfile::tempdir().map_err(|e| format!("tempdir: {e}"))?;
    let path = tmp.path().join(format!("perf-{n}.worldos"));

    let t = Instant::now();
    let mut engine = Engine::create(format!("bench-perf-{n}"), &path)
        .map_err(|e| format!("engine create: {e}"))?;
    let project_create_ms = t.elapsed().as_millis();

    let t = Instant::now();
    for i in 0..n {
        engine
            .execute(
                "object.create",
                json!({"type": "core:note", "name": format!("obj-{i:07}")}),
            )
            .map_err(|e| format!("object.create #{i}: {e}"))?;
    }
    let create_ms = t.elapsed().as_millis();

    let t = Instant::now();
    engine.save().map_err(|e| format!("save: {e}"))?;
    let save_ms = t.elapsed().as_millis();
    let file_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let history_records = engine.history().records.len();
    drop(engine);

    let t = Instant::now();
    let mut engine = Engine::open(&path).map_err(|e| format!("reopen: {e}"))?;
    let reopen_ms = t.elapsed().as_millis();

    // `find_by_name` is an O(n) scan — probe the most recently created
    // objects, which sit at the end of the iteration order.
    let lookups = n.min(200);
    let t = Instant::now();
    for i in 0..lookups {
        let name = format!("obj-{:07}", n - 1 - (i % n));
        if engine.find_object(&name).is_none() {
            return Err(format!("probe object `{name}` missing"));
        }
    }
    let find_by_name_us = t.elapsed().as_micros() / lookups.max(1) as u128;

    let t = Instant::now();
    let hits = engine.search(&SearchQuery {
        type_id: Some("core:note".into()),
        limit: Some(50),
        ..Default::default()
    });
    let search_ms = t.elapsed().as_millis();
    if hits.len() != n.min(50) {
        return Err(format!(
            "search returned {} hits (want {})",
            hits.len(),
            n.min(50)
        ));
    }

    let t = Instant::now();
    engine.undo().map_err(|e| format!("undo: {e}"))?;
    let undo_ms = t.elapsed().as_millis();
    let t = Instant::now();
    engine.redo().map_err(|e| format!("redo: {e}"))?;
    let redo_ms = t.elapsed().as_millis();

    Ok(PerfRun {
        objects: n,
        project_create_ms,
        create_ms,
        create_us_per_object: create_ms.saturating_mul(1000) / n as u128,
        save_ms,
        reopen_ms,
        find_by_name_us,
        search_ms,
        undo_ms,
        redo_ms,
        file_bytes,
        history_records,
        query_lookups: lookups,
    })
}
