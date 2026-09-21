//! Non-creating, bounded access to an existing project's sidecar.
use crate::{ArtifactError, ArtifactRef};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Fixed maximum verified copy size; not an unlimited allocation option.
pub const MAX_EXPORT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ArtifactReadError {
    #[error(transparent)]
    Artifact(#[from] ArtifactError),
    #[error("artifact read: {0}")]
    Io(#[from] std::io::Error),
    #[error("artifact must be a regular file, not a symlink or directory")]
    NotRegular,
    #[error("artifact exceeds the fixed {MAX_EXPORT_BYTES}-byte export limit")]
    TooLarge,
}

/// Sidecar association only: no database validation or graph-membership check.
/// Constructors and reads never create directories. Parent paths are trusted;
/// this is not a race-resistant filesystem sandbox.
pub struct ArtifactReader {
    dir: PathBuf,
}

impl ArtifactReader {
    pub fn for_project(project: impl AsRef<Path>) -> Self {
        let mut dir = project.as_ref().as_os_str().to_os_string();
        dir.push(".artifacts");
        Self { dir: dir.into() }
    }

    /// Read at most the fixed limit plus one byte and verify before returning.
    pub fn get(&self, reference: &ArtifactRef) -> Result<Vec<u8>, ArtifactReadError> {
        let path = self
            .dir
            .join("objects")
            .join(&reference.hex()[..2])
            .join(reference.hex());
        if !fs::symlink_metadata(&path)?.file_type().is_file() {
            return Err(ArtifactReadError::NotRegular);
        }
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(ArtifactReadError::NotRegular);
        }
        if metadata.len() > MAX_EXPORT_BYTES {
            return Err(ArtifactReadError::TooLarge);
        }
        let mut bytes = Vec::new();
        file.take(MAX_EXPORT_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_EXPORT_BYTES {
            return Err(ArtifactReadError::TooLarge);
        }
        let actual = ArtifactRef::of(&bytes);
        if actual != *reference {
            return Err(ArtifactError::Corrupt {
                expected: reference.to_string(),
                actual: actual.to_string(),
            }
            .into());
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArtifactStore;

    #[test]
    fn exact_limit_is_verified_and_one_more_byte_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        let store = ArtifactStore::for_project(&project).unwrap();
        let bytes = vec![42; MAX_EXPORT_BYTES as usize];
        let reference = store.put(&bytes).unwrap().artifact_ref;
        drop(bytes);
        let reader = ArtifactReader::for_project(&project);
        assert_eq!(
            reader.get(&reference).unwrap().len() as u64,
            MAX_EXPORT_BYTES
        );
        let path = store
            .dir()
            .join("objects")
            .join(&reference.hex()[..2])
            .join(reference.hex());
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(MAX_EXPORT_BYTES + 1)
            .unwrap();
        assert!(matches!(
            reader.get(&reference),
            Err(ArtifactReadError::TooLarge)
        ));
    }

    #[test]
    fn empty_artifact_is_valid_and_missing_store_is_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        let reference = ArtifactRef::of(b"");
        let reader = ArtifactReader::for_project(&project);
        assert!(reader.get(&reference).is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        ArtifactStore::for_project(project)
            .unwrap()
            .put(b"")
            .unwrap();
        assert!(reader.get(&reference).unwrap().is_empty());
    }
}
