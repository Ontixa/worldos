//! Project snapshot: the complete serializable state of a project,
//! including history. Used for save/load, export and semantic diff.

use serde::{Deserialize, Serialize};
use worldos_commands::History;
use worldos_kernel::project::Project;

/// File-format version written into `meta.format_version` / snapshot JSON.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub format_version: u32,
    pub project: Project,
    #[serde(default)]
    pub history: History,
}

impl Snapshot {
    pub fn new(project: Project, history: History) -> Self {
        Self { format_version: FORMAT_VERSION, project, history }
    }
}
