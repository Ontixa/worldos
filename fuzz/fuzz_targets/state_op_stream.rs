#![no_main]

//! Fuzz `StateOp` deserialization + `Project::apply`: arbitrary JSON is
//! decoded as an op stream, applied forward then backward. Apply must
//! stay total — errors are values, never panics.

use libfuzzer_sys::fuzz_target;
use worldos_kernel::{Project, StateOp};

fuzz_target!(|data: &[u8]| {
    let Ok(ops) = serde_json::from_slice::<Vec<StateOp>>(data) else {
        return;
    };
    let mut project = Project::new("fuzz");
    for op in &ops {
        let _ = project.apply(op, true);
    }
    for op in ops.iter().rev() {
        let _ = project.apply(op, false);
    }
});
