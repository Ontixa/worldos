//! Machine-readable capability descriptors.
//!
//! A capability is a named, schema-described operation the runtime can
//! perform. Multiple providers may implement the same capability id
//! (e.g. `geometry.boolean` via OCCT or a remote service).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Determinism {
    Deterministic,
    NonDeterministic,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    InProcess,
    Subprocess,
    Remote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

impl ProviderInfo {
    pub fn builtin(id: &str) -> Self {
        Self {
            id: id.into(),
            name: "WorldOS builtin".into(),
            version: Some(env!("CARGO_PKG_VERSION").into()),
            kind: Some("builtin".into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    pub id: String,
    pub version: String,
    pub description: String,
    pub provider: ProviderInfo,
    pub input_schema: Value,
    #[serde(default)]
    pub output_schema: Option<Value>,
    /// Permissions the actor must hold (all required).
    pub permissions: Vec<String>,
    pub determinism: Determinism,
    pub execution: ExecutionMode,
    /// Optional cost metadata (units, currency, estimated seconds).
    #[serde(default)]
    pub cost: Option<Value>,
}

impl CapabilityDescriptor {
    pub fn new(id: &str, description: &str, input_schema: Value) -> Self {
        Self {
            id: id.into(),
            version: "1.0.0".into(),
            description: description.into(),
            provider: ProviderInfo::builtin(id),
            input_schema,
            output_schema: None,
            permissions: vec![],
            determinism: Determinism::Unknown,
            execution: ExecutionMode::InProcess,
            cost: None,
        }
    }
    pub fn deterministic(mut self) -> Self {
        self.determinism = Determinism::Deterministic;
        self
    }
    pub fn requires(mut self, perms: &[&str]) -> Self {
        self.permissions = perms.iter().map(|s| s.to_string()).collect();
        self
    }
}
