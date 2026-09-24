#![no_main]

//! Fuzz the hosted-plugin line protocol: arbitrary bytes become the
//! plugin's stdout stream served by `serve_stream` on a fresh in-memory
//! engine — request parsing, dispatch and response framing must never
//! panic on garbage, partial lines, or hostile command sequences.

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;
use worldos_capability::plugin::serve_stream;
use worldos_engine::Engine;
use worldos_kernel::Actor;

fuzz_target!(|data: &[u8]| {
    let mut engine = Engine::new("fuzz");
    let actor = Actor::plugin("fuzz");
    let reader = Cursor::new(data);
    let mut sink = Vec::new();
    let _ = serve_stream(&mut engine, &actor, reader, &mut sink);
});
