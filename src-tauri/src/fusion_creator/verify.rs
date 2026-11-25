//! Clean-install verification for the exact signed managed Build receipt.
use super::build::{
    insert_capability_hashes, supported_component, supported_controls, INDEX_ID, INVENTORY_ID,
};
use super::jobs::{CheckpointOutput, JobContext, JobHandler, ReleaseContent, ResumePoint};
use super::{
    CreatorCheckpoint, CreatorCheckpointBoundary, CreatorJobStartRequest, CreatorReport,
    CreatorReportKind, CreatorReportStatus, SavedPrivateBindings,
};
use crate::fusion_source::{
    adapters::ArtifactTransport,
    canonical::QualifiedGameId,
    dto::{GameOptions, InstallRequest},
    errors::{ErrorCode, FusionError},
    evolution::SchemaUpcaster,
    fs_safety,
    integrity::PreLaunchIntegrityGuard,
    layers::LayerInput,
    offline::{OfflineTransport, CREATOR_SOURCE_ROOT},
    packer::PackerEngine,
    program_schema::{CreatorProgramProject, ProgramControlsProfile, MAX_PROGRAM_BYTES},
    schema::{DigestAlgorithm, LayerSource, SourceIndex},
    service::{FusionService, NoProgress, ServiceEnvironment},
    signatures,
    workspace::WorkspaceAssembler,
};
use crate::storage::{fusion_source::SqliteFusionStorage, RepositoryStore};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub(super) struct ManagedReleaseVerify;

fn unsupported() -> FusionError {
    FusionError::new(
        ErrorCode::ActionUnavailable,
        "Verify requires a saved managed release and one compatible build and controls profile pair.",
    )
}

fn integrity() -> FusionError {
    FusionError::new(
        ErrorCode::IntegrityViolation,
        "The saved release does not match its signed managed publication.",
    )
}

fn redact(error: FusionError, stage: &'static str) -> FusionError {
    FusionError::new(
        error.code,
        "Creator could not verify or install the saved release in a clean library.",
    )
    .with_detail("stage", stage)
}

fn clean_install_error(error: FusionError) -> FusionError {
    if error.details.get("reason") == Some(&serde_json::json!("environment_approval_required")) {
        return error.with_detail("passedStages", vec!["structure_integrity"]);
    }
    redact(error, "clean_install").with_detail("passedStages", vec!["structure_integrity"])
}

pub(super) fn implementation_digest() -> &'static str {
    static DIGEST: OnceLock<String> = OnceLock::new();
    DIGEST.get_or_init(|| {
        let mut hash = blake3::Hasher::new();
        for bytes in [
            include_bytes!("verify.rs").as_slice(),
            include_bytes!("build.rs"),
            include_bytes!("declarative/capabilities.rs"),
            include_bytes!("declarative/capabilities/eden_param_package.rs"),
            include_bytes!("declarative/runtime.rs"),
            include_bytes!("declarative/compiler.rs"),
            include_bytes!("declarative/candidate.rs"),
            include_bytes!("declarative/codecs.rs"),
            include_bytes!("../input/eden.rs"),
            include_bytes!("../input/service.rs"),
            include_bytes!("projection.rs"),
            include_bytes!("../fusion_source/evolution.rs"),
            include_bytes!("../fusion_source/service.rs"),
            include_bytes!("../fusion_source/workspace.rs"),
            include_bytes!("../fusion_source/layers.rs"),
            include_bytes!("../fusion_source/offline.rs"),
            include_bytes!("../fusion_source/packer.rs"),
            include_bytes!("../fusion_source/cas.rs"),
            include_bytes!("../fusion_source/transforms.rs"),
            include_bytes!("../fusion_source/fs_safety.rs"),
            include_bytes!("../fusion_source/integrity.rs"),
            include_bytes!("../fusion_source/plan.rs"),
            include_bytes!("../../Cargo.lock"),
        ] {
            hash.update(&blake3::hash(bytes).as_bytes()[..]);
        }
        hash.finalize().to_hex().to_string()
    })
}

