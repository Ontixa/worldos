//! Simple structured project search (v0.1): name, type, tag, component.
//! Later extensions (full-text, semantic, spatial) plug in behind the
//! same query shape.

use crate::model::Object;
use crate::project::Project;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub type_id: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub has_component: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

pub fn search<'a>(project: &'a Project, q: &SearchQuery) -> Vec<&'a Object> {
    let text = q.text.as_deref().map(|t| t.to_lowercase());
    let mut out: Vec<&Object> = project
        .objects
        .values()
        .filter(|o| {
            if let Some(t) = &text {
                let in_name = o.name.to_lowercase().contains(t);
                let in_tags = o.tags.iter().any(|tag| tag.to_lowercase().contains(t));
                let in_type = o.type_id.0.to_lowercase().contains(t);
                if !(in_name || in_tags || in_type) {
                    return false;
                }
            }
            if let Some(ty) = &q.type_id
                && o.type_id.0 != *ty
            {
                return false;
            }
            if let Some(tag) = &q.tag
                && !o.tags.iter().any(|t| t == tag)
            {
                return false;
            }
            if let Some(c) = &q.has_component
                && !o.components.contains_key(c)
            {
                return false;
            }
            true
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    if let Some(l) = q.limit {
        out.truncate(l);
    }
    out
}
