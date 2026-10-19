//! Prepared layer bindings. Network acquisition and storefront discovery belong
//! to Stage 3; Stage 2 consumes already acquired files or an explicit local root.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::canonical::CanonicalLayer;
use super::cas::{verify_digest, CasObject, CasStore};
use super::errors::{ErrorCode, FusionError};
use super::fs_safety;
use super::schema::{ArchiveFormat, CriticalFileSpec, DigestSpec, LayerKind, LayerSource};
use super::transforms::ArchiveTransformer;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LayerInput {
    Cas {
        digest: String,
        runtime_lock: Option<ResolvedRuntimeLock>,
    },
    /// This is a resolved location supplied by a trusted local adapter, not a
    /// path copied from a downloaded manifest or a persisted absolute CAS path.
    LocalDirectory { root: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedRuntimeLock {
    pub release_id: String,
    pub asset_id: String,
    #[serde(default)]
    pub asset_url: Option<String>,
    #[serde(default)]
    pub release_tag: Option<String>,
    pub transport_digest: DigestSpec,
    pub critical_files: Vec<CriticalFileSpec>,
    pub tool_versions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerDelta {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
    pub reused: Vec<String>,
}

impl LayerDelta {
    pub fn between(old: &BTreeMap<String, LayerInput>, new: &BTreeMap<String, LayerInput>) -> Self {
        let mut delta = Self::default();
        for (id, input) in new {
            match old.get(id) {
                None => delta.added.push(id.clone()),
                Some(previous) if previous != input => delta.changed.push(id.clone()),
                Some(_) => delta.reused.push(id.clone()),
            }
        }
        delta
            .removed
            .extend(old.keys().filter(|id| !new.contains_key(*id)).cloned());
        delta
    }
}

#[derive(Debug, Clone)]
pub struct LayerCache {
    cas: CasStore,
}

impl LayerCache {
    pub fn open(library_root: &Path) -> Result<Self, FusionError> {
        Ok(Self {
            cas: CasStore::open(library_root)?,
        })
    }

    pub fn cas(&self) -> &CasStore {
        &self.cas
    }

    /// Verifies the container while its descriptor denies mutation, extracts to
    /// private staging and publishes an independently hashed immutable CAS tree.
    /// A raw import's relative filename is explicit, never inferred from a URL.
    pub fn prepare_artifact(
        &self,
        layer: &CanonicalLayer,
        artifact: &Path,
        raw_relative: Option<&str>,
        runtime_lock: Option<ResolvedRuntimeLock>,
    ) -> Result<LayerInput, FusionError> {
        let _source_parent = fs_safety::guard(artifact.parent().ok_or_else(|| {
            FusionError::new(
                ErrorCode::InvalidPath,
                "An artifact requires an absolute parent.",
            )
        })?)?;
        let _source = fs_safety::open_read(artifact)?;
        if runtime_lock.is_some()
            && (layer.kind != LayerKind::Runtime || layer.dynamic_provider.is_none())
        {
            return Err(FusionError::new(ErrorCode::InvalidRuntimeChannel,
                "Only a runtime with a dynamic provider may substitute a resolved transport digest."));
        }
        let expected = runtime_lock
            .as_ref()
            .map(|lock| &lock.transport_digest)
            .or(layer.transport_digest.as_ref());
        if matches!(layer.source, LayerSource::Remote { .. }) && expected.is_none() {
            return Err(FusionError::new(
                ErrorCode::InvalidDigest,
                "A remote artifact requires its expected transport digest.",
            ));
        }
        if let Some(expected) = expected {
            verify_digest(artifact, expected)?;
        }
        let format = match &layer.source {
            LayerSource::Remote { archive, .. } | LayerSource::UserProvided { archive, .. } => {
                *archive
            }
            LayerSource::LocalLocator { .. } => {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "A local locator requires a resolved directory, not an archive.",
                ))
            }
        };
        let object = if format == ArchiveFormat::Raw {
            let name = if layer.kind == LayerKind::Patch {
                "patch.payload"
            } else {
                raw_relative.ok_or_else(|| {
                    FusionError::new(
                        ErrorCode::InvalidPath,
                        "A raw artifact requires an explicit layer-relative filename.",
                    )
                })?
            };
            self.cas.import_file(artifact, name)?
        } else {
            let parent = self.cas.root().join("staging");
            let _parent = fs_safety::guard(&parent)?;
            let stage = parent.join(fs_safety::unique_name("extract"));
            fs::create_dir(&stage)
                .map_err(|error| fs_safety::io_error("Create extraction staging", &stage, error))?;
            let result = (|| {
                ArchiveTransformer::default().extract(artifact, format, &stage)?;
                self.cas.import_directory(&stage)
            })();
            let cleanup = fs_safety::safe_remove_tree(&stage);
            match (result, cleanup) {
                (Ok(object), Ok(())) => object,
                (Err(error), _) | (Ok(_), Err(error)) => return Err(error),
            }
        };
        Ok(LayerInput::Cas {
            digest: object.digest,
            runtime_lock,
        })
    }

    pub fn prepare_directory(&self, source: &Path) -> Result<LayerInput, FusionError> {
        let object = self.cas.import_directory(source)?;
        Ok(LayerInput::Cas {
            digest: object.digest,
            runtime_lock: None,
        })
    }

    /// Ordinary cache access validates metadata and immutable structure without
    /// hashing an entire large layer. Use `cas().verify` for an explicit byte audit.
    pub fn object(&self, input: &LayerInput) -> Result<CasObject, FusionError> {
        match input {
            LayerInput::Cas { digest, .. } => self.cas.get(digest),
            LayerInput::LocalDirectory { .. } => Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "An external local base is not a managed CAS object.",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fusion_source::schema::{DigestAlgorithm, LayerSpec};

    #[test]
    fn delta_reuses_unchanged_layers_and_reports_selection_changes() {
        let cas = |digest: &str| LayerInput::Cas {
            digest: digest.into(),
            runtime_lock: None,
        };
        let old = BTreeMap::from([
            ("base".into(), cas("a")),
            ("runtime".into(), cas("b")),
            ("old-mod".into(), cas("c")),
        ]);
        let new = BTreeMap::from([
            ("base".into(), cas("a")),
            ("runtime".into(), cas("d")),
            ("new-mod".into(), cas("e")),
        ]);
        let delta = LayerDelta::between(&old, &new);
        assert_eq!(delta.reused, ["base"]);
        assert_eq!(delta.changed, ["runtime"]);
        assert_eq!(delta.added, ["new-mod"]);
        assert_eq!(delta.removed, ["old-mod"]);
    }

    #[test]
    fn raw_artifact_verifies_transport_and_has_a_separate_tree_digest() {
        let temp = tempfile::tempdir().unwrap();
        let artifact = temp.path().join("runtime.exe");
        fs::write(&artifact, b"abc").unwrap();
        let spec: LayerSpec = serde_json::from_value(serde_json::json!({
            "id":"runtime","kind":"runtime",
            "source":{"kind":"remote","mirrors":["https://example.org/runtime.exe"],"archive":"raw"},
            "transport_digest":{"algorithm":"sha256","value":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}
        })).unwrap();
        let layer = CanonicalLayer::from(spec);
        let cache = LayerCache::open(&temp.path().join("library")).unwrap();
        let input = cache
            .prepare_artifact(&layer, &artifact, Some("runtime.exe"), None)
            .unwrap();
        let object = cache.object(&input).unwrap();
        assert_eq!(fs::read(object.root.join("runtime.exe")).unwrap(), b"abc");
        assert_ne!(object.digest, layer.transport_digest.unwrap().value);
        fs::write(&artifact, b"changed").unwrap();
        let mut invalid = layer_from_json();
        invalid.transport_digest = Some(DigestSpec {
            algorithm: DigestAlgorithm::Blake3,
            value: "0".repeat(64),
        });
        assert_eq!(
            cache
                .prepare_artifact(&invalid, &artifact, Some("runtime.exe"), None)
                .unwrap_err()
                .code,
            ErrorCode::IntegrityViolation
        );
    }

    fn layer_from_json() -> CanonicalLayer {
        serde_json::from_value::<LayerSpec>(serde_json::json!({
            "id":"runtime","kind":"runtime","source":{"kind":"user_provided"}
        }))
        .unwrap()
        .into()
    }
}