impl JobHandler for ManagedReleaseVerify {
    fn validate_request(&self, request: &CreatorJobStartRequest) -> Result<(), FusionError> {
        if request.release_id.as_ref().is_none_or(String::is_empty)
            || request.target_directory.is_some()
            || request.source_url.is_some()
            || request.build_profile_ids.len() != 1
            || request.controls_profile_ids.len() != 1
        {
            return Err(unsupported());
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
        Err(unsupported())
    }

    fn input_hashes_for_release(
        &self,
        project: &CreatorProgramProject,
        _: Option<&SavedPrivateBindings>,
        release: Option<&ReleaseContent>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        let release = release.ok_or_else(unsupported)?;
        let descriptor = &project.program_descriptor;
        if descriptor.components.is_empty()
            || descriptor
                .components
                .iter()
                .any(|component| !supported_component(component))
            || descriptor
                .controls_profiles
                .iter()
                .any(|profile| !supported_controls(profile))
        {
            return Err(unsupported());
        }
        // JobRunner already audited the exact receipt-backed bundle. There are no
        // independent author file inputs; these component stamps bind to its bytes.
        let mut inputs = BTreeMap::from([(
            "executor_implementation".into(),
            implementation_digest().into(),
        )]);
        insert_capability_hashes(descriptor, &mut inputs)?;
        for component in &descriptor.components {
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
                matches!(
                    report.kind,
                    CreatorReportKind::StructureIntegrity | CreatorReportKind::CleanInstall
                ) && report.status == CreatorReportStatus::Passed
            }),
        )
    }

    fn run(&self, context: JobContext, resume: Option<ResumePoint>) -> Result<(), FusionError> {
        self.validate_request(context.request())?;

        let project = context.project();
        let release = context.selected_release().ok_or_else(unsupported)?;
        let build = &context.request().build_profile_ids[0];
        let controls = &context.request().controls_profile_ids[0];
        context.progress("structure_integrity", 0, Some(2))?;
        let scratch = Scratch::new(context.workspace())?;
        let publication = scratch.0.join("source");
        context.with_work_checks(|check| {
            materialize(release, project, &publication, check)
                .map_err(|error| redact(error, "materialize"))
        })?;
        let verification = context.with_work_checks(|check| {
            PackerEngine::new()
                .verify_audit_checked(&publication, check)
                .map_err(|error| redact(error, "publication_audit"))
        })?;
        if !verification.report.signature_verified
            || verification.report.source_id != project.source_id
            || verification.report.manifests_verified != 1
            || verification.report.artifacts_verified != project.program_descriptor.components.len()
            || verification.report.attestations_verified != 0
        {
            return Err(integrity());
        }
        if let Some(resume) = resume {
            let report: CreatorReport =
                serde_json::from_slice(&resume.evidence).map_err(|_| integrity())?;
            if report.kind != CreatorReportKind::StructureIntegrity
                || report.status != CreatorReportStatus::Passed
                || report.executor != project.program_descriptor.executor
                || report.release_id != release.release().release_id
                || report.release_blake3 != release.release().manifest_blake3
                || report.project_revision != project.revision
                || report.build_profile_id != *build
                || report.controls_profile_id != *controls
            {
                return Err(integrity());
            }
        } else {
            publish_report(
                &context,
                release,
                CreatorReportKind::StructureIntegrity,
                build,
                controls,
                false,
            )?;
        }
        context.progress("clean_install", 1, Some(2))?;
        if let Err(error) = tauri::async_runtime::block_on(clean_install(
            &context,
            &publication,
            &scratch.0,
            release,
            build,
            controls,
        )) {
            if matches!(
                error.code,
                ErrorCode::Cancelled | ErrorCode::SnapshotConflict
            ) {
                return Err(error);
            }
            let sanitized = clean_install_error(error);
            let failed_report = build_report(
                &context,
                release,
                CreatorReportKind::CleanInstall,
                CreatorReportStatus::Failed,
                build,
                controls,
                vec![sanitized.clone()],
            );
            context.fail_with_report(failed_report, sanitized.clone())?;
            return Err(sanitized);
        }
        fs_safety::safe_remove_tree(&scratch.0).map_err(|error| redact(error, "cleanup"))?;
        context.progress("clean_install", 2, Some(2))?;
        publish_report(
            &context,
            release,
            CreatorReportKind::CleanInstall,
            build,
            controls,
            true,
        )
    }
}

pub(super) fn build_report(
    context: &JobContext,
    release: &ReleaseContent,
    kind: CreatorReportKind,
    status: CreatorReportStatus,
    build: &str,
    controls: &str,
    diagnostics: Vec<FusionError>,
) -> CreatorReport {
    CreatorReport {
        report_id: fs_safety::unique_name("verify"),
        release_id: release.release().release_id.clone(),
        release_blake3: release.release().manifest_blake3.clone(),
        project_revision: context.project().revision,
        kind,
        status,
        executor: context.project().program_descriptor.executor.clone(),
        build_profile_id: build.to_owned(),
        controls_profile_id: controls.to_owned(),
        environment: BTreeMap::from([
            ("mode".into(), "isolated_offline_library".into()),
            ("os".into(), std::env::consts::OS.into()),
            ("arch".into(), std::env::consts::ARCH.into()),
        ]),
        diagnostics,
    }
}

