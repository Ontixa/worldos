//! Builtin commands shipped with the kernel-adjacent core domains.
//! Domains added later (CAD, BIM, EDA) register their own via plugins.

mod code;
mod document;
mod geometry;
mod meta;
mod object;
mod relation;
mod requirement;

pub use code::*;
pub use document::*;
pub use geometry::*;
pub use meta::*;
pub use object::*;
pub use relation::*;
pub use requirement::*;

use crate::handler::CommandRegistry;
use std::sync::Arc;

/// Register every builtin command into a fresh registry.
pub fn builtin_registry() -> CommandRegistry {
    let mut r = CommandRegistry::new();
    for h in builtin_handlers() {
        r.register(h);
    }
    r
}

pub fn builtin_handlers() -> Vec<Arc<dyn crate::handler::CommandHandler>> {
    vec![
        Arc::new(ObjectCreate),
        Arc::new(ObjectDelete),
        Arc::new(ObjectRename),
        Arc::new(ObjectSetProperty),
        Arc::new(ObjectSetComponent),
        Arc::new(ObjectRemoveComponent),
        Arc::new(ObjectAddTag),
        Arc::new(ObjectRemoveTag),
        Arc::new(RelationAdd),
        Arc::new(RelationRemove),
        Arc::new(DocumentCreate),
        Arc::new(DocumentSetText),
        Arc::new(DocumentAppendText),
        Arc::new(CodeCreateFile),
        Arc::new(CodeSetSource),
        Arc::new(GeometryCreatePrimitive),
        Arc::new(GeometryTransform),
        Arc::new(ProjectRename),
        Arc::new(ProjectSetMeta),
        Arc::new(RequirementCreate),
        Arc::new(RequirementEvaluate),
        Arc::new(DecisionRecord),
    ]
}
