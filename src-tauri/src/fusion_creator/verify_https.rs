//! Production HTTPS acquisition verification with an empty cache and byte-hash pinning.
use super::build::{insert_capability_hashes, supported_component, supported_controls};
use super::jobs::{JobContext, JobHandler, ReleaseContent, ResumePoint};
use super::verify;
use super::{
    CreatorCheckpoint, CreatorCheckpointBoundary, CreatorJobStartRequest, CreatorReport,
    CreatorReportKind, CreatorReportStatus, SavedPrivateBindings,
};
use crate::fusion_source::{
    adapters::{https_url, SecureHttpsTransport},
    canonical::QualifiedGameId,
    cas::CasStore,
    errors::{ErrorCode, FusionError},
    fs_safety,
    packer::PackerEngine,
    program_schema::CreatorProgramProject,
    service::FusionStorage,
};
use crate::storage::{fusion_source::SqliteFusionStorage, RepositoryStore};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

pub(super) struct ManagedReleaseVerifyHttps;

#[derive(Deserialize)]
struct InventoryRow {
    layer_id: String,
    cas_digest: String,
}

#[derive(Deserialize)]
struct InventoryDoc {
    artifacts: Vec<InventoryRow>,
}

fn unavailable(message: &'static str) -> FusionError {
    FusionError::new(ErrorCode::ActionUnavailable, message)
        .with_detail("command", "fusion_creator_verify_https")
}

fn integrity() -> FusionError {
    FusionError::new(
        ErrorCode::IntegrityViolation,
        "HTTPS source bytes do not match the pinned signed release.",
    )
}

fn redact_https_error(error: FusionError, stage: &'static str) -> FusionError {
    if error.details.get("reason") == Some(&serde_json::json!("environment_approval_required")) {
        return error;
    }
    FusionError::new(error.code, error.message).with_detail("stage", stage)
}

fn implementation_digest() -> &'static str {
    static DIGEST: OnceLock<String> = OnceLock::new();
    DIGEST.get_or_init(|| {
        let mut hash = blake3::Hasher::new();
        hash.update(verify::implementation_digest().as_bytes());
        hash.update(&blake3::hash(include_bytes!("verify_https.rs")).as_bytes()[..]);
        hash.finalize().to_hex().to_string()
    })
}

impl JobHandler for ManagedReleaseVerifyHttps {
    fn validate_request(&self, request: &CreatorJobStartRequest) -> Result<(), FusionError> {
        let Some(url) = request.source_url.as_deref().filter(|s| !s.is_empty()) else {
            return Err(unavailable(
                "HTTPS verification requires a saved release, one compatible profile pair and an HTTPS source URL.",
            ));
        };
        if url.ends_with('/') {
            return Err(FusionError::new(
                ErrorCode::InvalidUri,
                "HTTPS source URL must point to source-index.json without a trailing slash.",
            ));
        }
        https_url(url)?;
        if request.release_id.as_ref().is_none_or(String::is_empty)
            || request.target_directory.is_some()
            || request.build_profile_ids.len() != 1
            || request.controls_profile_ids.len() != 1
        {
            return Err(unavailable(
                "HTTPS verification requires a saved release, one compatible profile pair and an HTTPS source URL.",
            ));
        }
        Ok(())
    }

    fn deferred_input_audit(&self) -> bool {
        true
    }

    fn input_hashes(
        &self,
        _: &CreatorProgramProject,
        _: Option<&SavedPrivateBindings>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        Err(unavailable("HTTPS verification requires a saved release."))
    }

    fn input_hashes_for_release(
        &self,
        project: &CreatorProgramProject,
        _: Option<&SavedPrivateBindings>,
        release: Option<&ReleaseContent>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        let release =
            release.ok_or_else(|| unavailable("HTTPS verification requires a saved release."))?;
        if project
            .program_descriptor
            .components
            .iter()
            .any(|component| !supported_component(component))
            || project
                .program_descriptor
                .controls_profiles
                .iter()
                .any(|profile| !supported_controls(profile))
        {
            return Err(unavailable(
                "HTTPS verification supports only redistributable managed components with supported controls writers.",
            ));
        }
        let mut inputs = BTreeMap::from([(
            "executor_implementation".into(),
            implementation_digest().into(),
        )]);
        insert_capability_hashes(&project.program_descriptor, &mut inputs)?;
        for component in &project.program_descriptor.components {
            inputs.insert(
                format!("component:{}", component.id),
                release.receipt_blake3().to_owned(),
            );
        }
        Ok(inputs)
    }

    fn validate_checkpoint(
        &self,
        checkpoint: &CreatorCheckpoint,
        evidence: &[u8],
    ) -> Result<bool, FusionError> {
        if checkpoint.boundary != CreatorCheckpointBoundary::VerificationCompleted {
            return Ok(false);
        }
        Ok(
            serde_json::from_slice::<CreatorReport>(evidence).is_ok_and(|report| {
                report.kind == CreatorReportKind::HttpsAcquisition
                    && report.status == CreatorReportStatus::Passed
            }),
        )
    }

