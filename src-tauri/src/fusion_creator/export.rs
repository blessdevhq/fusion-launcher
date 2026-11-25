//! Allowlisted, verified-only export of a portable managed release without private inputs or keys.
use super::build::{insert_capability_hashes, supported_component, supported_controls, INDEX_ID};
use super::jobs::{
    path_contains, prospective_root, JobContext, JobHandler, ReleaseContent, ResumePoint,
};
use super::verify;
use super::{
    CreatorCheckpoint, CreatorCheckpointBoundary, CreatorJobStartRequest, CreatorReportKind,
    CreatorReportStatus, SavedPrivateBindings,
};
use crate::fusion_source::{
    cas::hash_file,
    errors::{ErrorCode, FusionError},
    fs_safety,
    packer::PackerEngine,
    program_schema::CreatorProgramProject,
    schema::DigestAlgorithm,
    signatures,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub(super) struct ManagedReleaseExport;

fn unavailable(message: &'static str) -> FusionError {
    FusionError::new(ErrorCode::ActionUnavailable, message)
        .with_detail("command", "fusion_creator_export")
}

fn integrity(message: &'static str) -> FusionError {
    FusionError::new(ErrorCode::IntegrityViolation, message)
}

fn implementation_digest() -> &'static str {
    static DIGEST: OnceLock<String> = OnceLock::new();
    DIGEST.get_or_init(|| {
        let mut hash = blake3::Hasher::new();
        hash.update(verify::implementation_digest().as_bytes());
        hash.update(&blake3::hash(include_bytes!("export.rs")).as_bytes()[..]);
        hash.finalize().to_hex().to_string()
    })
}

struct SiblingStage {
    path: PathBuf,
    armed: bool,
}
impl SiblingStage {
    fn new(parent: &Path) -> Result<Self, FusionError> {
        let name = format!(".{}", fs_safety::unique_name("fusion-export"));
        let path = parent.join(name);
        signatures::create_private_directory(&path).map_err(|_| super::jobs::io())?;
        Ok(Self { path, armed: true })
    }
    fn disarm(&mut self) {
        self.armed = false;
    }
}
impl Drop for SiblingStage {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs_safety::safe_remove_tree(&self.path);
        }
    }
}

fn target_path(request: &CreatorJobStartRequest) -> Result<PathBuf, FusionError> {
    let raw = request
        .target_directory
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| unavailable("Export requires an explicit target directory."))?;
    let path = PathBuf::from(raw);
    if !path.is_absolute() || path.parent().is_none() {
        return Err(FusionError::new(
            ErrorCode::InvalidPath,
            "Export target directory must be a non-root absolute path.",
        ));
    }
    Ok(path)
}

impl JobHandler for ManagedReleaseExport {
    fn validate_request(&self, request: &CreatorJobStartRequest) -> Result<(), FusionError> {
        if request.release_id.as_ref().is_none_or(String::is_empty) || request.source_url.is_some()
        {
            return Err(unavailable(
                "Export requires a saved release ID and an absolute target directory without a Source URL.",
            ));
        }
        let _ = target_path(request)?;
        Ok(())
    }

    fn deferred_input_audit(&self) -> bool {
        true
    }

    fn validate_write_roots(
        &self,
        _: &CreatorProgramProject,
        _: Option<&SavedPrivateBindings>,
        _: &Path,
        _: &Path,
    ) -> Result<(), FusionError> {
        Ok(())
    }

    fn validate_write_roots_for_request(
        &self,
        request: &CreatorJobStartRequest,
        _: &CreatorProgramProject,
        bindings: Option<&SavedPrivateBindings>,
        workspace: &Path,
        release_cas: &Path,
    ) -> Result<(), FusionError> {
        let target = prospective_root(&target_path(request)?).map_err(|_| {
            FusionError::new(
                ErrorCode::InvalidPath,
                "Could not resolve the Export target directory.",
            )
        })?;
        let overlaps = |candidate: &Path| -> Result<bool, FusionError> {
            let resolved = prospective_root(candidate).map_err(|_| {
                FusionError::new(
                    ErrorCode::InvalidPath,
                    "Could not inspect protected Creator directories.",
                )
            })?;
            Ok(path_contains(&target, &resolved) || path_contains(&resolved, &target))
        };
        let mut protected = vec![workspace.to_path_buf(), release_cas.to_path_buf()];
        if let Some(root) = workspace.parent().and_then(|parent| parent.parent()) {
            protected.push(root.to_path_buf());
        }
        if let Some(saved) = bindings {
            for path in saved.bindings.component_paths.values() {
                if !path.is_empty() {
                    protected.push(PathBuf::from(path));
                }
            }
            if let Some(key) = &saved.bindings.signing_key_ref {
                if !key.is_empty() {
                    protected.push(PathBuf::from(key));
                }
            }
        }
        for candidate in &protected {
            if overlaps(candidate)? {
                return Err(FusionError::new(
                    ErrorCode::InvalidPath,
                    "Export target directory cannot overlap Creator storage, workspaces or private inputs.",
                ));
            }
        }
        Ok(())
    }