fn publish_report(
    context: &JobContext,
    release: &ReleaseContent,
    kind: CreatorReportKind,
    build: &str,
    controls: &str,
    terminal: bool,
) -> Result<(), FusionError> {
    let report = build_report(
        context,
        release,
        kind,
        CreatorReportStatus::Passed,
        build,
        controls,
        vec![],
    );
    let output = CheckpointOutput {
        boundary: CreatorCheckpointBoundary::VerificationCompleted,
        evidence: serde_json::to_vec(&report).map_err(|_| integrity())?,
        artifact_id: None,
        release: None,
        report: Some(report),
    };
    if terminal {
        context.checkpoint_verified(output)?;
    } else {
        context.checkpoint(output)?;
    }
    Ok(())
}

pub(super) struct MaterializedRelease {
    pub(super) portable: bool,
    pub(super) public_key: String,
    pub(super) manifest_relative: String,
    pub(super) inventory_relative: String,
    pub(super) artifact_ids: BTreeSet<String>,
}

pub(super) fn materialize(
    release: &ReleaseContent,
    project: &CreatorProgramProject,
    publication: &Path,
    check: &dyn Fn() -> Result<(), FusionError>,
) -> Result<MaterializedRelease, FusionError> {
    let read_metadata = |id| -> Result<Vec<u8>, FusionError> {
        const LIMIT: u64 = 8 * 1024 * 1024;
        let mut bytes = Vec::new();
        release
            .open_artifact_checked(id, check)?
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| super::jobs::io())?;
        if bytes.is_empty() || bytes.len() as u64 > LIMIT {
            return Err(integrity());
        }
        Ok(bytes)
    };
    let raw_index = read_metadata(INDEX_ID)?;
    let text = std::str::from_utf8(&raw_index).map_err(|_| integrity())?;
    let wire = signatures::parse_wire_index(text)?;
    if !signatures::verify_wire_index(&wire, None, None)? {
        return Err(integrity());
    }
    let index: SourceIndex = serde_json::from_str(text).map_err(|_| integrity())?;
    let public_key = index
        .source_info
        .public_key
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(integrity)?
        .to_owned();
    let [game] = index.games.as_slice() else {
        return Err(integrity());
    };
    if index.source_info.id != project.source_id
        || game.id != project.game_id
        || game.version != project.game_version
        || game.manifest_blake3 != release.release().manifest_blake3
    {
        return Err(integrity());
    }
    let manifest = SchemaUpcaster::parse_game_manifest(
        std::str::from_utf8(release.manifest()).map_err(|_| integrity())?,
    )?;
    if manifest.extends.is_some()
        || manifest.layers.len() != project.program_descriptor.components.len()
        || manifest
            .creator
            .as_ref()
            .and_then(|creator| creator.as_program())
            .map(|creator| &creator.program_descriptor)
            != Some(&project.program_descriptor)
    {
        return Err(integrity());
    }
    let stem = format!("manifests/{}-{}", game.id, game.manifest_blake3);
    let manifest_relative = format!("{stem}.json");
    let inventory_relative = format!("{stem}.artifacts.json");
    let portable = if game.manifest_url == manifest_relative {
        true
    } else if game.manifest_url == format!("{CREATOR_SOURCE_ROOT}{manifest_relative}") {
        false
    } else {
        return Err(integrity());
    };
    let mut ids = BTreeSet::new();
    for layer in &manifest.layers {
        use crate::fusion_source::schema::ArchiveFormat;
        let LayerSource::Remote {
            archive: ArchiveFormat::Zip,
            mirrors,
        } = &layer.source
        else {
            return Err(integrity());
        };
        let digest = layer
            .transport_digest
            .as_ref()
            .filter(|digest| digest.algorithm == DigestAlgorithm::Blake3)
            .ok_or_else(integrity)?;
        let id = format!("{}.zip", digest.value);
        let expected_mirror = if portable {
            format!("artifacts/{id}")
        } else {
            format!("{CREATOR_SOURCE_ROOT}artifacts/{id}")
        };
        if mirrors.as_slice() != [expected_mirror] {
            return Err(integrity());
        }
        if release.release().artifact_hashes.get(&id) != Some(&digest.value) {
            return Err(integrity());
        }
        ids.insert(id);
    }
    if release.release().artifact_hashes.len() != ids.len() + 2 {
        return Err(integrity());
    }
    fs_safety::ensure_directory(&publication.join("manifests"))?;
    fs_safety::ensure_directory(&publication.join("artifacts"))?;
    fs_safety::atomic_write(&publication.join(INDEX_ID), &raw_index)?;
    fs_safety::atomic_write(&publication.join(&manifest_relative), release.manifest())?;
    fs_safety::atomic_write(
        &publication.join(&inventory_relative),
        &read_metadata(INVENTORY_ID)?,
    )?;
    for id in &ids {
        check()?;
        let path = publication
            .join("artifacts")
            .join(fs_safety::validate_relative_path(id)?);
        let mut source = release.open_artifact_checked(id, check)?;
        let mut output = fs_safety::create_new(&path)?;
        fs_safety::stream_copy_checked(&mut source, &mut output, &path, check)?;
        output.sync_all().map_err(|_| super::jobs::io())?;
    }
    Ok(MaterializedRelease {
        portable,
        public_key,
        manifest_relative,
        inventory_relative,
        artifact_ids: ids,
    })
}

