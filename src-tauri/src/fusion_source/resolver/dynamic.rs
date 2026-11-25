use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::manifests::fetch_bounded_bytes;
use crate::fusion_source::adapters::{https_url, ArtifactTransport, Cancellation, TransferLimits};
use crate::fusion_source::cas::validate_hash;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::fs_safety;
use crate::fusion_source::layers::ResolvedRuntimeLock;
use crate::fusion_source::plan::ProviderLockRequirement;
use crate::fusion_source::schema::{
    CriticalFileSpec, DigestAlgorithm, DigestSpec, GithubReleaseProvider, ReleaseChannel,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub id: String,
    pub name: String,
    pub url: String,
    /// Optional server metadata, never a substitute for source-authorized hashes.
    pub api_digest: Option<DigestSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseMetadata {
    pub id: String,
    pub tag: String,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<ReleaseAsset>,
}

#[async_trait]
pub trait GithubReleaseClient: Send + Sync {
    async fn release(
        &self,
        provider: &GithubReleaseProvider,
        cancel: &Cancellation,
    ) -> Result<ReleaseMetadata, FusionError>;
}

/// Expected metadata supplied by a trusted source-policy adapter for this exact
/// release/asset. Merely hashing a newly downloaded unknown binary does not create
/// this attestation. v1 has no signature protocol and does not infer publisher trust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicAssetAttestation {
    pub release_id: String,
    pub tag: String,
    pub asset_id: String,
    pub asset_url: String,
    pub transport_digest: DigestSpec,
    pub critical_files: Vec<CriticalFileSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DynamicRuntimeLock {
    pub repo: String,
    pub release_id: String,
    pub release_tag: String,
    pub asset_id: String,
    pub asset_name: String,
    pub asset_url: String,
    pub transport_digest: DigestSpec,
    pub critical_files: Vec<CriticalFileSpec>,
}

impl DynamicRuntimeLock {
    /// Snapshot conversion preserves the exact URL/tag and all expected hashes.
    /// Rollback consumes this record and cached CAS bytes without calling latest.
    pub fn snapshot_lock(&self) -> ResolvedRuntimeLock {
        ResolvedRuntimeLock {
            release_id: self.release_id.clone(),
            asset_id: self.asset_id.clone(),
            asset_url: Some(self.asset_url.clone()),
            release_tag: Some(self.release_tag.clone()),
            transport_digest: self.transport_digest.clone(),
            critical_files: self.critical_files.clone(),
            tool_versions: BTreeMap::from([
                ("provider".into(), "github-releases-v1".into()),
                ("runtime".into(), self.release_tag.clone()),
            ]),
        }
    }
}

pub struct DynamicProviderResolver {
    client: Arc<dyn GithubReleaseClient>,
}

impl DynamicProviderResolver {
    pub fn new(client: Arc<dyn GithubReleaseClient>) -> Self {
        Self { client }
    }

    pub async fn resolve(
        &self,
        provider: &GithubReleaseProvider,
        requirement: &ProviderLockRequirement,
        expected: Option<&DynamicAssetAttestation>,
        cancel: &Cancellation,
    ) -> Result<DynamicRuntimeLock, FusionError> {
        validate_provider(provider)?;
        cancel.check()?;
        let expected = expected.ok_or_else(|| FusionError::new(
            ErrorCode::ProviderResolutionFailed,
            "Dynamic resolution requires source-authorized expected transport and critical-file hashes.",
        ))?;
        validate_hash(&expected.transport_digest.value)?;
        https_url(&expected.asset_url)?;
        if expected.release_id.is_empty() || expected.tag.is_empty() || expected.asset_id.is_empty()
        {
            return Err(provider_error(
                "The expected release and asset must be pinned.",
            ));
        }
        let mut declared = BTreeMap::new();
        for file in &expected.critical_files {
            fs_safety::validate_relative_path(&file.path)?;
            validate_hash(&file.blake3)?;
            if declared
                .insert(file.path.to_ascii_lowercase(), file.role)
                .is_some()
            {
                return Err(provider_error(
                    "Dynamic critical-file paths must be unique.",
                ));
            }
        }
        if !requirement.release_and_asset
            || !requirement.transport_digest
            || requirement.critical_files.is_empty()
            || requirement
                .critical_files
                .iter()
                .any(|file| declared.get(&file.path.to_ascii_lowercase()) != Some(&file.role))
        {
            return Err(provider_error(
                "The dynamic attestation does not cover the runtime's integrity inventory.",
            ));
        }
        let release = self.client.release(provider, cancel).await?;
        cancel.check()?;
        if release.draft
            || release.id != expected.release_id
            || release.tag != expected.tag
            || provider.tag.as_ref().is_some_and(|tag| tag != &release.tag)
        {
            return Err(provider_error(
                "The discovered release differs from the authorized release.",
            ));
        }
        let mask = glob::Pattern::new(&provider.asset_mask)
            .map_err(|_| provider_error("Invalid release asset mask."))?;
        let assets = release
            .assets
            .iter()
            .filter(|asset| mask.matches(&asset.name))
            .collect::<Vec<_>>();
        if assets.len() != 1 {
            return Err(provider_error(
                "A dynamic provider must select exactly one release asset.",
            ));
        }
        let asset = assets[0];
        https_url(&asset.url)?;
        if asset.id != expected.asset_id || asset.url != expected.asset_url {
            return Err(provider_error(
                "The discovered asset differs from the authorized asset.",
            ));
        }
        if asset.api_digest.as_ref().is_some_and(|digest| {
            digest.algorithm == expected.transport_digest.algorithm
                && !digest
                    .value
                    .eq_ignore_ascii_case(&expected.transport_digest.value)
        }) {
            return Err(provider_error(
                "GitHub's asset digest conflicts with the expected digest.",
            ));
        }
        Ok(DynamicRuntimeLock {
            repo: provider.repo.clone(),
            release_id: release.id,
            release_tag: release.tag,
            asset_id: asset.id.clone(),
            asset_name: asset.name.clone(),
            asset_url: asset.url.clone(),
            transport_digest: DigestSpec {
                algorithm: expected.transport_digest.algorithm,
                value: expected.transport_digest.value.to_ascii_lowercase(),
            },
            critical_files: expected
                .critical_files
                .iter()
                .cloned()
                .map(|mut file| {
                    file.blake3.make_ascii_lowercase();
                    file
                })
                .collect(),
        })
    }
}

pub struct SecureGithubReleaseClient {
    transport: Arc<dyn ArtifactTransport>,
    staging_root: PathBuf,
}

impl SecureGithubReleaseClient {
    pub fn open(
        library_root: &Path,
        transport: Arc<dyn ArtifactTransport>,
    ) -> Result<Self, FusionError> {
        let staging_root = library_root
            .join(".fusion-source")
            .join("provider-metadata");
        let _guard = fs_safety::ensure_directory(&staging_root)?;
        Ok(Self {
            transport,
            staging_root,
        })
    }
}

#[derive(Deserialize)]
struct ApiRelease {
    id: u64,
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    id: u64,
    name: String,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

#[async_trait]
impl GithubReleaseClient for SecureGithubReleaseClient {
    async fn release(
        &self,
        provider: &GithubReleaseProvider,
        cancel: &Cancellation,
    ) -> Result<ReleaseMetadata, FusionError> {
        validate_provider(provider)?;
        let mut url = url::Url::parse("https://api.github.com/").expect("constant URL");
        {
            let mut path = url.path_segments_mut().expect("hierarchical URL");
            path.pop_if_empty().push("repos");
            for part in provider.repo.split('/') {
                path.push(part);
            }
            path.push("releases");
            if let Some(tag) = &provider.tag {
                path.push("tags").push(tag);
            } else if provider.channel == ReleaseChannel::Latest {
                path.push("latest");
            }
        }
        let bytes = fetch_bounded_bytes(
            self.transport.as_ref(),
            url.as_str(),
            &self.staging_root,
            &TransferLimits::json(),
            cancel,
        )
        .await?;
        let release: ApiRelease =
            if provider.tag.is_none() && provider.channel == ReleaseChannel::Nightly {
                let releases: Vec<ApiRelease> = serde_json::from_slice(&bytes)
                    .map_err(|_| provider_error("GitHub returned invalid releases JSON."))?;
                releases
                    .into_iter()
                    .find(|release| release.prerelease && !release.draft)
                    .ok_or_else(|| {
                        provider_error("No published prerelease matches the nightly channel.")
                    })?
            } else {
                serde_json::from_slice(&bytes)
                    .map_err(|_| provider_error("GitHub returned invalid release JSON."))?
            };
        if release.draft
            || provider
                .tag
                .as_ref()
                .is_some_and(|tag| tag != &release.tag_name)
        {
            return Err(provider_error(
                "GitHub returned an unpublished or mismatched release.",
            ));
        }
        let assets = release
            .assets
            .into_iter()
            .map(|asset| {
                https_url(&asset.browser_download_url)?;
                let api_digest = match asset.digest {
                    Some(value) => {
                        let hash = value
                            .strip_prefix("sha256:")
                            .ok_or_else(|| provider_error("Unsupported GitHub asset digest."))?;
                        validate_hash(hash)?;
                        Some(DigestSpec {
                            algorithm: DigestAlgorithm::Sha256,
                            value: hash.to_ascii_lowercase(),
                        })
                    }
                    None => None,
                };
                Ok(ReleaseAsset {
                    id: asset.id.to_string(),
                    name: asset.name,
                    url: asset.browser_download_url,
                    api_digest,
                })
            })
            .collect::<Result<Vec<_>, FusionError>>()?;
        Ok(ReleaseMetadata {
            id: release.id.to_string(),
            tag: release.tag_name,
            draft: release.draft,
            prerelease: release.prerelease,
            assets,
        })
    }
}

fn validate_provider(provider: &GithubReleaseProvider) -> Result<(), FusionError> {
    let parts = provider.repo.split('/').collect::<Vec<_>>();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
        || provider.asset_mask.is_empty()
        || provider.asset_mask.contains(['/', '\\'])
        || provider
            .tag
            .as_ref()
            .is_some_and(|tag| tag.is_empty() || tag.chars().any(char::is_control))
        || (provider.channel == ReleaseChannel::Nightly && provider.tag.is_none())
        || (provider.channel == ReleaseChannel::Latest && provider.tag.is_some())
    {
        return Err(provider_error("Invalid GitHub provider selector."));
    }
    if let Some(tag) = &provider.tag {
        fs_safety::validate_relative_path(tag)?;
    }
    Ok(())
}

fn provider_error(message: &str) -> FusionError {
    FusionError::new(ErrorCode::ProviderResolutionFailed, message)
}
