//! Kernel error types. Typed errors only — no silent failures.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum KernelError {
    #[error("object not found: {0}")]
    ObjectNotFound(String),
    #[error("relation not found: {0}")]
    RelationNotFound(String),
    #[error("duplicate object name: {0}")]
    DuplicateName(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("schema validation failed: {0}")]
    SchemaValidation(String),
    #[error("permission denied: actor lacks `{0}`")]
    PermissionDenied(String),
    #[error("relation endpoint does not exist: {0}")]
    DanglingRelation(String),
    #[error("type not registered: {0}")]
    UnknownType(String),
}

/// A structured diagnostic produced by validators.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_id: Option<crate::ids::ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Diagnostic {
    pub fn error(code: &str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code: code.into(),
            message: message.into(),
            object_id: None,
            hint: None,
        }
    }
    pub fn warning(code: &str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code: code.into(),
            message: message.into(),
            object_id: None,
            hint: None,
        }
    }
    pub fn at(mut self, id: crate::ids::ObjectId) -> Self {
        self.object_id = Some(id);
        self
    }
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}
