//! Well-known type, component and relation identifiers.
//!
//! The kernel treats these as data — domains may register more via plugins.
//! `core:*` and `geom:*`/`doc:*`/`code:*` are the builtin namespaces.

pub mod types {
    pub const NOTE: &str = "core:note";
    pub const FOLDER: &str = "core:folder";
    pub const REQUIREMENT: &str = "core:requirement";
    pub const DECISION: &str = "core:decision";
    pub const CODE_FILE: &str = "code:file";
    pub const AGENT_TASK: &str = "core:agent-task";
    pub const AGENT_RUN: &str = "core:agent-run";
    pub const ARTIFACT: &str = "core:artifact";
    pub const CUBE: &str = "geom:cube";
    pub const SPHERE: &str = "geom:sphere";
    pub const CYLINDER: &str = "geom:cylinder";
    pub const PLANE: &str = "geom:plane";

    /// True for builtin spatial primitive types.
    pub fn is_primitive(t: &str) -> bool {
        matches!(t, CUBE | SPHERE | CYLINDER | PLANE)
    }
}

pub mod components {
    pub const TRANSFORM: &str = "core:transform";
    pub const GEOMETRY: &str = "geom:geometry";
    pub const TEXT: &str = "doc:text";
    pub const SOURCE: &str = "code:source";
    pub const MATERIAL: &str = "geom:material";
    pub const REQUIREMENT_EXPR: &str = "core:requirement-expr";
    pub const REQUIREMENT_STATUS: &str = "core:requirement-status";
    pub const DECISION_INFO: &str = "core:decision-info";
    pub const AGENT_TASK_INFO: &str = "core:agent-task-info";
    pub const COST: &str = "core:cost";
}

pub mod rel {
    pub const CONTAINS: &str = "core:contains";
    pub const REFERENCES: &str = "core:references";
    pub const DEPENDS_ON: &str = "core:depends-on";
    pub const SATISFIES: &str = "core:satisfies";
    pub const DERIVED_FROM: &str = "core:derived-from";
    pub const CREATED_BY: &str = "core:created-by";
}

pub mod permissions {
    pub const PROJECT_READ: &str = "project.read";
    pub const PROJECT_WRITE: &str = "project.write";
    pub const PROJECT_SEARCH: &str = "project.search";
    pub const COMMAND_EXECUTE: &str = "command.execute";
    pub const CAPABILITY_EXECUTE: &str = "capability.execute";
    pub const VALIDATION_RUN: &str = "validation.run";
    pub const ARTIFACT_EXPORT: &str = "artifact.export";
    pub const FILESYSTEM_READ: &str = "filesystem.read";
    pub const FILESYSTEM_WRITE: &str = "filesystem.write";
    pub const NETWORK_ACCESS: &str = "network.access";
    pub const SHELL_EXECUTE: &str = "shell.execute";
}
