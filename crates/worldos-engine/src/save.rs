//! Staged save protocol for `.worldos` files.
//!
//! A saved project is TWO resources: the SQLite file and the
//! `<file>.artifacts/` sidecar directory. No filesystem operation makes
//! both atomic, so saving is a journaled two-phase protocol:
//!
//! 1. journal (`phase: "staged"`) written at `<file>.worldos-journal`
//! 2. every blob from the current artifact store is copied into the
//!    target sidecar, then every artifact reference reachable from the
//!    snapshot — objects AND full history including undone ops — is
//!    verified present in it
//! 3. the snapshot is written to `<file>.worldos-wip` (staged with
//!    `journal_mode=DELETE` → a single self-contained file), fsynced,
//!    and verified by a read-only load
//! 4. journal flipped to `phase: "committed"` — the point of no return
//! 5. the wip file is renamed over the target (stale `-wal`/`-shm`
//!    sidecars of the target removed first)
//! 6. journal deleted
//!
//! Recovery (`reconcile`, called by `Engine::open` and before every new
//! save): a `committed` journal means the rename may or may not have
//! happened — finish it idempotently. Anything else means the save
//! never committed — discard the wip staging and leave the pre-existing
//! target untouched.
//!
//! On any error before the commit point the caller's engine state
//! (`path`, `dirty`, bound artifact store) is left unchanged and the
//! staging files are removed; copied sidecar blobs may remain — they are
//! content-addressed and collectable, never corrupt.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use worldos_artifact::{ArtifactRef, ArtifactStore};
use worldos_store::{ProjectStore, Snapshot, SqliteStore};

use crate::engine::Engine;
use crate::error::EngineError;

/// How far the save may overwrite the destination.
#[derive(Debug, Clone, Copy, Default)]
pub struct SaveOptions {
    /// Permit replacing an existing file that is not the currently
    /// bound path. Without this grant a foreign file is never touched.
    pub overwrite: bool,
}

/// Boundaries where a save can fail — test hooks inject errors here to
/// exercise each failure frontier honestly.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveStage {
    /// Before copying blobs into the target sidecar.
    Artifacts,
    /// Before writing the staged wip database.
    WipWrite,
    /// Before read-back verification of the wip database.
    WipVerify,
    /// Before the journal is flipped to committed (point of no return).
    CommitPoint,
    /// Before renaming wip over the target.
    Rename,
    /// Before rebinding the in-memory artifact store pointer.
    Rebind,
}

fn suffixed(target: &Path, suffix: &str) -> PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// `<file>.worldos-wip` — the staged database.
pub fn wip_path(target: &Path) -> PathBuf {
    suffixed(target, "-wip")
}

/// `<file>.worldos-journal` — the commit marker.
pub fn journal_path(target: &Path) -> PathBuf {
    suffixed(target, "-journal")
}

fn wal_sidecars(target: &Path) -> [PathBuf; 2] {
    [suffixed(target, "-wal"), suffixed(target, "-shm")]
}

fn rm(p: &Path) -> Result<(), EngineError> {
    match std::fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(EngineError::Other(format!("remove {}: {e}", p.display()))),
    }
}

fn write_journal(target: &Path, phase: &str) -> Result<(), EngineError> {
    let j = json!({
        "v": 1,
        "op": "save",
        "phase": phase,
        "target": target.display().to_string(),
        "wip": wip_path(target).display().to_string(),
        "pid": std::process::id(),
        "at_ms": worldos_kernel::model::now_ms(),
    });
    std::fs::write(journal_path(target), j.to_string())
        .map_err(|e| EngineError::Other(format!("save journal: {e}")))
}

/// Move `wip` over `target`, clearing the target's stale WAL sidecars so
/// they can never be replayed against the new main file.
fn commit_rename(wip: &Path, target: &Path) -> Result<(), EngineError> {
    for s in wal_sidecars(target) {
        rm(&s)?;
    }
    std::fs::rename(wip, target).or_else(|e| {
        // Some platforms refuse rename over an existing file; the
        // journal already says `committed`, so a crash between remove
        // and rename is recovered by `reconcile`.
        rm(target).map_err(|r| std::io::Error::new(e.kind(), r.to_string()))?;
        std::fs::rename(wip, target)
    })
    .map_err(|e| EngineError::Other(format!("commit rename: {e}")))
}

/// Recover an interrupted save for `target`. Safe to call anytime.
pub fn reconcile(target: &Path) -> Result<(), EngineError> {
    let jp = journal_path(target);
    let wip = wip_path(target);
    let journal = match std::fs::read_to_string(&jp) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No commit marker: a leftover wip is from a save that never
            // reached the commit point — it is staging garbage.
            return rm(&wip);
        }
        Err(e) => return Err(EngineError::Other(format!("read save journal: {e}"))),
    };
    let phase = serde_json::from_str::<Value>(&journal)
        .ok()
        .and_then(|j| j["phase"].as_str().map(String::from))
        .unwrap_or_default();
    if phase == "committed" {
        // Point of no return was crossed: finish the rename if the crash
        // landed between the journal flip and the rename.
        if wip.exists() {
            commit_rename(&wip, target)?;
        }
    } else {
        // `staged`, torn, or unrecognized: the save never committed.
        rm(&wip)?;
    }
    rm(&jp)
}