    fn run(&self, context: JobContext, resume: Option<ResumePoint>) -> Result<(), FusionError> {
        self.validate_request(context.request())?;
        if resume.is_some() {
            return Err(unavailable(
                "HTTPS verification restarts cleanly with an empty cache.",
            ));
        }
        let project = context.project();
        let release = context
            .selected_release()
            .ok_or_else(|| unavailable("HTTPS verification requires a saved release."))?;
        let build = &context.request().build_profile_ids[0];
        let controls = &context.request().controls_profile_ids[0];
        let source_url = context
            .request()
            .source_url
            .clone()
            .ok_or_else(|| unavailable("Missing HTTPS source URL."))?;

        verify::reject_unapproved_env_paths(project, "https_acquisition")?;

        context.progress("checking_release", 0, Some(2))?;
        let scratch = verify::Scratch::new_prefixed(context.workspace(), "verify-https")?;
        let pinned_dir = scratch.0.join("pinned");
        let materialized = context
            .with_work_checks(|check| verify::materialize(release, project, &pinned_dir, check))?;
        if !materialized.portable {
            return Err(FusionError::new(
                ErrorCode::InvalidUri,
                "This release embeds local offline.fusion.invalid URLs and cannot be verified over HTTPS. Build a new release with Source-relative URLs.",
            )
            .with_detail("reason", "legacy_offline_release_urls"));
        }
        let audit = context.with_work_checks(|check| {
            PackerEngine::new().verify_audit_checked(&pinned_dir, check)
        })?;
        if !audit.report.signature_verified
            || audit.report.source_id != project.source_id
            || audit.report.manifests_verified != 1
            || audit.report.artifacts_verified != project.program_descriptor.components.len()
            || audit.report.attestations_verified != 0
        {
            return Err(integrity());
        }
        let inventory_bytes = std::fs::read(pinned_dir.join(&materialized.inventory_relative))
            .map_err(|_| super::jobs::io())?;
        let inventory: InventoryDoc =
            serde_json::from_slice(&inventory_bytes).map_err(|_| integrity())?;
        let expected_cas: BTreeMap<String, String> = inventory
            .artifacts
            .into_iter()
            .map(|item| (item.layer_id, item.cas_digest))
            .collect();
        // Remove local publication bytes before touching HTTPS so acquisition uses an empty cache.
        fs_safety::safe_remove_tree(&pinned_dir)?;

        context.progress("https_acquisition", 1, Some(2))?;
        let run_https = async {
            let (library, db_path, _) = verify::execute_isolated_install(
                &context,
                &scratch.0,
                Arc::new(SecureHttpsTransport),
                &source_url,
                release,
                build,
                controls,
                Some(&materialized.public_key),
                "https_acquisition",
            )
            .await?;
            let game = QualifiedGameId {
                source_id: project.source_id.clone(),
                game_id: project.game_id.clone(),
            };
            let expected_manifest = release.manifest().to_vec();
            let expected_manifest_blake3 = release.release().manifest_blake3.clone();
            let cas_digests = expected_cas.clone();
            tokio::task::spawn_blocking(move || {
                verify_downloaded_bytes(
                    &library,
                    &db_path,
                    &game,
                    &expected_manifest_blake3,
                    &expected_manifest,
                    &cas_digests,
                )
            })
            .await
            .map_err(|_| super::jobs::io())??;
            Ok::<(), FusionError>(())
        };

        if let Err(error) = tauri::async_runtime::block_on(run_https) {
            if matches!(
                error.code,
                ErrorCode::Cancelled | ErrorCode::SnapshotConflict
            ) {
                return Err(error);
            }
            let reported = redact_https_error(error, "https_acquisition");
            let mut report = verify::build_report(
                &context,
                release,
                CreatorReportKind::HttpsAcquisition,
                CreatorReportStatus::Failed,
                build,
                controls,
                vec![reported.clone()],
            );
            report
                .environment
                .insert("mode".into(), "isolated_https_library".into());
            report
                .environment
                .insert("source_url".into(), source_url.clone());
            context.fail_with_report(report, reported)?;
            return Ok(());
        }

        let mut report = verify::build_report(
            &context,
            release,
            CreatorReportKind::HttpsAcquisition,
            CreatorReportStatus::Passed,
            build,
            controls,
            vec![],
        );
        report
            .environment
            .insert("mode".into(), "isolated_https_library".into());
        report.environment.insert("source_url".into(), source_url);
        let evidence = serde_json::to_vec(&report).map_err(|_| integrity())?;
        context.checkpoint_verified(super::jobs::CheckpointOutput {
            boundary: CreatorCheckpointBoundary::VerificationCompleted,
            evidence,
            artifact_id: None,
            release: None,
            report: Some(report),
        })?;
        Ok(())
    }
}

fn verify_downloaded_bytes(
    library: &Path,
    db_path: &Path,
    game: &QualifiedGameId,
    expected_manifest_blake3: &str,
    expected_manifest: &[u8],
    expected_cas: &BTreeMap<String, String>,
) -> Result<(), FusionError> {
    let store = RepositoryStore::open(db_path).map_err(|_| super::jobs::io())?;
    let storage = SqliteFusionStorage::new(Arc::new(Mutex::new(store)));
    let record = storage
        .get_manifest(game, expected_manifest_blake3)?
        .ok_or_else(integrity)?;
    if blake3::hash(&record.raw_bytes).to_hex().as_str() != expected_manifest_blake3
        || record.raw_bytes != expected_manifest
    {
        return Err(integrity());
    }
    let cas = CasStore::open(library)?;
    for cas_digest in expected_cas.values() {
        cas.verify(cas_digest)?;
    }
    Ok(())
}
