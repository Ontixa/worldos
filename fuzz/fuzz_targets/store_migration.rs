#![no_main]

//! Fuzz the on-disk/migration boundary two ways:
//! 1. bytes → `Snapshot` JSON (the serialized project+history envelope);
//! 2. bytes → a `.worldos` file opened by `SqliteStore` — `open` runs
//!    schema migration then `load` reads every row — on garbage,
//!    truncated, or hostile content both must fail as values, never
//!    panic or write outside the temp dir.

use libfuzzer_sys::fuzz_target;
use std::sync::atomic::{AtomicU64, Ordering};
use worldos_store::{ProjectStore, Snapshot, SqliteStore};

static CASE: AtomicU64 = AtomicU64::new(0);

fuzz_target!(|data: &[u8]| {
    if let Ok(snap) = serde_json::from_slice::<Snapshot>(data) {
        // A parsed snapshot must still replay cleanly through history.
        let mut project = snap.project.clone();
        snap.history.replay(&mut project);
    }

    let dir = std::env::temp_dir().join(format!("worldos-fuzz-{}", std::process::id()));
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join(format!("case-{}.worldos", CASE.fetch_add(1, Ordering::Relaxed)));
    if std::fs::write(&path, data).is_err() {
        return;
    }
    if let Ok(store) = SqliteStore::open(&path) {
        let _ = store.load();
    }
    let _ = std::fs::remove_file(&path);
});
