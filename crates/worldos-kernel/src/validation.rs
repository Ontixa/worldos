//! Validation framework: validators produce structured diagnostics.

use crate::error::Diagnostic;
use crate::project::Project;
use serde::{Deserialize, Serialize};

/// A validator inspects the project and reports structured diagnostics.
/// Implemented by builtin checks and later by plugins.
pub trait Validator: Send + Sync {
    fn id(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn validate(&self, project: &Project) -> Vec<Diagnostic>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub validator_runs: Vec<ValidatorRun>,
    pub diagnostics: Vec<Diagnostic>,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorRun {
    pub validator_id: String,
    pub diagnostics: usize,
    pub duration_ms: u64,
}

impl ValidationReport {
    pub fn from_runs(runs: Vec<ValidatorRun>, diagnostics: Vec<Diagnostic>) -> Self {
        let passed = !diagnostics
            .iter()
            .any(|d| d.severity == crate::error::Severity::Error);
        Self {
            validator_runs: runs,
            diagnostics,
            passed,
        }
    }
}

/// Builtin: relations must point at existing objects.
pub struct RelationIntegrity;

impl Validator for RelationIntegrity {
    fn id(&self) -> &'static str {
        "core:relation-integrity"
    }
    fn description(&self) -> &'static str {
        "All relation endpoints must reference existing objects"
    }
    fn validate(&self, project: &Project) -> Vec<Diagnostic> {
        project
            .dangling_relations()
            .into_iter()
            .map(|r| {
                Diagnostic::error(
                    "dangling-relation",
                    format!(
                        "relation {} ({}) references a missing object",
                        r.id, r.type_id
                    ),
                )
                .hint("remove the relation or restore the missing object")
            })
            .collect()
    }
}

/// Builtin: object names should be unique (warning, not error).
pub struct UniqueNames;

impl Validator for UniqueNames {
    fn id(&self) -> &'static str {
        "core:unique-names"
    }
    fn description(&self) -> &'static str {
        "Object names should be unique for reliable referencing"
    }
    fn validate(&self, project: &Project) -> Vec<Diagnostic> {
        let mut seen = std::collections::HashMap::new();
        let mut out = Vec::new();
        for obj in project.objects.values() {
            if let Some(prev) = seen.insert(obj.name.clone(), obj.id) {
                out.push(
                    Diagnostic::warning(
                        "duplicate-name",
                        format!(
                            "name `{}` is shared by objects {} and {}",
                            obj.name, prev, obj.id
                        ),
                    )
                    .at(obj.id),
                );
            }
        }
        out
    }
}
