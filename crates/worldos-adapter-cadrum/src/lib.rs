//! # worldos-adapter-cadrum
//!
//! [`CadKernel`] implementation over cadrum — statically linked
//! OCCT 8.0.1 (see `docs/adr/0006-cad-engine.md`).
//!
//! The adapter owns a session-scoped handle table: [`ShapeId`]s index
//! into a `Mutex<HashMap>` of live `cadrum::Solid`s. Everything the
//! trait returns is plain data, so callers never see an OCCT type.

mod kernel;

pub use kernel::CadrumKernel;
