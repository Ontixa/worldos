//! # worldos-artifact
//!
//! Content-addressed artifact storage for WorldOS. Large binary outputs
//! (STEP files, BReps, meshes, renderings) live in a sidecar directory
//! addressed by SHA-256 digest; the project graph stores only
//! [`ArtifactRef`] strings (`sha256:<hex>`), never blobs.
//!
//! Artifacts are outside transaction undo scope: undoing the command
//! that produced an artifact removes the *reference*; the blob is
//! collectable garbage for [`ArtifactStore::gc`].

pub mod digest;
pub mod error;
pub mod store;

pub use digest::ArtifactRef;
pub use error::ArtifactError;
pub use store::{ArtifactStore, GcReport, PutOutcome};
