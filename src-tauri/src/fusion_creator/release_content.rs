//! Host-local release bundles. A receipt is committed with the immutable release row
//! and the ReleaseBuilt checkpoint; an orphaned CAS object is not a release.
use super::{digest, encode, io, valid_hash, CreatorRelease, FusionError};
use crate::fusion_source::cas::{verify_open_digest, CasFile, CasObject, CasStore};
use crate::fusion_source::errors::ErrorCode;
use crate::fusion_source::fs_safety;
use crate::fusion_source::schema::{DigestAlgorithm, DigestSpec};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;

const MANIFEST: &str = "manifest.json";
const MAX_MANIFEST: u64 = 8 * 1024 * 1024;
pub(super) const CHECKPOINT_PREFIX: &str = "checkpoint-release-";

pub(super) fn checkpoint_id(release_id: &str) -> String {
    format!("{CHECKPOINT_PREFIX}{}", digest(release_id.as_bytes()))
}

pub(super) fn integrity() -> FusionError {
    FusionError::new(
        ErrorCode::IntegrityViolation,
        "Creator release content does not match its receipt.",
    )
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    pub release_id: String,
    release_blake3: String,
    bundle_blake3: String,
}

/// Audited raw manifest and exact artifact inventory. No absolute paths are
/// persisted or returned to the caller. Artifact reads re-verify the open handle.
#[derive(Clone)]
pub struct ReleaseContent {
    release: CreatorRelease,
    object: CasObject,
    manifest: Vec<u8>,
    receipt_blake3: String,
}

impl ReleaseContent {
    pub fn release(&self) -> &CreatorRelease {
        &self.release
    }
    pub fn manifest(&self) -> &[u8] {
        &self.manifest
    }
    pub fn inventory(&self) -> &[CasFile] {
        &self.object.files
    }
    pub fn receipt_blake3(&self) -> &str {
        &self.receipt_blake3
    }

    pub fn open_artifact(&self, artifact_id: &str) -> Result<File, FusionError> {
        let hash = self
            .release
            .artifact_hashes
            .get(artifact_id)
            .ok_or_else(integrity)?;
        let mut file = fs_safety::open_read(&self.object.root.join(artifact_path(artifact_id)?))?;
        verify_open_digest(
            &mut file,
            &DigestSpec {
                algorithm: DigestAlgorithm::Blake3,
                value: hash.clone(),
            },
        )?;
        file.rewind().map_err(|_| io())?;
        Ok(file)
    }
}

fn artifact_path(id: &str) -> Result<String, FusionError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(integrity());
    }
    let path = format!("artifacts/{id}");
    fs_safety::validate_relative_path(&path)?;
    Ok(path)
}

fn expected(release: &CreatorRelease) -> Result<BTreeMap<String, &str>, FusionError> {
    if release.release_id.is_empty()
        || release.release_id.len() > 256
        || !release.report_ids.is_empty()
        || release.artifact_hashes.len() > 4096
    {
        return Err(integrity());
    }
    let mut result = BTreeMap::from([(MANIFEST.into(), release.manifest_blake3.as_str())]);
    let mut seen = BTreeSet::new();
    for (id, hash) in &release.artifact_hashes {
        let path = artifact_path(id)?;
        if !seen.insert(path.to_ascii_lowercase()) {
            return Err(integrity());
        }
        result.insert(path, hash);
    }
    if result.values().any(|hash| !valid_hash(hash)) {
        return Err(integrity());
    }
    Ok(result)
}

impl Receipt {
    /// Only a trusted handler's owned workspace subdirectory is supplied here.
    /// No archive unpacking, signing, redistribution, or publish-to-Source occurs.
    pub(super) fn capture(
        root: &Path,
        stage: &Path,
        release: &CreatorRelease,
    ) -> Result<Vec<u8>, FusionError> {
        let expected = expected(release)?;
        let _stage = fs_safety::guard(stage)?;
        let files = fs_safety::list_plain_files(stage)?;
        if files.len() != expected.len()
            || files.iter().any(|(name, _)| !expected.contains_key(name))
        {
            return Err(integrity());
        }
        let object = CasStore::open(root)?.import_directory(stage)?;
        manifest(&object, &expected)?;
        encode(&Self {
            release_id: release.release_id.clone(),
            release_blake3: digest(&encode(release)?),
            bundle_blake3: object.digest,
        })
    }

    pub(super) fn audit(
        root: &Path,
        release: CreatorRelease,
        bytes: &[u8],
    ) -> Result<ReleaseContent, FusionError> {
        let receipt: Self = serde_json::from_slice(bytes).map_err(|_| integrity())?;
        if receipt.release_id != release.release_id
            || receipt.release_blake3 != digest(&encode(&release)?)
            || !valid_hash(&receipt.bundle_blake3)
        {
            return Err(integrity());
        }
        let expected = expected(&release)?;
        let object = CasStore::open(root)?.verify(&receipt.bundle_blake3)?;
        let manifest = manifest(&object, &expected)?;
        Ok(ReleaseContent {
            release,
            object,
            manifest,
            receipt_blake3: digest(bytes),
        })
    }
}

fn manifest(object: &CasObject, expected: &BTreeMap<String, &str>) -> Result<Vec<u8>, FusionError> {
    if object.files.len() != expected.len()
        || object.files.iter().any(|file| {
            expected.get(&file.path) != Some(&file.blake3.as_str())
                || (file.path == MANIFEST && !(1..=MAX_MANIFEST).contains(&file.size))
        })
    {
        return Err(integrity());
    }
    let file = fs_safety::open_read(&object.root.join(MANIFEST))?;
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| io())?;
    if bytes.len() as u64 > MAX_MANIFEST || digest(&bytes) != expected[MANIFEST] {
        return Err(integrity());
    }
    Ok(bytes)
}