    fn input_hashes(
        &self,
        _: &CreatorProgramProject,
        _: Option<&SavedPrivateBindings>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        Err(unavailable("Export requires a verified release."))
    }

    fn input_hashes_for_release(
        &self,
        project: &CreatorProgramProject,
        _: Option<&SavedPrivateBindings>,
        release: Option<&ReleaseContent>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        let release = release.ok_or_else(|| unavailable("Export requires a release."))?;
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
                "Export does not support this component origin or controls writer.",
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
        Ok(
            checkpoint.boundary == CreatorCheckpointBoundary::ExportCompleted
                && !evidence.is_empty(),
        )
    }

    fn run(&self, context: JobContext, resume: Option<ResumePoint>) -> Result<(), FusionError> {
        self.validate_request(context.request())?;
        if resume.is_some() {
            return Err(unavailable(
                "Export restarts cleanly from the verified release.",
            ));
        }
        let project = context.project();
        let release = context
            .selected_release()
            .ok_or_else(|| unavailable("Export requires an audited release."))?;
        let target = target_path(context.request())?;

        context.progress("checking_verification", 0, Some(3))?;
        ensure_all_pairs_verified(&context, release)?;
        context.check_cancelled()?;

        let parent = target.parent().ok_or_else(|| {
            FusionError::new(
                ErrorCode::InvalidPath,
                "Export target directory requires a valid parent directory.",
            )
        })?;
        let _parent_guard = fs_safety::guard(parent)?;
        if target.exists() {
            let meta = std::fs::symlink_metadata(&target)
                .map_err(|e| fs_safety::io_error("Inspect export target", &target, e))?;
            fs_safety::check_metadata(&meta, &target)?;
            if !meta.is_dir() {
                return Err(FusionError::new(
                    ErrorCode::InvalidPath,
                    "Export target path already exists and is not a directory.",
                ));
            }
            let _target_guard = fs_safety::guard(&target)?;
            let mut entries = std::fs::read_dir(&target)
                .map_err(|e| fs_safety::io_error("Read export target", &target, e))?;
            if entries
                .next()
                .transpose()
                .map_err(|_| super::jobs::io())?
                .is_some()
            {
                return Err(FusionError::new(
                    ErrorCode::SnapshotConflict,
                    "Export target directory must be empty.",
                ));
            }
        }

        context.progress("auditing_release", 1, Some(3))?;
        let mut stage = SiblingStage::new(parent)?;
        let materialized = context
            .with_work_checks(|check| verify::materialize(release, project, &stage.path, check))?;
        if !materialized.portable {
            return Err(FusionError::new(
                ErrorCode::InvalidUri,
                "This release embeds local offline.fusion.invalid URLs and cannot be exported for HTTPS. Build a new release to produce portable Source-relative links.",
            )
            .with_detail("reason", "legacy_offline_release_urls"));
        }

        let audit = context.with_work_checks(|check| {
            PackerEngine::new().verify_audit_checked(&stage.path, check)
        })?;
        if !audit.report.signature_verified
            || audit.report.source_id != project.source_id
            || audit.report.manifests_verified != 1
            || audit.report.artifacts_verified != project.program_descriptor.components.len()
            || audit.report.attestations_verified != 0
        {
            return Err(integrity("Exported publication failed Packer audit."));
        }

        let mut expected_files = BTreeSet::from([
            INDEX_ID.to_owned(),
            materialized.manifest_relative.clone(),
            materialized.inventory_relative.clone(),
        ]);
        for id in &materialized.artifact_ids {
            expected_files.insert(format!("artifacts/{id}"));
        }
        let plain = context
            .with_work_checks(|check| fs_safety::list_plain_files_checked(&stage.path, check))?;
        let actual_files: BTreeSet<String> = plain.iter().map(|(rel, _)| rel.clone()).collect();
        if actual_files != expected_files {
            return Err(integrity(
                "Export staging contains unapproved files outside the Source allowlist.",
            ));
        }
        let mut exported_hashes = BTreeMap::new();
        for (rel, path) in plain {
            context.check_cancelled()?;
            exported_hashes.insert(rel, hash_file(&path, DigestAlgorithm::Blake3)?);
        }

        context.progress("publishing", 2, Some(3))?;
        context.check_cancelled()?;
        publish_export(&mut stage, &target)?;

        let evidence = serde_json::to_vec(&serde_json::json!({
            "releaseId": release.release().release_id,
            "manifestBlake3": release.release().manifest_blake3,
            "files": exported_hashes,
        }))
        .map_err(|_| super::jobs::io())?;
        context.checkpoint_exported(evidence)?;
        Ok(())
    }
}

