//! Actors: who performs actions on the project.
//!
//! Humans, agents, plugins, scripts, services and devices are all actors.
//! Every mutation is attributable to an actor via command envelopes.

use crate::ids::ActorId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Human,
    Agent,
    Plugin,
    Script,
    Service,
    Device,
}

/// A permission string such as `project.write` or `filesystem.read`.
/// `*` grants everything; `project.*` grants a namespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Permission(pub String);

impl Permission {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    /// True when `self` (a grant pattern) covers `required`.
    pub fn grants(&self, required: &Permission) -> bool {
        let grant = self.0.as_str();
        let req = required.0.as_str();
        if grant == "*" || grant == req {
            return true;
        }
        if let Some(ns) = grant.strip_suffix(".*") {
            return req == ns || req.starts_with(&format!("{ns}."));
        }
        false
    }
}

impl From<&str> for Permission {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionSet {
    pub grants: BTreeSet<Permission>,
}

impl PermissionSet {
    pub fn all() -> Self {
        Self {
            grants: BTreeSet::from([Permission::new("*")]),
        }
    }
    pub fn read_only() -> Self {
        Self {
            grants: BTreeSet::from([
                Permission::new("project.read"),
                Permission::new("project.search"),
            ]),
        }
    }
    /// Sensible local default for agents: full project access, no shell/network.
    pub fn agent_default() -> Self {
        Self {
            grants: BTreeSet::from([
                Permission::new("project.*"),
                Permission::new("command.execute"),
                Permission::new("capability.execute"),
                Permission::new("validation.run"),
                Permission::new("artifact.export"),
            ]),
        }
    }
    pub fn is_allowed(&self, required: &Permission) -> bool {
        self.grants.iter().any(|g| g.grants(required))
    }
    pub fn grant(&mut self, p: Permission) {
        self.grants.insert(p);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Actor {
    pub id: ActorId,
    pub kind: ActorKind,
    pub name: String,
    pub permissions: PermissionSet,
}

impl Actor {
    pub fn human(name: impl Into<String>) -> Self {
        Self {
            id: ActorId::new("local-user"),
            kind: ActorKind::Human,
            name: name.into(),
            permissions: PermissionSet::all(),
        }
    }
    pub fn agent(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            id: ActorId::new(format!("agent:{name}")),
            kind: ActorKind::Agent,
            name,
            permissions: PermissionSet::agent_default(),
        }
    }
    /// Hosted plugin: same project access as an agent, tagged `plugin:`.
    /// Tighter grants come from the plugin manifest when signed
    /// permissions land; for now plugins are trusted local code.
    pub fn plugin(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            id: ActorId::new(format!("plugin:{name}")),
            kind: ActorKind::Plugin,
            name,
            permissions: PermissionSet::agent_default(),
        }
    }
}
