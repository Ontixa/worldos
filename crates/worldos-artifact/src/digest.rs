//! Content digests and artifact references.
//!
//! An [`ArtifactRef`] is the canonical way the project graph points at a
//! blob: the string form `sha256:<64-hex>` is what components store.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::error::ArtifactError;

/// SHA-256 digest of a stored blob, serialized as `sha256:<hex>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ArtifactRef(String);

impl ArtifactRef {
    /// Digest of `bytes` — the address the blob is stored under.
    pub fn of(bytes: &[u8]) -> Self {
        let mut h = Sha256::new();
        h.update(bytes);
        Self(hex::encode(h.finalize()))
    }

    /// Lowercase hex digest (64 chars), without the `sha256:` prefix.
    pub fn hex(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ArtifactRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", self.0)
    }
}

impl FromStr for ArtifactRef {
    type Err = ArtifactError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hexpart = s.strip_prefix("sha256:").unwrap_or(s);
        let valid = hexpart.len() == 64
            && hexpart
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
        if valid {
            Ok(Self(hexpart.to_string()))
        } else {
            Err(ArtifactError::InvalidRef(s.to_string()))
        }
    }
}

impl TryFrom<String> for ArtifactRef {
    type Error = ArtifactError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<ArtifactRef> for String {
    fn from(r: ArtifactRef) -> String {
        r.to_string()
    }
}