/// Collect `sha256:`-prefixed strings from a serialized JSON tree.
fn collect_refs(v: Value, out: &mut HashSet<ArtifactRef>) {
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        match v {
            Value::String(s) => {
                if s.starts_with("sha256:")
                    && let Ok(r) = s.parse::<ArtifactRef>()
                {
                    out.insert(r);
                }
            }
            Value::Array(a) => stack.extend(a),
            Value::Object(m) => stack.extend(m.into_values()),
            _ => {}
        }
    }
}

/// Artifact refs live in the CURRENT graph (objects + relations). A
/// save must not commit while any of these is absent from the target
/// sidecar — reopening would be broken.
pub fn project_artifact_refs(snap: &Snapshot) -> HashSet<ArtifactRef> {
    let mut out = HashSet::new();
    if let Ok(v) = serde_json::to_value(&snap.project) {
        collect_refs(v, &mut out);
    }
    out
}

/// Every artifact reference reachable from the snapshot: current object
/// components, relations, and the full history (including undone
/// transactions — undo/redo can make them live again). Used as the GC
/// keep-set; refs only reachable via history are migration best-effort.
pub fn snapshot_artifact_refs(snap: &Snapshot) -> HashSet<ArtifactRef> {
    let mut out = HashSet::new();
    if let Ok(v) = serde_json::to_value(snap) {
        collect_refs(v, &mut out);
    }
    out
}

/// A cheap structural sanity check that the staged file round-trips.
fn verify_staged(wip: &Path, snap: &Snapshot) -> Result<(), EngineError> {
    let store = SqliteStore::open_readonly(wip).map_err(EngineError::Store)?;
    let back = store.load().map_err(EngineError::Store)?;
    let (p, q) = (&snap.project, &back.project);
    if p.id != q.id
        || p.objects.len() != q.objects.len()
        || p.relations.len() != q.relations.len()
        || snap.history.records.len() != back.history.records.len()
    {
        return Err(EngineError::Other(
            "staged save failed read-back verification".into(),
        ));
    }
    Ok(())
}

type FailHook<'a> = dyn FnMut(SaveStage) -> Result<(), EngineError> + 'a;

/// The whole save. `fail` is invoked before each stage; tests inject
/// errors to exercise every failure boundary for real.
pub(crate) fn save_project(
    e: &mut Engine,
    target: &Path,
    opts: SaveOptions,
    fail: &mut FailHook,
) -> Result<(), EngineError> {
    reconcile(target)?;
    let same_file = e
        .path
        .as_ref()
        .is_some_and(|cur| paths_equal(cur, target));
    if !same_file && !opts.overwrite && target.exists() {
        return Err(EngineError::DestinationExists(
            target.display().to_string(),
        ));
    }
    let snap = e.snapshot();
    let wip = wip_path(target);

    let run = |e: &mut Engine, fail: &mut FailHook| -> Result<Option<ArtifactStore>, EngineError> {
        // -- stage 1: artifacts into the target sidecar -------------------
        fail(SaveStage::Artifacts)?;
        let store = match &e.cad {
            Some(cad) => {
                let s = ArtifactStore::for_project(target)
                    .map_err(|e| EngineError::Other(e.to_string()))?;
                cad.migrate_to(&s)
                    .map_err(|e| EngineError::Other(format!("artifact migration: {e}")))?;
                let have: HashSet<ArtifactRef> =
                    s.list().map_err(|e| EngineError::Other(e.to_string()))?.into_iter().collect();
                // Live refs are mandatory — the committed file must be
                // openable with every artifact the graph needs.
                let missing: Vec<String> = project_artifact_refs(&snap)
                    .difference(&have)
                    .map(|r| r.to_string())
                    .collect();
                if !missing.is_empty() {
                    return Err(EngineError::Other(format!(
                        "artifact migration incomplete — {} live ref(s) absent from target sidecar: {}",
                        missing.len(),
                        missing.join(", ")
                    )));
                }
                Some(s)
            }
            None => None,
        };

        // -- stage 2: staged database -------------------------------------
        fail(SaveStage::WipWrite)?;
        write_journal(target, "staged")?;
        {
            let mut st = SqliteStore::open_staged(&wip).map_err(EngineError::Store)?;
            st.save(&snap).map_err(EngineError::Store)?;
        }
        std::fs::File::options()
            .read(true)
            .write(true)
            .open(&wip)
            .and_then(|f| f.sync_all())
            .map_err(|e| EngineError::Other(format!("fsync staged save: {e}")))?;

        fail(SaveStage::WipVerify)?;
        verify_staged(&wip, &snap)?;

        // -- stage 3: commit point ----------------------------------------
        fail(SaveStage::CommitPoint)?;
        write_journal(target, "committed")?;

        // -- stage 4: atomic handover -------------------------------------
        fail(SaveStage::Rename)?;
        commit_rename(&wip, target)?;
        rm(&journal_path(target))?;
        Ok(store)
    };

    match run(e, fail) {
        Ok(store) => {
            fail(SaveStage::Rebind)?;
            // Commit succeeded on disk; only now flip in-memory state.
            if let (Some(cad), Some(s)) = (&e.cad, store) {
                cad.set_store(s);
            }
            e.path = Some(target.to_path_buf());
            e.dirty = false;
            e.emit(worldos_kernel::events::EngineEvent::ProjectSaved {
                path: target.display().to_string(),
            });
            Ok(())
        }
        Err(err) => {
            // Roll back staging; the previous target (if any) is intact.
            let _ = rm(&wip);
            let _ = rm(&journal_path(target));
            Err(err)
        }
    }
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    match (std::path::absolute(a), std::path::absolute(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
