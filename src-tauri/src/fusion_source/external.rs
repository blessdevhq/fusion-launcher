//! Descriptor-owned external acquisition. No installation or process authority.
use super::adapters::https_url;
use super::adapters::{ArtifactTransport, Cancellation, TransferLimits, TransferProgressSink};
use super::cas::CasStore;
use super::creator_schema::ComponentOrigin;
use super::errors::{ErrorCode, FusionError};
use super::fs_safety;
use super::layers::LayerInput;
use super::program_schema::ProgramComponent;
use super::schema::{ArchiveFormat, DigestAlgorithm, DigestSpec, LayerSource};
use super::CanonicalGameManifest;
use crate::fusion_creator::declarative::io::{normalize_zip_with_limits, ZipNormalizationLimits};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Uses the existing bounded HTTPS transport, digest verifier and X normalizer.
/// Only a fully verified normalized object is imported into the library CAS.
pub async fn acquire(
    library: &Path,
    component: &ProgramComponent,
    transport: Arc<dyn ArtifactTransport>,
    limits: &TransferLimits,
    zip_limits: ZipNormalizationLimits,
    cancel: &Cancellation,
    progress: Option<Arc<dyn TransferProgressSink>>,
) -> Result<LayerInput, FusionError> {
    let (url, digest) = external_pins(component)?;
    limits.validate()?;
    cancel.check()?;
    let stage = AcquisitionStage::create(library)?;
    let artifact = stage.root.join("payload.zip");
    let limits = TransferLimits {
        max_bytes: limits
            .max_bytes
            .min(zip_limits.max_compressed_bytes)
            .min(ZipNormalizationLimits::default().max_compressed_bytes),
        timeout: limits.timeout,
    };
    // This adapter joins its writer before returning or being dropped. Do not
    // delegate hashing to a detached delivery worker that cannot own our stage.
    let receipt = transport
        .fetch_with_progress(url, &artifact, &limits, cancel, progress)
        .await?;
    https_url(&receipt.final_url)?;
    cancel.check()?;
    let component = component.clone();
    let library = library.to_owned();
    let cancellation = cancel.clone();
    let abort = Cancellation::default();
    let abort_guard = AbortOnDrop(abort.clone());
    let result = tokio::task::spawn_blocking(move || {
        let _stage = stage;
        cancellation.check()?;
        abort.check()?;
        // Stage ownership travels into this worker BEFORE transport hashing,
        // including when it is queued and its async caller is aborted.
        let _archive_parent = fs_safety::guard(&_stage.root)?;
        let archive_guard = fs_safety::open_read(&artifact)?;
        let metadata = archive_guard
            .metadata()
            .map_err(|e| fs_safety::io_error("Inspect external transfer", &artifact, e))?;
        if metadata.len() != receipt.bytes || metadata.len() > limits.max_bytes {
            return Err(FusionError::new(
                ErrorCode::TransferLimitExceeded,
                "The external transfer returned inconsistent or oversized output.",
            ));
        }
        super::cas::verify_digest(&artifact, &digest).map_err(|mut error| {
            if error.code == ErrorCode::IntegrityViolation {
                error.code = ErrorCode::TransportDigestMismatch;
                error.message = "The downloaded container failed its expected digest.".into();
            }
            error
        })?;
        cancellation.check()?;
        abort.check()?;
        // X's normalizer publishes before checking its expected inventory. Keep
        // that publication private so a bad pin cannot pollute the library CAS.
        let quarantine = CasStore::open(&_stage.root.join("quarantine"))?;
        let object = normalize_zip_with_limits(
            &quarantine,
            &artifact,
            &_stage.root,
            &component,
            zip_limits,
        )?;
        cancellation.check()?;
        abort.check()?;
        let committed = CasStore::open(&library)?.import_directory(&object.root)?;
        Ok(LayerInput::Cas {
            digest: committed.digest,
            runtime_lock: None,
        })
    })
    .await
    .map_err(|_| FusionError::new(ErrorCode::Io, "External normalization worker failed."))?;
    drop(abort_guard);
    cancel.check()?;
    result
}

struct AbortOnDrop(Cancellation);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct AcquisitionStage {
    root: PathBuf,
    _parent: fs_safety::DirectoryGuard,
}
impl AcquisitionStage {
    fn create(library: &Path) -> Result<Self, FusionError> {
        let parent = library.join(".fusion-source/external-staging");
        let guard = fs_safety::ensure_directory(&parent)?;
        let root = parent.join(fs_safety::unique_name("acquire"));
        std::fs::create_dir(&root)
            .map_err(|e| fs_safety::io_error("Reserve external staging", &root, e))?;
        Ok(Self {
            root,
            _parent: guard,
        })
    }
}
impl Drop for AcquisitionStage {
    fn drop(&mut self) {
        let _ = fs_safety::safe_remove_tree(&self.root);
    }
}

fn external_pins(component: &ProgramComponent) -> Result<(&str, DigestSpec), FusionError> {
    // Public Rust values need the same frozen validation as deserialized values.
    let value = serde_json::to_value(component).map_err(|_| invalid())?;
    let _: ProgramComponent = serde_json::from_value(value).map_err(|_| invalid())?;
    let ComponentOrigin::External {
        url,
        transport_blake3,
        ..
    } = &component.origin
    else {
        return Err(invalid());
    };
    https_url(url)?;
    Ok((
        url,
        DigestSpec {
            algorithm: DigestAlgorithm::Blake3,
            value: transport_blake3.clone(),
        },
    ))
}

/// Derived legacy fields are transport plumbing, never a second authority.
/// A user-provided placeholder carries no competing external acquisition data.
pub(crate) fn lower_plan_layers(manifest: &mut CanonicalGameManifest) -> Result<(), FusionError> {
    let Some(program) = crate::fusion_creator::declarative::runtime::descriptor(manifest) else {
        return Ok(());
    };
    let components = program.components.clone();
    for component in &components {
        if !matches!(component.origin, ComponentOrigin::External { .. }) {
            continue;
        }
        let (url, digest) = external_pins(component)?;
        let source = LayerSource::Remote {
            mirrors: vec![url.to_owned()],
            archive: ArchiveFormat::Zip,
        };
        let layer = manifest
            .layers
            .get_mut(&component.layer_id)
            .ok_or_else(invalid)?;
        if !matches!(
            layer.source,
            LayerSource::UserProvided {
                archive: ArchiveFormat::Raw,
                ..
            }
        ) && layer.source != source
            || layer
                .transport_digest
                .as_ref()
                .is_some_and(|value| value != &digest)
        {
            return Err(invalid());
        }
        layer.source = source;
        layer.transport_digest = Some(digest);
    }
    Ok(())
}

fn invalid() -> FusionError {
    FusionError::new(
        ErrorCode::InvalidManifest,
        "External component acquisition data is invalid or contradictory.",
    )
}