pub(super) fn reject_unapproved_env_paths(
    project: &CreatorProgramProject,
    stage: &'static str,
) -> Result<(), FusionError> {
    if !project
        .program_descriptor
        .entrypoint
        .environment_paths
        .is_empty()
    {
        let value = serde_json::to_value(&project.program_descriptor).map_err(|_| integrity())?;
        let bytes = serde_json::to_vec(&value).map_err(|_| integrity())?;
        let descriptor_blake3 = blake3::hash(&bytes).to_hex().to_string();
        let variables: Vec<String> = project
            .program_descriptor
            .entrypoint
            .environment_paths
            .iter()
            .map(|variable| variable.name.clone())
            .collect();
        return Err(FusionError::new(
            ErrorCode::ActionUnavailable,
            "Clean install requires explicit local approval for declared application environment paths bound to this exact descriptor hash and variable set.",
        )
        .with_detail("stage", stage)
        .with_detail("reason", "environment_approval_required")
        .with_detail("descriptorBlake3", descriptor_blake3)
        .with_detail("variables", variables));
    }
    Ok(())
}

async fn clean_install(
    context: &JobContext,
    publication: &Path,
    scratch: &Path,
    release: &ReleaseContent,
    build: &str,
    controls: &str,
) -> Result<(), FusionError> {
    let transport = Arc::new(OfflineTransport::open_creator(publication)?);
    let index_url = transport.index_url();
    execute_isolated_install(
        context,
        scratch,
        transport,
        &index_url,
        release,
        build,
        controls,
        None,
        "clean_install",
    )
    .await
    .map(|_| ())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_isolated_install(
    context: &JobContext,
    scratch: &Path,
    transport: Arc<dyn ArtifactTransport>,
    index_url: &str,
    release: &ReleaseContent,
    build: &str,
    controls: &str,
    expected_public_key: Option<&str>,
    stage: &'static str,
) -> Result<(PathBuf, PathBuf, String), FusionError> {
    let project = context.project();
    reject_unapproved_env_paths(project, stage)?;
    context.check_cancelled()?;
    let library = scratch.join("clean-library");
    let appdata = scratch.join("appdata");
    let local_appdata = scratch.join("localappdata");
    for path in [&library, &appdata, &local_appdata] {
        std::fs::create_dir(path).map_err(|_| super::jobs::io())?;
    }
    let db_path = scratch.join("clean.sqlite");
    let store = RepositoryStore::open(&db_path).map_err(|_| super::jobs::io())?;
    let storage = Arc::new(SqliteFusionStorage::new(Arc::new(Mutex::new(store))));
    let service = FusionService::open(
        &library,
        storage,
        transport,
        Arc::new(NoProgress),
        ServiceEnvironment {
            appdata,
            local_appdata,
            display_width: 1920,
            display_height: 1080,
            refresh_rate: 60,
        },
    )?;
    let game = QualifiedGameId {
        source_id: project.source_id.clone(),
        game_id: project.game_id.clone(),
    };
    context.check_cancelled()?;
    if let Some(expected_key) = expected_public_key {
        let preview = service.source_preview(index_url).await?;
        if !preview.signature_verified
            || preview.source_id != project.source_id
            || preview.public_key.as_deref() != Some(expected_key)
        {
            return Err(integrity());
        }
        context.check_cancelled()?;
    }
    service.source_add(index_url).await?;
    context.check_cancelled()?;
    let options = GameOptions {
        build_profile_id: Some(build.into()),
        controls_profile_id: Some(controls.into()),
        ..GameOptions::default()
    };
    let before = service.game_set_options(&game, options).await?;
    if before.installed || before.active_snapshot_id.is_some() {
        return Err(integrity());
    }
    context.check_cancelled()?;
    let result = {
        let install = service.game_install(&game, InstallRequest::default());
        tokio::pin!(install);
        loop {
            tokio::select! {
                biased;
                result = &mut install => break result?,
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if let Err(error) = context.check_cancelled() {
                        // Do not drop the waiter: the installer owns a separate task
                        // and must release its workspace before scratch can be removed.
                        let _ = service.game_cancel(&game);
                        let _ = install.await;
                        return Err(error);
                    }
                }
            }
        }
    };
    context.check_cancelled()?;
    if let Some(error) = result.error {
        return Err(error);
    }
    if result.status != "completed" {
        return Err(integrity());
    }
    let state = service.game_get_state(&game).await?;
    let active_snapshot_id = state.active_snapshot_id.clone().ok_or_else(integrity)?;
    // Snapshot options normalize defaults (e.g. Some([]) for no add-ons).
    // The evidence binds to the actual profile IDs used by the install plan.
    if !state.installed
        || !state.applied_options.as_ref().is_some_and(|applied| {
            applied.build_profile_id.as_deref() == Some(build)
                && applied.controls_profile_id.as_deref() == Some(controls)
        })
    {
        return Err(integrity());
    }
    drop(service);
    let library_verify = library.clone();
    let snapshot_verify = active_snapshot_id.clone();
    let project_verify = project.clone();
    let release_verify = release.clone();
    let build_verify = build.to_owned();
    let controls_verify = controls.to_owned();
    tokio::task::spawn_blocking(move || {
        verify_clean_install_snapshot(
            &library_verify,
            &game,
            &snapshot_verify,
            &project_verify,
            &release_verify,
            &build_verify,
            &controls_verify,
        )
    })
    .await
    .map_err(|_| super::jobs::io())??;
    context.check_cancelled()?;
    Ok((library, db_path, active_snapshot_id))
}

