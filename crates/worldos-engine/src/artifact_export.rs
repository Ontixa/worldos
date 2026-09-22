//! Trusted local-operator file copying, separate from project transactions.
use crate::Engine;
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use worldos_artifact::{ArtifactError, ArtifactReadError, ArtifactReader, ArtifactRef};
use worldos_kernel::actor::{Actor, ActorKind, Permission};
use worldos_kernel::known::permissions::{ARTIFACT_EXPORT, FILESYSTEM_WRITE};

#[derive(Debug, Serialize)]
pub struct ArtifactExportReport {
    pub reference: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub verified: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactExportError {
    #[error("artifact file export requires a trusted local human operator")]
    OperatorRequired,
    #[error("artifact file export requires permission {0}")]
    PermissionDenied(&'static str),
    #[error("project path must name an existing regular file: {0}")]
    Project(String),
    #[error(transparent)]
    Reference(#[from] ArtifactError),
    #[error(transparent)]
    Read(#[from] ArtifactReadError),
    #[error(
        "cannot create new destination {path:?}: {source}; existing paths are never overwritten"
    )]
    Create {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "artifact {stage} failed for {path:?}: {source}; newly created file retained and may be partial; inspect and remove it manually before retrying"
    )]
    Partial {
        path: PathBuf,
        stage: &'static str,
        #[source]
        source: io::Error,
    },
}

impl Engine {
    /// Copy a verified sidecar artifact to a new file without opening SQLite or CAD.
    /// Actor identity is caller-asserted, not authenticated. This operator-only API
    /// is not registered with RPC/agents. It checks permissions before filesystem
    /// access but does not provide containment, graph authorization, or undo.
    /// Parents are trusted. Failed writes retain the created path; crash atomicity
    /// and concurrent path replacement protection are not guaranteed.
    pub fn export_project_artifact(
        project: &Path,
        actor: &Actor,
        reference: &str,
        destination: &Path,
    ) -> Result<ArtifactExportReport, ArtifactExportError> {
        if actor.kind != ActorKind::Human {
            return Err(ArtifactExportError::OperatorRequired);
        }
        for permission in [ARTIFACT_EXPORT, FILESYSTEM_WRITE] {
            if !actor.permissions.is_allowed(&Permission::from(permission)) {
                return Err(ArtifactExportError::PermissionDenied(permission));
            }
        }
        let reference: ArtifactRef = reference.parse()?;
        let metadata = fs::symlink_metadata(project)
            .map_err(|e| ArtifactExportError::Project(e.to_string()))?;
        if !metadata.file_type().is_file() {
            return Err(ArtifactExportError::Project("not a regular file".into()));
        }
        let bytes = ArtifactReader::for_project(project).get(&reference)?;
        write_new_with(destination, &bytes, |file, bytes| {
            file.write_all(bytes).map_err(|e| ("write", e))?;
            file.sync_all().map_err(|e| ("sync", e))
        })?;
        Ok(ArtifactExportReport {
            reference: reference.to_string(),
            path: destination.into(),
            bytes: bytes.len() as u64,
            verified: true,
        })
    }
}

fn write_new_with(
    destination: &Path,
    bytes: &[u8],
    write: impl FnOnce(&mut File, &[u8]) -> Result<(), (&'static str, io::Error)>,
) -> Result<(), ArtifactExportError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|source| ArtifactExportError::Create {
            path: destination.into(),
            source,
        })?;
    write(&mut file, bytes).map_err(|(stage, source)| ArtifactExportError::Partial {
        path: destination.into(),
        stage,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_write_and_sync_retain_created_output() {
        let dir = tempfile::tempdir().unwrap();
        for stage in ["write", "sync"] {
            let path = dir.path().join(stage);
            let error = write_new_with(&path, b"verified", |file, bytes| {
                file.write_all(if stage == "write" { &bytes[..3] } else { bytes })
                    .unwrap();
                Err((stage, io::Error::other("injected failure")))
            })
            .unwrap_err();
            assert!(
                matches!(error, ArtifactExportError::Partial { stage: actual, .. } if actual == stage)
            );
            assert_eq!(
                fs::read(&path).unwrap(),
                if stage == "write" {
                    b"ver".as_slice()
                } else {
                    b"verified".as_slice()
                }
            );
            assert!(error.to_string().contains("retained"));
        }
    }
}
