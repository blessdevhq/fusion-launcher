use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::fusion_source::adapters::{https_url, ArtifactTransport, Cancellation, TransferLimits};
use crate::fusion_source::canonical::{
    CanonicalGameManifest, CanonicalSourceIndex, QualifiedGameId,
};
use crate::fusion_source::cas::validate_hash;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::evolution::SchemaUpcaster;
use crate::fusion_source::fs_safety;
use crate::fusion_source::schema::{GameManifest, SourceGameEntry};
use crate::fusion_source::signatures::{self, SourceIdentityLookup};

#[derive(Debug, Clone)]
pub struct VerifiedSourceIndex {
    pub canonical: CanonicalSourceIndex,
    pub signature_verified: bool,
}

#[derive(Debug, Clone)]
pub struct VerifiedManifest {
    pub raw_blake3: String,
    pub raw_bytes: Vec<u8>,
    pub canonical: CanonicalGameManifest,
    pub cache_reused: bool,
    binding: crate::fusion_source::workspace::VerifiedManifestBinding,
}

impl VerifiedManifest {
    pub fn snapshot_binding(&self) -> crate::fusion_source::workspace::VerifiedManifestBinding {
        self.binding.clone()
    }
}

/// Evidence can only be produced by a fresh index fetch. It is intentionally not
/// serializable: a persisted proof is never launch authorization after restart.
#[derive(Debug, Clone)]
pub struct StrictSyncProof {
    game: QualifiedGameId,
    manifest_blake3: String,
    checked_at: Instant,
}

impl StrictSyncProof {
    pub fn validate(
        &self,
        game: &QualifiedGameId,
        hash: &str,
        max_age: Duration,
    ) -> Result<(), FusionError> {
        if &self.game != game
            || !self.manifest_blake3.eq_ignore_ascii_case(hash)
            || self.checked_at.elapsed() > max_age
        {
            return Err(FusionError::new(
                ErrorCode::StrictSyncRequired,
                "A fresh matching source index check is required.",
            ));
        }
        Ok(())
    }
}

pub struct ManifestFetcher {
    transport: Arc<dyn ArtifactTransport>,
    cache_root: PathBuf,
    limits: TransferLimits,
    identity_lookup: Option<Arc<dyn SourceIdentityLookup>>,
}

impl ManifestFetcher {
    pub fn open(
        library_root: &Path,
        transport: Arc<dyn ArtifactTransport>,
    ) -> Result<Self, FusionError> {
        let cache_root = library_root.join(".fusion-source").join("manifests");
        let _guard = fs_safety::ensure_directory(&cache_root)?;
        Ok(Self {
            transport,
            cache_root,
            limits: TransferLimits::json(),
            identity_lookup: None,
        })
    }

    pub fn with_identity_lookup(mut self, lookup: Arc<dyn SourceIdentityLookup>) -> Self {
        self.identity_lookup = Some(lookup);
        self
    }

    pub fn with_limits(mut self, limits: TransferLimits) -> Result<Self, FusionError> {
        limits.validate()?;
        // JSON metadata stays bounded even if a payload's transfer policy is large.
        if limits.max_bytes > crate::net::JSON_BODY_LIMIT as u64 {
            return Err(FusionError::new(
                ErrorCode::TransferLimitExceeded,
                "Manifest JSON exceeds the metadata budget.",
            ));
        }
        self.limits = limits;
        Ok(self)
    }

    /// Indices are deliberately fetched fresh. Adding a source never acquires a
    /// game's manifest, runtime, firmware, keys or payload.
    pub async fn fetch_index(
        &self,
        url: &str,
        cancel: &Cancellation,
    ) -> Result<CanonicalSourceIndex, FusionError> {
        Ok(self.fetch_verified_index(url, cancel).await?.canonical)
    }

    pub async fn fetch_verified_index(
        &self,
        url: &str,
        cancel: &Cancellation,
    ) -> Result<VerifiedSourceIndex, FusionError> {
        let bytes = fetch_bounded_bytes(
            self.transport.as_ref(),
            url,
            &self.cache_root,
            &self.limits,
            cancel,
        )
        .await?;
        let json = std::str::from_utf8(&bytes).map_err(|_| {
            FusionError::new(
                ErrorCode::InvalidJson,
                "The source index is not UTF-8 JSON.",
            )
        })?;
        let document = signatures::parse_wire_index(json)?;
        let pin = self
            .identity_lookup
            .as_ref()
            .map(|lookup| lookup.pinned_identity(signatures::source_id(&document)?, url))
            .transpose()?
            .flatten();
        let signature_verified = signatures::verify_wire_index(&document, pin.as_ref(), Some(url))?;
        let canonical = SchemaUpcaster::upcast_source_index(json)?;
        Ok(VerifiedSourceIndex {
            canonical,
            signature_verified,
        })
    }