pub(super) fn verify_clean_install_snapshot(
    library: &Path,
    game: &QualifiedGameId,
    active_snapshot_id: &str,
    project: &CreatorProgramProject,
    release: &ReleaseContent,
    build: &str,
    controls: &str,
) -> Result<(), FusionError> {
    verify_snapshot_with_pin(
        library,
        game,
        active_snapshot_id,
        project,
        &release.release().manifest_blake3,
        build,
        controls,
    )
}

fn verify_snapshot_with_pin(
    library: &Path,
    game: &QualifiedGameId,
    active_snapshot_id: &str,
    project: &CreatorProgramProject,
    expected_manifest_blake3: &str,
    build: &str,
    controls: &str,
) -> Result<(), FusionError> {
    let lease = WorkspaceAssembler::open(library, game)?
        .active()?
        .ok_or_else(integrity)?;
    if lease.snapshot.id != active_snapshot_id
        || lease.snapshot.game != *game
        || lease.snapshot.source_manifest_blake3.as_deref() != Some(expected_manifest_blake3)
        || lease.snapshot.manifest.id != project.game_id
        || lease.snapshot.manifest.version != project.game_version
        || lease.snapshot.manifest.source_id != project.source_id
        || crate::fusion_creator::declarative::runtime::descriptor(&lease.snapshot.manifest)
            != Some(&project.program_descriptor)
    {
        return Err(integrity());
    }
    let build_profile = project
        .program_descriptor
        .build_profiles
        .iter()
        .find(|profile| profile.id == build)
        .ok_or_else(integrity)?;
    if lease.snapshot.plan.build_profile_id.as_deref() != Some(build)
        || lease.snapshot.plan.controls_profile_id.as_deref() != Some(controls)
    {
        return Err(integrity());
    }
    let expected_layers: BTreeSet<&str> =
        build_profile.layer_ids.iter().map(String::as_str).collect();
    let actual_layers: BTreeSet<&str> = lease
        .snapshot
        .plan
        .selected_layers
        .iter()
        .map(String::as_str)
        .collect();
    if lease.snapshot.plan.selected_layers.len() != build_profile.layer_ids.len()
        || actual_layers != expected_layers
        || !build_profile.layer_ids.iter().all(|layer_id| {
            matches!(
                lease.snapshot.inputs.get(layer_id),
                Some(LayerInput::Cas { .. })
            )
        })
    {
        return Err(integrity());
    }
    lease.verify_execution_contract()?;
    PreLaunchIntegrityGuard::verify_integrity(&lease.snapshot)?;
    let _workspace = fs_safety::guard(&lease.snapshot.workspace_root)?;
    for directory in &project.program_descriptor.directories {
        let path = if directory.is_empty() {
            lease.snapshot.workspace_root.clone()
        } else {
            lease
                .snapshot
                .workspace_root
                .join(fs_safety::validate_relative_path(directory)?)
        };
        fs_safety::guard(&path)?;
    }
    let controls_profile = project
        .program_descriptor
        .controls_profiles
        .iter()
        .find(|profile| profile.id() == controls)
        .ok_or_else(integrity)?;
    for config in &project.program_descriptor.configs {
        let mut expected = crate::fusion_creator::declarative::codecs::render_config(
            config,
            None,
            &build_profile.settings,
        )?;
        if let ProgramControlsProfile::EdenParamPackageV1 { config_id, .. } = controls_profile {
            if config_id == &config.id {
                let assignments =
                    crate::fusion_creator::declarative::capabilities::ProgramCapabilityRegistry
                        .render_controls_plan(controls_profile, config, None)?;
                expected = crate::fusion_creator::declarative::codecs::apply_ini_assignments(
                    &expected,
                    assignments,
                )?;
            }
        }
        let path = lease
            .snapshot
            .workspace_root
            .join(fs_safety::validate_relative_path(&config.path)?);
        let file = fs_safety::open_read(&path)?;
        if fs_safety::link_count(&file)? != 1 {
            return Err(integrity());
        }
        let mut actual = Vec::new();
        file.take(MAX_PROGRAM_BYTES as u64 + 1)
            .read_to_end(&mut actual)
            .map_err(|_| super::jobs::io())?;
        if actual.len() > MAX_PROGRAM_BYTES || actual != expected.as_bytes() {
            return Err(integrity());
        }
    }
    drop(lease);
    Ok(())
}