fn ensure_all_pairs_verified(
    context: &JobContext,
    release: &ReleaseContent,
) -> Result<(), FusionError> {
    let project = context.project();
    let mut required_pairs = BTreeSet::new();
    for build in &project.program_descriptor.build_profiles {
        for controls_id in &build.controls_profile_ids {
            required_pairs.insert((build.id.clone(), controls_id.clone()));
        }
    }
    if required_pairs.is_empty() {
        return Err(unavailable(
            "Export requires at least one declared build and controls profile pair.",
        ));
    }

    let reports = context.release_reports(&release.release().release_id)?;
    let mut structure_passed = BTreeSet::new();
    let mut clean_passed = BTreeSet::new();
    for report in reports {
        if report.release_id != release.release().release_id
            || report.release_blake3 != release.release().manifest_blake3
            || report.project_revision != project.revision
            || report.executor != project.program_descriptor.executor
        {
            continue;
        }
        if !matches!(
            report.kind,
            CreatorReportKind::StructureIntegrity | CreatorReportKind::CleanInstall
        ) {
            continue;
        }
        match report.status {
            CreatorReportStatus::Failed => {
                return Err(integrity(
                    "Release has a failed verification report and cannot be exported.",
                ));
            }
            CreatorReportStatus::Passed => {
                let pair = (report.build_profile_id, report.controls_profile_id);
                match report.kind {
                    CreatorReportKind::StructureIntegrity => {
                        structure_passed.insert(pair);
                    }
                    CreatorReportKind::CleanInstall => {
                        clean_passed.insert(pair);
                    }
                    _ => {}
                }
            }
            CreatorReportStatus::NotTested => {}
        }
    }

    let missing_pairs: Vec<String> = required_pairs
        .into_iter()
        .filter(|pair| !(structure_passed.contains(pair) && clean_passed.contains(pair)))
        .map(|(build, controls)| format!("{build}/{controls}"))
        .collect();
    if !missing_pairs.is_empty() {
        return Err(FusionError::new(
            ErrorCode::ActionUnavailable,
            "All declared compatible profile pairs must pass both structure integrity and clean install verification before exporting.",
        )
        .with_detail("command", "fusion_creator_export")
        .with_detail("reason", "verification_required")
        .with_detail("missingPairs", missing_pairs));
    }
    Ok(())
}

fn publish_export(stage: &mut SiblingStage, target: &Path) -> Result<(), FusionError> {
    if !target.exists() {
        fs_safety::publish_directory(&stage.path, target)?;
        stage.disarm();
        return Ok(());
    }
    // Remove an empty destination directory so directory publication stays a single
    // atomic rename; restore the empty dir or fall back cleanly if held open on Windows.
    if std::fs::remove_dir(target).is_ok() {
        if let Err(error) = fs_safety::publish_directory(&stage.path, target) {
            let _ = std::fs::create_dir(target);
            return Err(error);
        }
        stage.disarm();
        return Ok(());
    }

    let mut moved_dirs: Vec<(&str, PathBuf)> = Vec::new();
    for dir in ["manifests", "artifacts"] {
        let src = stage.path.join(dir);
        let dst = target.join(dir);
        if let Err(error) = fs_safety::publish_directory(&src, &dst) {
            for (orig, placed) in moved_dirs.into_iter().rev() {
                let _ = fs_safety::publish_directory(&placed, &stage.path.join(orig));
            }
            return Err(error);
        }
        moved_dirs.push((dir, dst));
    }
    let index_bytes = std::fs::read(stage.path.join(INDEX_ID))
        .map_err(|e| fs_safety::io_error("Read staged index", &stage.path, e))?;
    if let Err(error) = fs_safety::atomic_write(&target.join(INDEX_ID), &index_bytes) {
        let _ = fs_safety::unlink_file(&target.join(INDEX_ID));
        for (_, placed) in moved_dirs {
            let _ = fs_safety::safe_remove_tree(&placed);
        }
        return Err(error);
    }
    Ok(())
}
