//! Stable, sortable identifiers for all WorldOS entities.
//!
//! IDs are ULIDs: lexicographically sortable by creation time, URL-safe,
//! and independent of filenames or mutable names.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use ulid::Ulid;

macro_rules! define_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Ulid);

        impl $name {
            pub fn new() -> Self {
                Self(Ulid::new())
            }
            pub fn as_str(&self) -> String {
                self.0.to_string()
            }
            pub fn nil() -> Self {
                Self(Ulid::nil())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = ulid::DecodeError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Ulid::from_str(s)?))
            }
        }
    };
}

define_id!(ProjectId, "Unique identifier for a project.");
define_id!(ObjectId, "Unique identifier for an object in the project graph.");
define_id!(RelationId, "Unique identifier for a typed relation edge.");
define_id!(CommandId, "Unique identifier for an executed command.");
define_id!(TransactionId, "Unique identifier for a transaction.");
define_id!(ArtifactId, "Unique identifier for a generated artifact.");
define_id!(EventId, "Unique identifier for a emitted event.");
define_id!(AgentRunId, "Unique identifier for an agent run.");

/// Actor ids are human-meaningful strings (e.g. `local-user`, `agent:genesis`).
/// They are not ULIDs because external tools supply them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActorId(pub String);

impl ActorId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

impl Default for ActorId {
    fn default() -> Self {
        Self::new("local-user")
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ActorId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for ActorId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Namespaced semantic type identifier, e.g. `core:note`, `geom:cube`.
/// Domains own prefixes; `core:*` is reserved for kernel-provided types.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TypeId(pub String);

impl TypeId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn namespace(&self) -> &str {
        self.0.split(':').next().unwrap_or(&self.0)
    }
}

impl fmt::Display for TypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for TypeId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}
