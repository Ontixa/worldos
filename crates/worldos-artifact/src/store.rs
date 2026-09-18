//! `ArtifactStore`: content-addressed blob storage on the filesystem.
//!
//! Layout mirrors git's object store: `<dir>/objects/<hex[0..2]>/<hex>`.
//! Writes are atomic (tmp file + rename); reads always re-hash and fail
//! on corruption rather than returning wrong bytes.
//!
//! Artifacts live outside transaction undo scope by design — the graph
//! stores digests, and unreferenced blobs are collectable garbage.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::digest::ArtifactRef;
use crate::error::ArtifactError;

/// Result of a [`ArtifactStore::put`].
#[derive(Debug)]
pub struct PutOutcome {
    pub artifact_ref: ArtifactRef,
    pub size: u64,
    /// False when the blob already existed (dedup hit).
    pub written: bool,
}

/// Result of [`ArtifactStore::gc`].
#[derive(Debug, Default)]
pub struct GcReport {
    pub removed: u64,
    pub freed_bytes: u64,
    pub kept: u64,
}

#[derive(Debug)]
pub struct ArtifactStore {
    dir: PathBuf,
}

impl ArtifactStore {
    /// Open (creating if needed) an artifact directory.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, ArtifactError> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(dir.join("objects"))?;
        fs::create_dir_all(dir.join("tmp"))?;
        Ok(Self { dir })
    }

    /// Sidecar directory for a `.worldos` project file:
    /// `foo.worldos` → `foo.worldos.artifacts/`.
    pub fn for_project(project_path: impl AsRef<Path>) -> Result<Self, ArtifactError> {
        let p = project_path.as_ref();
        let mut s = p.as_os_str().to_os_string();
        s.push(".artifacts");
        Self::open(PathBuf::from(s))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Store `bytes` under their content digest. Idempotent; a corrupt
    /// blob already sitting at the address is overwritten (self-heal —
    /// we hold the correct bytes).
    pub fn put(&self, bytes: &[u8]) -> Result<PutOutcome, ArtifactError> {
        let artifact_ref = ArtifactRef::of(bytes);
        let dest = self.object_path(&artifact_ref);
        let mut written = true;
        if dest.exists() && self.verify(&artifact_ref).unwrap_or(false) {
            written = false;
        } else {
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            let tmp =
                self.dir
                    .join("tmp")
                    .join(format!("{}-{}", std::process::id(), artifact_ref.hex()));
            fs::write(&tmp, bytes)?;
            fs::rename(&tmp, &dest).or_else(|_| {
                // Windows rename fails if dest appeared concurrently —
                // identical content is fine either way.
                fs::remove_file(&dest)?;
                fs::rename(&tmp, &dest)
            })?;
        }
        Ok(PutOutcome {
            artifact_ref,
            size: bytes.len() as u64,
            written,
        })
    }

    /// Read a blob, re-hashing to guarantee integrity.
    pub fn get(&self, artifact_ref: &ArtifactRef) -> Result<Vec<u8>, ArtifactError> {
        let path = self.object_path(artifact_ref);
        if !path.exists() {
            return Err(ArtifactError::NotFound(artifact_ref.to_string()));
        }
        let bytes = fs::read(&path)?;
        let actual = ArtifactRef::of(&bytes);
        if actual != *artifact_ref {
            return Err(ArtifactError::Corrupt {
                expected: artifact_ref.to_string(),
                actual: actual.to_string(),
            });
        }
        Ok(bytes)
    }

    pub fn exists(&self, artifact_ref: &ArtifactRef) -> bool {
        self.object_path(artifact_ref).exists()
    }

    /// Re-hash a stored blob and compare — `Ok(true)` means intact.
    pub fn verify(&self, artifact_ref: &ArtifactRef) -> Result<bool, ArtifactError> {
        match self.get(artifact_ref) {
            Ok(_) => Ok(true),
            Err(ArtifactError::Corrupt { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Every artifact currently stored.
    pub fn list(&self) -> Result<Vec<ArtifactRef>, ArtifactError> {
        let mut out = Vec::new();
        let objects = self.dir.join("objects");
        if !objects.exists() {
            return Ok(out);
        }
        for fan in fs::read_dir(&objects)? {
            for entry in fs::read_dir(fan?.path())? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    out.push(name.parse::<ArtifactRef>()?);
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// Delete every blob not in `keep`. Returns what was freed.
    pub fn gc(&self, keep: &HashSet<ArtifactRef>) -> Result<GcReport, ArtifactError> {
        let mut report = GcReport::default();
        for r in self.list()? {
            if keep.contains(&r) {
                report.kept += 1;
            } else {
                let path = self.object_path(&r);
                report.freed_bytes += fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                fs::remove_file(&path)?;
                report.removed += 1;
            }
        }
        Ok(report)
    }

    fn object_path(&self, artifact_ref: &ArtifactRef) -> PathBuf {
        let hex = artifact_ref.hex();
        self.dir.join("objects").join(&hex[..2]).join(hex)
    }
}
