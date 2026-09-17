//! # worldos-capability
//!
//! The capability registry and provider abstraction. Capabilities are
//! machine-described operations (inspect, search, validate, measure,
//! export…) that may be implemented by multiple providers. Mutating
//! capabilities route through `CapabilityHost::run_command`, so every
//! effect remains a transactional, undoable command.

pub mod builtin;
pub mod descriptor;
pub mod error;
pub mod host;
pub mod registry;

pub use descriptor::{CapabilityDescriptor, Determinism, ExecutionMode, ProviderInfo};
pub use error::CapabilityError;
pub use host::CapabilityHost;
pub use registry::{Capability, CapabilityRegistry};