pub(super) struct Scratch(pub(super) PathBuf);
impl Scratch {
    pub(super) fn new(workspace: &Path) -> Result<Self, FusionError> {
        Self::new_prefixed(workspace, "verify")
    }
    pub(super) fn new_prefixed(workspace: &Path, prefix: &str) -> Result<Self, FusionError> {
        let path = workspace.join(fs_safety::unique_name(prefix));
        signatures::create_private_directory(&path).map_err(|_| super::jobs::io())?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs_safety::safe_remove_tree(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fusion_creator::{
        bridge::production_runner, project_service, project_storage, CreatorJobKind,
        CreatorJobStatus, CreatorPrivateBindings,
    };

    #[tokio::test]
    async fn snapshot_verification_checks_manifest_pin_layers_and_config_values() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let runtime_dir = dir.path().join("runtime-input");
        let addon_dir = dir.path().join("addon-input");
        std::fs::create_dir(&runtime_dir).unwrap();
        std::fs::create_dir(&addon_dir).unwrap();
        std::fs::write(runtime_dir.join("app.exe"), b"synthetic program exe").unwrap();
        std::fs::write(addon_dir.join("extra.dat"), b"synthetic addon payload").unwrap();
        let key = dir.path().join("key.pem");
        signatures::keygen(&key).unwrap();
        let mut store = RepositoryStore::open(&dir.path().join("creator.sqlite")).unwrap();
        let project: CreatorProgramProject = serde_json::from_value(json!({
            "schema_version":"1.1","project_id":"verify-snapshot","revision":1,"title":"Snapshot Check",
            "source_id":"signed-source","game_id":"snapshot-demo","game_version":"1.0",
            "program_descriptor":{
                "document_type":"program_descriptor","schema_version":"1.0",
                "identity":{"id":"snapshot-demo","title":"Snapshot Demo","version":"1.0","os":"windows","architecture":"x86_64"},
                "executor":{"id":"fusion.declarative","version":"1.0"},
                "components":[
                    {"id":"program","layer_id":"runtime","origin":{"kind":"managed","artifact_ref":"program","redistribution_allowed":true},"source_root":"","mount_path":"app"},
                    {"id":"addon","layer_id":"addon","origin":{"kind":"managed","artifact_ref":"addon","redistribution_allowed":true},"source_root":"","mount_path":"app/addons"}
                ],
                "required_files":[{"path":"app/app.exe","role":"main_executable"}],
                "projection_rules":[],
                "directories":["data"],
                "entrypoint":{"executable":"app/app.exe","working_directory":"app","arguments":[],"environment_paths":[]},
                "configs":[
                    {
                        "id":"ini","path":"data/config.ini","format":"ini","seed":{"kind":"empty_ini"},
                        "preserve_unmanaged":true,
                        "keys":[
                            {"id":"fullscreen","address":{"kind":"ini","section":"Video","key":"fullscreen"},"scalar_type":"boolean","allowed_values":[false,true]},
                            {"id":"mode","address":{"kind":"ini","section":"General","key":"mode"},"scalar_type":"string","allowed_values":["standard","non-default"]}
                        ]
                    },
                    {
                        "id":"xml","path":"data/config.xml","format":"xml_attributes","seed":{"kind":"xml_root","root":"settings","attributes":[{"name":"version","value":"1"}]},
                        "preserve_unmanaged":true,
                        "keys":[
                            {"id":"scale","address":{"kind":"xml_attribute","elements":["Video"],"attribute":"scale"},"scalar_type":"integer","allowed_values":[1,2]}
                        ]
                    }
                ],
                "persistence":[],
                "build_profiles":[
                    {
                        "id":"default","title":"Default","layer_ids":["runtime"],"controls_profile_ids":["native"],
                        "settings":[
                            {"config_id":"ini","key_id":"fullscreen","value":true},
                            {"config_id":"ini","key_id":"mode","value":"standard"},
                            {"config_id":"xml","key_id":"scale","value":1}
                        ]
                    },
                    {
                        "id":"custom","title":"Custom","layer_ids":["runtime","addon"],"controls_profile_ids":["native"],
                        "settings":[
                            {"config_id":"ini","key_id":"fullscreen","value":false},
                            {"config_id":"ini","key_id":"mode","value":"non-default"},
                            {"config_id":"xml","key_id":"scale","value":2}
                        ]
                    }
                ],
                "controls_profiles":[{"id":"native","title":"Native","kind":"application_defaults"}]
            }
        })).unwrap();
        project_storage::save_project_bytes(
            &mut store,
            &serde_json::to_vec(&project).unwrap(),
            None,
        )
        .unwrap();
        project_storage::save_private_bindings(
            &store,
            1,
            None,
            &CreatorPrivateBindings {
                project_id: "verify-snapshot".into(),
                component_paths: [
                    ("program".into(), runtime_dir.to_str().unwrap().into()),
                    ("addon".into(), addon_dir.to_str().unwrap().into()),
                ]
                .into(),
                signing_key_ref: Some(key.to_str().unwrap().into()),
            },
        )
        .unwrap();
        let runner = production_runner(
            Arc::new(Mutex::new(store)),
            &dir.path().join("creator"),
            Arc::new(|_| {}),
        )
        .unwrap();
        let receipt = runner
            .start(
                CreatorJobKind::Build,
                CreatorJobStartRequest {
                    project_id: "verify-snapshot".into(),
                    project_revision: 1,
                    build_profile_ids: vec!["custom".into()],
                    controls_profile_ids: vec!["native".into()],
                    release_id: None,
                    target_directory: None,
                    source_url: None,
                },
            )
            .unwrap();
        let built = loop {
            let job = runner.get(&receipt.job_id).unwrap();
            if job.progress.status == CreatorJobStatus::Completed {
                break job;
            }
            assert_ne!(job.progress.status, CreatorJobStatus::Failed);
            std::thread::sleep(Duration::from_millis(10));
        };
        let release_id = built.progress.release_id.unwrap();
        let content = runner
            .read_release_content("verify-snapshot", 1, &release_id)
            .unwrap();
        let scratch = dir.path().join("verify-scratch");
        let publication = scratch.join("source");
        std::fs::create_dir_all(&publication).unwrap();
        materialize(&content, &project, &publication, &|| Ok(())).unwrap();

        let library = scratch.join("clean-library");
        let appdata = scratch.join("appdata");
        let local_appdata = scratch.join("localappdata");
        for path in [&library, &appdata, &local_appdata] {
            std::fs::create_dir(path).unwrap();
        }
        let clean_store = RepositoryStore::open(&scratch.join("clean.sqlite")).unwrap();
        let storage = Arc::new(SqliteFusionStorage::new(Arc::new(Mutex::new(clean_store))));
        let transport = Arc::new(OfflineTransport::open_creator(&publication).unwrap());
        let index_url = transport.index_url();
        let service = FusionService::open(
            &library,
            storage,
            transport,
            Arc::new(NoProgress),
            ServiceEnvironment {
                appdata,
                local_appdata,
                display_width: 1920,
                display_height: 1080,
                refresh_rate: 60,
            },
        )
        .unwrap();
        let game = QualifiedGameId {
            source_id: project.source_id.clone(),
            game_id: project.game_id.clone(),
        };
        service.source_add(&index_url).await.unwrap();
        service
            .game_set_options(
                &game,
                GameOptions {
                    build_profile_id: Some("custom".into()),
                    controls_profile_id: Some("native".into()),
                    ..GameOptions::default()
                },
            )
            .await
            .unwrap();
        let installed = service
            .game_install(&game, InstallRequest::default())
            .await
            .unwrap();
        assert_eq!(installed.status, "completed");
        let state = service.game_get_state(&game).await.unwrap();
        let snapshot_id = state.active_snapshot_id.unwrap();
        drop(service);

        verify_clean_install_snapshot(
            &library,
            &game,
            &snapshot_id,
            &project,
            &content,
            "custom",
            "native",
        )
        .unwrap();

        let lease = WorkspaceAssembler::open(&library, &game)
            .unwrap()
            .active()
            .unwrap()
            .unwrap();
        let ini_path = lease.snapshot.workspace_root.join("data/config.ini");
        let xml_path = lease.snapshot.workspace_root.join("data/config.xml");
        let orig_ini = std::fs::read(&ini_path).unwrap();
        let orig_xml = std::fs::read(&xml_path).unwrap();
        let ini_text = std::str::from_utf8(&orig_ini).unwrap();
        let xml_text = std::str::from_utf8(&orig_xml).unwrap();
        assert!(ini_text.contains("fullscreen=false"));
        assert!(ini_text.contains("mode=\"non-default\""));
        assert!(xml_text.contains("scale=\"2\""));
        assert!(lease
            .snapshot
            .workspace_root
            .join("app/addons/extra.dat")
            .is_file());
        drop(lease);

        // Wrong INI setting value fails verification with IntegrityViolation.
        std::fs::write(
            &ini_path,
            b"[Video]\nfullscreen=true\n[General]\nmode=\"standard\"\n",
        )
        .unwrap();
        assert_eq!(
            verify_clean_install_snapshot(
                &library,
                &game,
                &snapshot_id,
                &project,
                &content,
                "custom",
                "native",
            )
            .unwrap_err()
            .code,
            ErrorCode::IntegrityViolation
        );
        std::fs::write(&ini_path, &orig_ini).unwrap();

        // Wrong XML setting value fails verification with IntegrityViolation.
        std::fs::write(
            &xml_path,
            b"<settings version=\"1\"><Video scale=\"1\"/></settings>\n",
        )
        .unwrap();
        assert_eq!(
            verify_clean_install_snapshot(
                &library,
                &game,
                &snapshot_id,
                &project,
                &content,
                "custom",
                "native",
            )
            .unwrap_err()
            .code,
            ErrorCode::IntegrityViolation
        );
        std::fs::write(&xml_path, &orig_xml).unwrap();

        // Mismatched manifest pin fails verification with IntegrityViolation.
        assert_eq!(
            verify_snapshot_with_pin(
                &library,
                &game,
                &snapshot_id,
                &project,
                &"0".repeat(64),
                "custom",
                "native",
            )
            .unwrap_err()
            .code,
            ErrorCode::IntegrityViolation
        );

        // Mismatched expected profile layers fails verification with IntegrityViolation.
        let mut wrong_layers = project.clone();
        wrong_layers.program_descriptor.build_profiles[1].layer_ids = vec!["runtime".into()];
        assert_eq!(
            verify_clean_install_snapshot(
                &library,
                &game,
                &snapshot_id,
                &wrong_layers,
                &content,
                "custom",
                "native",
            )
            .unwrap_err()
            .code,
            ErrorCode::IntegrityViolation
        );
        assert!(
            project_service::release_list(&runner.store().lock().unwrap(), "verify-snapshot")
                .unwrap()
                .len()
                == 1
        );
    }
}
