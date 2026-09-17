//! # worldos-agent
//!
//! Agent runtime: an agent is an Actor that plans against live command
//! schemas, executes real commands inside one transaction, and verifies
//! results. No direct state mutation; no fake success.

pub mod capability;
pub mod planner;
pub mod provider;
pub mod report;
pub mod runtime;

pub use capability::AgentRun;
pub use planner::{FallbackPlanner, LlmPlanner, PlanError, PlannedStep, Planner, RulePlanner};
pub use provider::{EchoProvider, ModelProvider, ProviderConfig, ProviderError};
pub use report::{AgentReport, RunStatus, StepRecord};
pub use runtime::{AgentRuntime, Budget};