    pub async fn fetch_manifest(
        &self,
        source_id: &str,
        entry: &SourceGameEntry,
        parents: &BTreeMap<QualifiedGameId, GameManifest>,
        cancel: &Cancellation,
    ) -> Result<VerifiedManifest, FusionError> {
        validate_hash(&entry.manifest_blake3)?;
        https_url(&entry.manifest_url)?;
        cancel.check()?;
        let expected = entry.manifest_blake3.to_ascii_lowercase();
        let cached = self.cache_root.join(format!("{expected}.json"));
        let _guard = fs_safety::guard(&self.cache_root)?;
        let mut reused = false;
        let bytes = if fs::symlink_metadata(&cached).is_ok() {
            let bytes = read_bounded(&cached, self.limits.max_bytes)?;
            if blake3::hash(&bytes).to_hex().as_str() == expected {
                reused = true;
                bytes
            } else {
                // A corrupt cache is repairable only by fetching the authoritative
                // expected bytes; never upcast or execute its cached contents.
                fetch_bounded_bytes(
                    self.transport.as_ref(),
                    &entry.manifest_url,
                    &self.cache_root,
                    &self.limits,
                    cancel,
                )
                .await?
            }
        } else {
            fetch_bounded_bytes(
                self.transport.as_ref(),
                &entry.manifest_url,
                &self.cache_root,
                &self.limits,
                cancel,
            )
            .await?
        };
        cancel.check()?;
        if blake3::hash(&bytes).to_hex().as_str() != expected {
            return Err(FusionError::new(
                ErrorCode::ManifestDigestMismatch,
                "The raw manifest bytes do not match the source index.",
            ));
        }
        let json = std::str::from_utf8(&bytes).map_err(|_| {
            FusionError::new(
                ErrorCode::InvalidJson,
                "The game manifest is not UTF-8 JSON.",
            )
        })?;
        let canonical = SchemaUpcaster::upcast_game_manifest(source_id, json, parents)?;
        if canonical.id != entry.id || canonical.version != entry.version {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "The manifest identity/version does not match its index entry.",
            ));
        }
        if !reused {
            fs_safety::atomic_write(&cached, &bytes)?;
        }
        let binding = crate::fusion_source::workspace::VerifiedManifestBinding::from_verified(
            &canonical, &expected,
        )?;
        Ok(VerifiedManifest {
            raw_blake3: expected,
            raw_bytes: bytes,
            canonical,
            cache_reused: reused,
            binding,
        })
    }

    pub async fn confirm_current(
        &self,
        index_url: &str,
        game: &QualifiedGameId,
        manifest_blake3: &str,
        cancel: &Cancellation,
    ) -> Result<StrictSyncProof, FusionError> {
        validate_hash(manifest_blake3)?;
        let index = self.fetch_index(index_url, cancel).await?;
        if index.source_info.id != game.source_id
            || !index.games.iter().any(|entry| {
                entry.id == game.game_id
                    && entry.manifest_blake3.eq_ignore_ascii_case(manifest_blake3)
            })
        {
            return Err(FusionError::new(
                ErrorCode::StrictSyncRequired,
                "The active manifest no longer matches the current source index.",
            ));
        }
        Ok(StrictSyncProof {
            game: game.clone(),
            manifest_blake3: manifest_blake3.to_ascii_lowercase(),
            checked_at: Instant::now(),
        })
    }
}

/// Bind a verifier to the source's registered URL, rather than accepting a
/// manifest-controlled replacement endpoint during launch.
pub struct BoundManifestSyncVerifier {
    pub fetcher: Arc<ManifestFetcher>,
    pub source_id: String,
    pub index_url: String,
    pub cancellation: Cancellation,
}

#[async_trait]
impl crate::fusion_source::integrity::ManifestSyncVerifier for BoundManifestSyncVerifier {
    async fn verify_current(
        &self,
        game: &QualifiedGameId,
        manifest_blake3: &str,
    ) -> Result<(), FusionError> {
        if game.source_id != self.source_id {
            return Err(FusionError::new(
                ErrorCode::StrictSyncRequired,
                "The sync verifier belongs to another source.",
            ));
        }
        self.fetcher
            .confirm_current(&self.index_url, game, manifest_blake3, &self.cancellation)
            .await?;
        Ok(())
    }
}

pub(crate) async fn fetch_bounded_bytes(
    transport: &dyn ArtifactTransport,
    url: &str,
    staging_root: &Path,
    limits: &TransferLimits,
    cancel: &Cancellation,
) -> Result<Vec<u8>, FusionError> {
    https_url(url)?;
    limits.validate()?;
    cancel.check()?;
    let _guard = fs_safety::guard(staging_root)?;
    let mut stage = super::mirrors::StagedFile::new(staging_root)?;
    let result = async {
        let receipt = transport.fetch(url, &stage.path, limits, cancel).await?;
        https_url(&receipt.final_url)?;
        cancel.check()?;
        let bytes = read_bounded(&stage.path, limits.max_bytes)?;
        if receipt.bytes != bytes.len() as u64 {
            return Err(FusionError::new(
                ErrorCode::TransferLimitExceeded,
                "Metadata transport returned inconsistent size.",
            ));
        }
        Ok(bytes)
    }
    .await;
    stage.cleanup()?;
    result
}

fn read_bounded(path: &Path, max_bytes: u64) -> Result<Vec<u8>, FusionError> {
    let file = fs_safety::open_read(path)?;
    if file
        .metadata()
        .map_err(|error| fs_safety::io_error("Inspect metadata", path, error))?
        .len()
        > max_bytes
    {
        return Err(FusionError::new(
            ErrorCode::TransferLimitExceeded,
            "Metadata exceeds its byte limit.",
        ));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| fs_safety::io_error("Read metadata", path, error))?;
    if bytes.len() as u64 > max_bytes {
        return Err(FusionError::new(
            ErrorCode::TransferLimitExceeded,
            "Metadata exceeds its byte limit.",
        ));
    }
    Ok(bytes)
}
