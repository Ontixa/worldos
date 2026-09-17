//! # worldos-kernel
//!
//! The foundation of WorldOS: the Universal Project Graph.
//!
//! - [`model`]: objects, schema-versioned components, typed relations
//! - [`project`]: the graph itself — queries + `StateOp` application
//! - [`delta`]: invertible state operations (undo/redo/diff primitive)
//! - [`actor`]: actors and permissions
//! - [`schema`]: minimal JSON-Schema input validator
//! - [`validation`]: validator trait + builtin checks
//! - [`requirement`]: evaluable requirement expressions
//! - [`known`]: builtin type/component/relation/permission identifiers
//! - [`events`]: structured engine events

pub mod actor;
pub mod delta;
pub mod error;
pub mod events;
pub mod ids;
pub mod known;
pub mod model;
pub mod project;
pub mod requirement;
pub mod schema;
pub mod search;
pub mod validation;

pub use actor::{Actor, ActorKind, Permission, PermissionSet};
pub use delta::StateOp;
pub use error::{Diagnostic, KernelError, Severity};
pub use events::EngineEvent;
pub use ids::*;
pub use model::{Component, Object, ObjectMeta, Relation, TimestampMs, now_ms};
pub use project::{PROJECT_SCHEMA_VERSION, Project};
pub use requirement::RequirementStatus;
pub use search::{SearchQuery, search};
pub use validation::{ValidationReport, Validator, ValidatorRun};
