//! Versioned local workspaces. A completed snapshot is activated by one atomic
//! state-file replacement; an incomplete build never replaces the active version.
//! SQLite and process supervision are deliberately not involved at this stage.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::canonical::{CanonicalGameManifest, QualifiedGameId};
use super::capabilities::{CapabilityRegistry, CapabilitySelection};
use super::cas::{hash_file, CasStore};
use super::config_templates::{ConfigTemplateEngine, TemplateContext};
use super::errors::{ErrorCode, FusionError};
use super::evolution::SchemaUpcaster;
use super::fs_safety::{self, DirectoryGuard, FileIdentity};
use super::layers::{LayerDelta, LayerInput, ResolvedRuntimeLock};
use super::persistence::{
    PersistenceBindings, PersistenceManager, PersistenceRoots, SavePathKind, SaveSnapshot,
};
use super::plan::{
    EnvStep, ExecutionPlan, FetchStep, PlanBuilder, PlanOptions, PlanStep, TransformStep,
    WorkspaceLocation,
};
use super::schema::{DigestAlgorithm, LayerSource, LocalPlacementPolicy, RuntimeChannel};
use super::transforms::{PatchReceipt, PatchToolRegistry};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceSnapshot {
    pub id: String,
    pub game: QualifiedGameId,
    pub workspace_root: PathBuf,
    pub manifest: CanonicalGameManifest,
    pub canonical_blake3: String,
    /// Digest of verified source bytes, separate from normalized canonical JSON.
    /// Older/local-only snapshots have no freshness proof and cannot strict-sync launch.
    #[serde(default)]
    pub source_manifest_blake3: Option<String>,
    pub inputs: BTreeMap<String, LayerInput>,
    pub plan: ExecutionPlan,
    pub persistence: PersistenceBindings,
    pub save_backup: Option<SaveSnapshot>,
    pub patches: Vec<PatchReceipt>,
    pub delta: LayerDelta,
    container_identity: FileIdentity,
    workspace_identity: FileIdentity,
    files: BTreeMap<String, ProjectedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProjectedFile {
    workspace_path: String,
    layer_id: String,
    local_source_relative: Option<String>,
    local_identity: Option<FileIdentity>,
    symlink: bool,
    identity: Option<FileIdentity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SnapshotRef {
    id: String,
    container: PathBuf,
    metadata_blake3: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkspaceState {
    active: Option<SnapshotRef>,
    previous: Option<SnapshotRef>,
    #[serde(default)]
    snapshots: Vec<SnapshotRef>,
}

/// The lease must outlive every process using this workspace. It prevents
/// updates and retains deny-write descriptors for linked external base files,
/// without changing the permissions of a storefront installation.
pub struct AssembledWorkspace {
    pub snapshot: WorkspaceSnapshot,
    library_root: PathBuf,
    _operation_lock: File,
    _local_files: Vec<File>,
    _local_roots: Vec<DirectoryGuard>,
}

impl AssembledWorkspace {
    /// Call only once every process and descendant has stopped writing saves.
    pub fn reconcile_saves(&self) -> Result<(), FusionError> {
        PersistenceManager::open(&self.library_root, &self.snapshot.game)?
            .reconcile_workspace(&self.snapshot.workspace_root, &self.snapshot.persistence)
    }

    /// Always isolate process-local APPDATA, including games which only declared
    /// workspace saves. No process changes the user's global environment.
    pub fn isolated_environment(&self) -> Result<BTreeMap<String, String>, FusionError> {
        let manager = PersistenceManager::open(&self.library_root, &self.snapshot.game)?;
        let mut environment = BTreeMap::new();
        for (name, namespace) in [("APPDATA", "appdata"), ("LOCALAPPDATA", "localappdata")] {
            let path = manager.root().join("data").join(namespace);
            fs_safety::ensure_directory(&path)?;
            environment.insert(name.to_owned(), path.to_string_lossy().into_owned());
        }
        let temporary = manager.root().join("data").join("temp");
        fs_safety::ensure_directory(&temporary)?;
        environment.insert("TEMP".into(), temporary.to_string_lossy().into_owned());
        environment.insert("TMP".into(), temporary.to_string_lossy().into_owned());
        Ok(environment)
    }

    /// Local sanitized telemetry state belongs to this namespaced installation.
    /// This path is never serialized in a crash report or sent to a provider.
    pub(crate) fn telemetry_directory(&self) -> Result<PathBuf, FusionError> {
        let path = self
            .library_root
            .join(".fusion-source")
            .join("telemetry")
            .join(&self.snapshot.game.source_id)
            .join(&self.snapshot.game.game_id);
        fs_safety::ensure_directory(&path)?;
        Ok(path)
    }

    pub(crate) fn verify_execution_contract(&self) -> Result<(), FusionError> {
        SchemaUpcaster::validate_canonical(&self.snapshot.manifest)?;
        if self.snapshot.game.source_id != self.snapshot.manifest.source_id
            || self.snapshot.game.game_id != self.snapshot.manifest.id
            || self.snapshot.plan.game != self.snapshot.game
        {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "The leased execution identity changed.",
            ));
        }
        if canonical_hash(&self.snapshot.manifest)? != self.snapshot.canonical_blake3 {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "The leased manifest changed.",
            ));
        }
        validate_plan_contract(
            &self.snapshot.manifest,
            &self.snapshot.plan,
            &self.snapshot.inputs,
            &self.library_root,
        )
    }
}

impl WorkspaceSnapshot {
    pub(crate) fn verify_directory_identity(&self) -> Result<(), FusionError> {
        let container = self.workspace_root.parent().ok_or_else(|| {
            FusionError::new(
                ErrorCode::InvalidPath,
                "A workspace has no owned container.",
            )
        })?;
        if fs_safety::file_identity(&self.workspace_root)? != self.workspace_identity
            || fs_safety::file_identity(container)? != self.container_identity
        {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "The leased workspace directory was replaced.",
            ));
        }
        Ok(())
    }
    /// Generic file helpers reject links; only recorded, exact local projections
    /// may resolve to a source retained by the workspace lease.
    pub(crate) fn lock_critical_projection(
        &self,
        relative: &str,
    ) -> Result<(PathBuf, Option<File>), FusionError> {
        let path = self
            .workspace_root
            .join(fs_safety::validate_relative_path(relative)?);
        #[cfg(windows)]
        let mut projection_lock = None;
        #[cfg(not(windows))]
        let projection_lock = None;
        if self
            .files
            .get(&relative.to_lowercase())
            .is_some_and(|file| file.symlink)
        {
            #[cfg(windows)]
            {
                use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
                // Pin the known link itself, rather than only its target. The
                // caller retains the parent guard before entering this method.
                let handle = OpenOptions::new()
                    .read(true)
                    .share_mode(1)
                    .custom_flags(0x00200000)
                    .open(&path)
                    .map_err(|error| {
                        fs_safety::io_error("Lock critical local projection", &path, error)
                    })?;
                if handle
                    .metadata()
                    .map_err(|error| fs_safety::io_error("Inspect projection", &path, error))?
                    .file_attributes()
                    & 0x400
                    == 0
                {
                    return Err(FusionError::new(
                        ErrorCode::IntegrityViolation,
                        "A recorded local projection is no longer a file link.",
                    ));
                }
                projection_lock = Some(handle);
            }
        }
        let source =
            resolve_owned_source(&self.workspace_root, relative, &self.files, &self.inputs)?;
        Ok((source, projection_lock))
    }
}

impl std::fmt::Debug for AssembledWorkspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssembledWorkspace")
            .field("snapshot", &self.snapshot)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct VerifiedManifestBinding {
    game: QualifiedGameId,
    canonical_blake3: String,
    raw_blake3: String,
}

impl VerifiedManifestBinding {
    /// The verified fetch/upcast pipeline creates this binding. External hosts
    /// obtain it from VerifiedManifest::snapshot_binding, never from a raw string.
    pub(crate) fn from_verified(
        manifest: &CanonicalGameManifest,
        raw_blake3: &str,
    ) -> Result<Self, FusionError> {
        super::cas::validate_hash(raw_blake3)?;
        Ok(Self {
            game: QualifiedGameId {
                source_id: manifest.source_id.clone(),
                game_id: manifest.id.clone(),
            },
            canonical_blake3: canonical_hash(manifest)?,
            raw_blake3: raw_blake3.to_owned(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceBuildOptions {
    pub patch_timeout: Duration,
    pub verified_manifest: Option<VerifiedManifestBinding>,
    pub cancellation: Option<super::adapters::Cancellation>,
}

impl Default for WorkspaceBuildOptions {
    fn default() -> Self {
        Self {
            patch_timeout: Duration::from_secs(120),
            verified_manifest: None,
            cancellation: None,
        }
    }
}

#[derive(Debug)]
pub struct WorkspaceAssembler {
    library_root: PathBuf,
    game: QualifiedGameId,
    control_root: PathBuf,
    external_root: Option<PathBuf>,
    cas: CasStore,
    patch_tools: PatchToolRegistry,
}

impl WorkspaceAssembler {
    pub fn open(library_root: &Path, game: &QualifiedGameId) -> Result<Self, FusionError> {
        for id in [&game.source_id, &game.game_id] {
            let relative = fs_safety::validate_relative_path(id)?;
            if relative.components().count() != 1 {
                return Err(FusionError::new(
                    ErrorCode::InvalidId,
                    "Game identifiers cannot contain directories.",
                ));
            }
        }
        let _library = fs_safety::ensure_directory(library_root)?;
        let control_root = library_root
            .join(".fusion-source/workspaces")
            .join(&game.source_id)
            .join(&game.game_id);
        let _control = fs_safety::ensure_directory(&control_root.join("snapshots"))?;
        Ok(Self {
            library_root: library_root.to_owned(),
            game: game.clone(),
            control_root,
            external_root: None,
            cas: CasStore::open(library_root)?,
            patch_tools: PatchToolRegistry::default(),
        })
    }

    /// The caller explicitly supplies an approved sibling workspace root on the
    /// located base's volume. Never derive write authority from snapshot JSON.
    pub fn with_external_workspace_root(mut self, root: &Path) -> Result<Self, FusionError> {
        let _root = fs_safety::ensure_directory(root)?;
        self.external_root = Some(
            root.join(&self.game.source_id)
                .join(&self.game.game_id)
                .join("snapshots"),
        );
        fs_safety::ensure_directory(self.external_root.as_ref().expect("configured root"))?;
        Ok(self)
    }

    pub fn with_patch_tools(mut self, tools: PatchToolRegistry) -> Self {
        self.patch_tools = tools;
        self
    }

    pub fn assemble(
        &self,
        manifest: &CanonicalGameManifest,
        plan: &ExecutionPlan,
        inputs: &BTreeMap<String, LayerInput>,
        context: &TemplateContext,
        persistence_roots: &PersistenceRoots,
        options: &WorkspaceBuildOptions,
    ) -> Result<AssembledWorkspace, FusionError> {
        if let Some(cancel) = &options.cancellation {
            cancel.check()?;
        }
        if let Some(binding) = &options.verified_manifest {
            if binding.game != self.game || binding.canonical_blake3 != canonical_hash(manifest)? {
                return Err(FusionError::new(
                    ErrorCode::ManifestDigestMismatch,
                    "The verified raw manifest and canonical assembly input differ.",
                ));
            }
        }
        let operation_lock = self.lock()?;
        self.validate_request(manifest, plan, inputs)?;
        let previous_state = self.state()?;
        let previous = previous_state
            .active
            .as_ref()
            .map(|reference| self.read_snapshot(reference))
            .transpose()?;
        let persistence = PersistenceManager::open(&self.library_root, &self.game)?;
        if let Some(previous) = &previous {
            persistence.reconcile_workspace(&previous.workspace_root, &previous.persistence)?;
        }
        // This completes before any changed layer or workspace file is written.
        let save_backup = if previous.is_some() {
            Some(persistence.snapshot_before_update()?)
        } else {
            None
        };
        let parent = self.workspace_parent(manifest, plan, inputs)?;
        let _parent = fs_safety::ensure_directory(&parent)?;
        let id = fs_safety::unique_name("snapshot");
        let container = parent.join(&id);
        fs::create_dir(&container)
            .map_err(|error| fs_safety::io_error("Create owned snapshot", &container, error))?;
        let root = container.join("workspace");
        let _workspace = fs_safety::ensure_directory(&root)?;
        let mut files = BTreeMap::new();
        let mut patches = Vec::new();
        for id in &plan.selected_layers {
            if let Some(cancel) = &options.cancellation {
                cancel.check()?;
            }
            let layer = &manifest.layers[id];
            let input = &inputs[id];
            if let Some(patch) = &layer.patch {
                let LayerInput::Cas { digest, .. } = input else {
                    return Err(FusionError::new(
                        ErrorCode::InvalidManifest,
                        "Patch payloads must be verified CAS objects.",
                    ));
                };
                let object = self.cas.get(digest)?;
                if object.files.len() != 1 {
                    return Err(FusionError::new(
                        ErrorCode::InvalidManifest,
                        "A patch layer must contain one payload file.",
                    ));
                }
                let patch_file = object.root.join(&object.files[0].path);
                let source = resolve_owned_source(&root, &patch.input_path, &files, inputs)?;
                let destination = root.join(fs_safety::validate_relative_path(&patch.output_path)?);
                // A tool receives private copies, and its output never writes
                // through an existing link, even for an in-place patch.
                let temporary = container.join(fs_safety::unique_name("patch-result"));
                let receipt = self.patch_tools.apply_verified(
                    patch,
                    &source,
                    &patch_file,
                    &temporary,
                    options.patch_timeout,
                )?;
                detach_owned_file(
                    &destination,
                    files.get(&patch.output_path.to_lowercase()),
                    inputs,
                )?;
                fs_safety::copy_file(&temporary, &destination)?;
                fs_safety::unlink_file(&temporary)?;
                patches.push(receipt);
                files.insert(
                    patch.output_path.to_lowercase(),
                    ProjectedFile {
                        workspace_path: patch.output_path.clone(),
                        layer_id: id.clone(),
                        local_source_relative: None,
                        local_identity: None,
                        symlink: false,
                        identity: Some(fs_safety::file_identity(&destination)?),
                    },
                );
                continue;
            }
            let (source_root, source_files, local) = match input {
                LayerInput::Cas { digest, .. } => {
                    let object = self.cas.get(digest)?;
                    (
                        object.root,
                        object
                            .files
                            .into_iter()
                            .map(|file| file.path)
                            .collect::<Vec<_>>(),
                        false,
                    )
                }
                LayerInput::LocalDirectory { root } => {
                    let _source = fs_safety::guard(root)?;
                    verify_locator_keys(layer, root)?;
                    (
                        root.clone(),
                        fs_safety::list_plain_files(root)?
                            .into_iter()
                            .map(|(path, _)| path)
                            .collect(),
                        true,
                    )
                }
            };
            let _source_root = fs_safety::guard(&source_root)?;
            for relative in source_files {
                if let Some(cancel) = &options.cancellation {
                    cancel.check()?;
                }
                let source = source_root.join(fs_safety::validate_relative_path(&relative)?);
                let workspace_path = super::evolution::mounted_path(&layer.mount_path, &relative);
                let target = root.join(fs_safety::validate_relative_path(&workspace_path)?);
                let _target_parent =
                    fs_safety::ensure_directory(target.parent().expect("validated target parent"))?;
                if fs::symlink_metadata(&target).is_ok() {
                    let recorded = files.get(&workspace_path.to_lowercase()).ok_or_else(|| {
                        FusionError::new(
                            ErrorCode::SnapshotConflict,
                            "Refusing to replace an unrecorded workspace file.",
                        )
                    })?;
                    detach_owned_file(&target, Some(recorded), inputs)?;
                }
                let mutable = matches_writable(&relative, &layer.writable_copy_paths)?
                    || manifest
                        .config_templates
                        .iter()
                        .any(|template| template.path.eq_ignore_ascii_case(&workspace_path))
                    || manifest
                        .persistence
                        .saves_paths
                        .iter()
                        .filter_map(|path| path.strip_prefix("workspace/"))
                        .any(|path| path_is_inside(&workspace_path, path));
                let same_volume = fs_safety::same_volume(&source_root, &root)?;
                let mut symlink = false;
                if mutable {
                    fs_safety::copy_file(&source, &target)?;
                } else if same_volume {
                    fs::hard_link(&source, &target).map_err(|error| {
                        fs_safety::io_error("Project immutable hardlink", &target, error)
                    })?;
                } else if local
                    && manifest.local_placement == LocalPlacementPolicy::CrossVolumeFallback
                {
                    create_local_file_link(&source, &target)?;
                    symlink = true;
                } else if !local && plan.placement.location == WorkspaceLocation::LocatedBaseVolume
                {
                    // Explicit placement moves the workspace, not the library CAS.
                    // Only these non-base managed layers use the declared fallback.
                    fs_safety::copy_file(&source, &target)?;
                    let mut permissions = fs::metadata(&target)
                        .map_err(|error| {
                            fs_safety::io_error("Inspect projected copy", &target, error)
                        })?
                        .permissions();
                    permissions.set_readonly(true);
                    fs::set_permissions(&target, permissions).map_err(|error| {
                        fs_safety::io_error("Protect projected copy", &target, error)
                    })?;
                } else {
                    return Err(FusionError::new(
                        ErrorCode::CrossVolumeLinkNotSupported,
                        "This projection would require an undeclared cross-volume copy.",
                    )
                    .with_detail("layerId", id));
                }
                files.insert(
                    workspace_path.to_lowercase(),
                    ProjectedFile {
                        workspace_path,
                        layer_id: id.clone(),
                        local_source_relative: (local && !mutable).then_some(relative),
                        local_identity: if local && !mutable {
                            Some(fs_safety::file_identity(&source)?)
                        } else {
                            None
                        },
                        symlink,
                        identity: if symlink {
                            None
                        } else {
                            Some(fs_safety::file_identity(&target)?)
                        },
                    },
                );
            }
        }
        let bindings = persistence.prepare(&root, &manifest.persistence, persistence_roots)?;
        let mut template_context = context.clone();
        template_context.workspace_dir = root.clone();
        template_context.enabled_addons = plan
            .selected_layers
            .iter()
            .filter(|id| manifest.layers[*id].optional)
            .cloned()
            .collect();
        if let Some(path) = bindings.env_overrides.get("APPDATA") {
            template_context.appdata_dir = PathBuf::from(path);
        }
        if let Some(path) = bindings.env_overrides.get("LOCALAPPDATA") {
            template_context.local_appdata_dir = PathBuf::from(path);
        }
        let mut templates = Vec::new();
        for step in &plan.steps {
            match step {
                PlanStep::Env(EnvStep::RenderConfigTemplate { template, .. }) => {
                    if !manifest.config_templates.contains(template) {
                        return Err(FusionError::new(
                            ErrorCode::InvalidManifest,
                            "Plan contains a foreign config template.",
                        ));
                    }
                    templates.push(template.clone());
                }
                PlanStep::Env(EnvStep::WasmTransform { .. }) => {
                    return Err(FusionError::new(
                        ErrorCode::MissingCapability,
                        "Stage 2 does not execute Wasm extensions.",
                    ))
                }
                _ => {}
            }
        }
        for template in &templates {
            if files
                .get(&template.path.to_lowercase())
                .is_some_and(|entry| entry.symlink)
            {
                detach_owned_file(
                    &root.join(&template.path),
                    files.get(&template.path.to_lowercase()),
                    inputs,
                )?;
            }
        }
        for path in ConfigTemplateEngine::render_to_workspace(&templates, &template_context)? {
            files.insert(
                path.to_lowercase(),
                ProjectedFile {
                    workspace_path: path.clone(),
                    layer_id: String::new(),
                    local_source_relative: None,
                    local_identity: None,
                    symlink: false,
                    identity: Some(fs_safety::file_identity(&root.join(&path))?),
                },
            );
        }
        persistence.bind_workspace(&root, &bindings)?;
        // Persistent paths are deliberately not immutable local-base leases.
        files.retain(|_, file| {
            !bindings
                .bindings
                .iter()
                .filter_map(|binding| binding.workspace_relative.as_ref())
                .any(|path| path_is_inside(&file.workspace_path, path))
        });
        let used_inputs = plan
            .selected_layers
            .iter()
            .map(|id| (id.clone(), inputs[id].clone()))
            .collect();
        let old_inputs = previous
            .as_ref()
            .map(|snapshot| &snapshot.inputs)
            .cloned()
            .unwrap_or_default();
        let snapshot = WorkspaceSnapshot {
            id: id.clone(),
            game: self.game.clone(),
            workspace_root: root.clone(),
            manifest: manifest.clone(),
            canonical_blake3: canonical_hash(manifest)?,
            source_manifest_blake3: options
                .verified_manifest
                .as_ref()
                .map(|binding| binding.raw_blake3.clone()),
            inputs: used_inputs,
            plan: plan.clone(),
            persistence: bindings,
            save_backup,
            patches,
            delta: LayerDelta::between(
                &old_inputs,
                &plan
                    .selected_layers
                    .iter()
                    .map(|id| (id.clone(), inputs[id].clone()))
                    .collect(),
            ),
            files,
            container_identity: fs_safety::file_identity(&container)?,
            workspace_identity: fs_safety::file_identity(&root)?,
        };
        let (local_files, local_roots) = self.lease_local_files(&snapshot)?;
        verify_critical_files(&snapshot)?;
        let metadata = serde_json::to_vec(&snapshot)
            .map_err(|error| FusionError::new(ErrorCode::InvalidJson, error.to_string()))?;
        fs_safety::atomic_write(&container.join("snapshot.json"), &metadata)?;
        let reference = SnapshotRef {
            id,
            container,
            metadata_blake3: blake3::hash(&metadata).to_hex().to_string(),
        };
        let mut history = previous_state.snapshots;
        history.push(reference.clone());
        // Final cancellation boundary: a cancelled candidate never replaces
        // the previously active snapshot. Closed writers are retained safely.
        if let Some(cancel) = &options.cancellation {
            cancel.check()?;
        }
        fs_safety::atomic_write_json(
            &self.control_root.join("state.json"),
            &WorkspaceState {
                active: Some(reference),
                previous: previous_state.active,
                snapshots: history,
            },
        )?;
        Ok(AssembledWorkspace {
            snapshot,
            library_root: self.library_root.clone(),
            _operation_lock: operation_lock,
            _local_files: local_files,
            _local_roots: local_roots,
        })
    }

    /// Prunes only a recorded, inactive snapshot after checking its exact file
    /// inventory. Unknown files, replaced entries and foreign links are preserved.
    pub fn cleanup_snapshot(&self, id: &str) -> Result<(), FusionError> {
        let _lock = self.lock()?;
        let mut state = self.state()?;
        if state
            .active
            .as_ref()
            .is_some_and(|reference| reference.id == id)
            || state
                .previous
                .as_ref()
                .is_some_and(|reference| reference.id == id)
        {
            return Err(FusionError::new(
                ErrorCode::SnapshotConflict,
                "Active and rollback snapshots are pinned.",
            ));
        }
        let reference = state
            .snapshots
            .iter()
            .find(|reference| reference.id == id)
            .cloned()
            .ok_or_else(|| {
                FusionError::new(
                    ErrorCode::SnapshotNotFound,
                    "Snapshot has no trusted local ownership record.",
                )
            })?;
        let snapshot = self.read_snapshot(&reference)?;
        let _parent =
            fs_safety::guard(reference.container.parent().expect("owned snapshot parent"))?;
        let mut directories = Vec::new();
        check_cleanup_inventory(&snapshot, &snapshot.workspace_root, "", &mut directories)?;
        for entry in fs::read_dir(&reference.container).map_err(|error| {
            fs_safety::io_error("Inspect snapshot container", &reference.container, error)
        })? {
            let name = entry
                .map_err(|error| {
                    fs_safety::io_error("Inspect snapshot child", &reference.container, error)
                })?
                .file_name();
            if name != "workspace" && name != "snapshot.json" {
                return Err(FusionError::new(
                    ErrorCode::SnapshotConflict,
                    "Unrecorded snapshot-container content is preserved.",
                ));
            }
        }
        let persistence = PersistenceManager::open(&self.library_root, &self.game)?;
        // File saves may have detached by atomic rename while this version was
        // active. Its current data was reconciled before the subsequent update;
        // rebind to the current persistent slot before unlinking this old view.
        persistence.bind_workspace(&snapshot.workspace_root, &snapshot.persistence)?;
        persistence.unbind_workspace(&snapshot.workspace_root, &snapshot.persistence)?;
        for entry in snapshot.files.values() {
            let path = snapshot
                .workspace_root
                .join(fs_safety::validate_relative_path(&entry.workspace_path)?);
            if fs::symlink_metadata(&path).is_ok() {
                detach_owned_file(&path, Some(entry), &snapshot.inputs)?;
            }
        }
        directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        for directory in directories {
            if directory.try_exists().map_err(|error| {
                fs_safety::io_error("Inspect owned directory", &directory, error)
            })? {
                fs::remove_dir(&directory).map_err(|error| {
                    fs_safety::io_error("Remove empty owned directory", &directory, error)
                })?;
            }
        }
        fs::remove_dir(&snapshot.workspace_root).map_err(|error| {
            fs_safety::io_error("Remove empty workspace", &snapshot.workspace_root, error)
        })?;
        fs_safety::unlink_file(&reference.container.join("snapshot.json"))?;
        fs::remove_dir(&reference.container).map_err(|error| {
            fs_safety::io_error("Remove empty snapshot", &reference.container, error)
        })?;
        state.snapshots.retain(|reference| reference.id != id);
        fs_safety::atomic_write_json(&self.control_root.join("state.json"), &state)?;
        Ok(())
    }

    /// Deactivate before pruning. Saves/CAS are outside snapshot cleanup;
    /// unknown or changed files leave their snapshot owned and retained.
    pub fn remove_installation(&self) -> Result<Vec<String>, FusionError> {
        let lock = self.lock()?;
        let mut state = self.state()?;
        let snapshots: Vec<_> = state
            .snapshots
            .iter()
            .map(|snapshot| snapshot.id.clone())
            .collect();
        state.active = None;
        state.previous = None;
        fs_safety::atomic_write_json(&self.control_root.join("state.json"), &state)?;
        drop(lock);
        let mut retained = Vec::new();
        for id in snapshots {
            if self.cleanup_snapshot(&id).is_err() {
                retained.push(id);
            }
        }
        Ok(retained)
    }

    pub fn active(&self) -> Result<Option<AssembledWorkspace>, FusionError> {
        let lock = self.lock()?;
        let state = self.state()?;
        state
            .active
            .as_ref()
            .map(|reference| self.open_lease(reference, lock, true))
            .transpose()
    }

    /// Trusted build metadata for repair/update, never launch authorization.
    /// Snapshot address, directory ownership, manifest and CAS structure are
    /// validated under the operation lock; projected critical files may be
    /// missing or damaged because the new workspace is intended to repair them.
    pub(crate) fn snapshot_for_update(&self) -> Result<Option<WorkspaceSnapshot>, FusionError> {
        let _lock = self.lock()?;
        let state = self.state()?;
        state
            .active
            .as_ref()
            .map(|reference| self.read_snapshot(reference))
            .transpose()
    }

    /// Inspect the active state while retaining the per-game lock through the
    /// callback, including an empty state. Database recovery can therefore never
    /// publish stale activation/removal metadata after another build switches it.
    pub(crate) fn inspect_active<T>(
        &self,
        inspect: impl FnOnce(Option<&AssembledWorkspace>, bool) -> Result<T, FusionError>,
    ) -> Result<T, FusionError> {
        let lock = self.lock()?;
        let state = self.state()?;
        let can_rollback = state
            .previous
            .as_ref()
            .is_some_and(|reference| self.read_snapshot(reference).is_ok());
        if let Some(reference) = state.active.as_ref() {
            let lease = self.open_lease(reference, lock, true)?;
            inspect(Some(&lease), can_rollback)
        } else {
            let result = inspect(None, can_rollback);
            drop(lock);
            result
        }
    }

    /// Binary rollback only swaps existing validated snapshots. Saves are never
    /// restored implicitly, and a dynamic provider is never queried again.
    pub fn rollback(&self) -> Result<AssembledWorkspace, FusionError> {
        let lock = self.lock()?;
        let state = self.state()?;
        let target = state.previous.as_ref().ok_or_else(|| {
            FusionError::new(
                ErrorCode::SnapshotNotFound,
                "No previous workspace snapshot is available.",
            )
        })?;
        let persistence = PersistenceManager::open(&self.library_root, &self.game)?;
        if let Some(active) = &state.active {
            let current = self.read_snapshot(active)?;
            persistence.reconcile_workspace(&current.workspace_root, &current.persistence)?;
        }
        let lease = self.open_lease(target, lock, false)?;
        fs_safety::atomic_write_json(
            &self.control_root.join("state.json"),
            &WorkspaceState {
                active: Some(target.clone()),
                previous: state.active,
                snapshots: state.snapshots,
            },
        )?;
        Ok(lease)
    }

    fn open_lease(
        &self,
        reference: &SnapshotRef,
        lock: File,
        reconcile: bool,
    ) -> Result<AssembledWorkspace, FusionError> {
        let snapshot = self.read_snapshot(reference)?;
        let persistence = PersistenceManager::open(&self.library_root, &self.game)?;
        if reconcile {
            persistence.reconcile_workspace(&snapshot.workspace_root, &snapshot.persistence)?;
        }
        persistence.bind_workspace(&snapshot.workspace_root, &snapshot.persistence)?;
        let (files, roots) = self.lease_local_files(&snapshot)?;
        verify_critical_files(&snapshot)?;
        Ok(AssembledWorkspace {
            snapshot,
            library_root: self.library_root.clone(),
            _operation_lock: lock,
            _local_files: files,
            _local_roots: roots,
        })
    }

    fn validate_request(
        &self,
        manifest: &CanonicalGameManifest,
        plan: &ExecutionPlan,
        inputs: &BTreeMap<String, LayerInput>,
    ) -> Result<(), FusionError> {
        SchemaUpcaster::validate_canonical(manifest)?;
        if manifest.source_id != self.game.source_id
            || manifest.id != self.game.game_id
            || plan.game != self.game
            || plan.manifest_version != manifest.version
        {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Workspace identity differs from the manifest or plan.",
            ));
        }
        if plan.negotiation.is_blocked() {
            return Err(FusionError::new(
                ErrorCode::MissingCapability,
                "A blocked plan cannot be assembled.",
            ));
        }
        let selected: BTreeSet<_> = plan.selected_layers.iter().cloned().collect();
        let expected: Vec<_> = manifest
            .layer_order
            .iter()
            .filter(|id| selected.contains(*id))
            .cloned()
            .collect();
        if selected.len() != plan.selected_layers.len()
            || expected != plan.selected_layers
            || manifest
                .layers
                .values()
                .any(|layer| !layer.optional && !selected.contains(&layer.id))
        {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Selected layer order or required layers are invalid.",
            ));
        }
        for id in &plan.selected_layers {
            let layer = manifest.layers.get(id).ok_or_else(|| {
                FusionError::new(ErrorCode::MissingLayer, "Unknown selected layer.")
            })?;
            if layer.depends_on.iter().any(|id| !selected.contains(id)) {
                return Err(FusionError::new(
                    ErrorCode::MissingLayer,
                    "Selected layer dependency is disabled.",
                ));
            }
            let input = inputs.get(id).ok_or_else(|| {
                FusionError::new(
                    ErrorCode::MissingLayer,
                    "A layer must be acquired before local assembly.",
                )
                .with_detail("layerId", id)
            })?;
            match (&layer.source, input) {
                (LayerSource::LocalLocator { .. }, LayerInput::LocalDirectory { .. }) => {}
                (LayerSource::LocalLocator { .. }, _) | (_, LayerInput::LocalDirectory { .. }) => {
                    return Err(FusionError::new(
                        ErrorCode::InvalidManifest,
                        "Layer input does not match its source kind.",
                    ))
                }
                (
                    _,
                    LayerInput::Cas {
                        digest,
                        runtime_lock,
                    },
                ) => {
                    let object = self.cas.get(digest)?;
                    let dynamic = manifest
                        .runtime
                        .as_ref()
                        .is_some_and(|runtime| runtime.layer_id == *id)
                        && plan.runtime_channel == Some(RuntimeChannel::Dynamic);
                    if dynamic {
                        let lock = runtime_lock.as_ref().ok_or_else(|| {
                            FusionError::new(
                                ErrorCode::InvalidRuntimeChannel,
                                "Dynamic runtime assembly requires its resolved release lock.",
                            )
                        })?;
                        validate_runtime_lock(layer, lock, &object)?;
                    } else if runtime_lock.is_some() {
                        return Err(FusionError::new(
                            ErrorCode::InvalidRuntimeChannel,
                            "A pinned layer cannot substitute a dynamic runtime lock.",
                        ));
                    }
                }
            }
        }
        super::evolution::validate_selected_patch_integrity(manifest, &plan.selected_layers)?;
        validate_plan_contract(manifest, plan, inputs, &self.library_root)?;
        Ok(())
    }

    fn workspace_parent(
        &self,
        manifest: &CanonicalGameManifest,
        plan: &ExecutionPlan,
        inputs: &BTreeMap<String, LayerInput>,
    ) -> Result<PathBuf, FusionError> {
        let local = plan
            .selected_layers
            .iter()
            .filter_map(|id| inputs.get(id))
            .find_map(|input| match input {
                LayerInput::LocalDirectory { root } => Some(root),
                _ => None,
            });
        if let Some(root) = local {
            fs_safety::guard(root)?;
            let same = fs_safety::same_volume(root, &self.library_root)?;
            if !same && manifest.local_placement == LocalPlacementPolicy::RequireSameVolume {
                return Err(FusionError::new(
                    ErrorCode::CrossVolumeLinkNotSupported,
                    "The actual local-base volume differs from library_root.",
                ));
            }
            if manifest.local_placement == LocalPlacementPolicy::WorkspaceOnBaseVolume {
                let target = self.external_root.as_ref().ok_or_else(|| FusionError::new(ErrorCode::ProjectionUnavailable,
                    "Workspace-on-base-volume requires an explicitly configured sibling workspace root."))?;
                if !fs_safety::same_volume(root, target)? || paths_overlap(root, target) {
                    return Err(FusionError::new(ErrorCode::InvalidPath,"External workspace must be on the base volume, outside the installed game."));
                }
                return Ok(target.clone());
            }
        }
        if plan.placement.location != WorkspaceLocation::LibraryRoot {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Workspace placement does not match its local-base policy.",
            ));
        }
        Ok(self.control_root.join("snapshots"))
    }

    fn lease_local_files(
        &self,
        snapshot: &WorkspaceSnapshot,
    ) -> Result<(Vec<File>, Vec<DirectoryGuard>), FusionError> {
        let mut handles = Vec::new();
        let mut roots = Vec::new();
        for (id, input) in &snapshot.inputs {
            if let LayerInput::LocalDirectory { root } = input {
                roots.push(fs_safety::guard(root)?);
                if let LayerSource::LocalLocator { locator } = &snapshot.manifest.layers[id].source
                {
                    for key in &locator.key_files {
                        handles.push(fs_safety::open_read(
                            &root.join(fs_safety::validate_relative_path(&key.path)?),
                        )?);
                    }
                    verify_locator_keys(&snapshot.manifest.layers[id], root)?;
                }
            }
        }
        for entry in snapshot.files.values() {
            let Some(relative) = &entry.local_source_relative else {
                continue;
            };
            let Some(LayerInput::LocalDirectory { root }) = snapshot.inputs.get(&entry.layer_id)
            else {
                return Err(FusionError::new(
                    ErrorCode::IntegrityViolation,
                    "Local workspace lease has no base binding.",
                ));
            };
            let source = root.join(fs_safety::validate_relative_path(relative)?);
            let target = snapshot
                .workspace_root
                .join(fs_safety::validate_relative_path(&entry.workspace_path)?);
            handles.push(fs_safety::open_read(&source)?);
            if Some(fs_safety::file_identity(&source)?) != entry.local_identity {
                return Err(FusionError::new(
                    ErrorCode::IntegrityViolation,
                    "The located base changed since workspace assembly.",
                ));
            }
            if entry.symlink {
                if fs::read_link(&target)
                    .map_err(|error| fs_safety::io_error("Read local projection", &target, error))?
                    != source
                {
                    return Err(FusionError::new(
                        ErrorCode::IntegrityViolation,
                        "Local projection target changed.",
                    ));
                }
            } else if fs_safety::file_identity(&target)? != fs_safety::file_identity(&source)? {
                return Err(FusionError::new(
                    ErrorCode::IntegrityViolation,
                    "Local hardlink identity changed.",
                ));
            }
        }
        Ok((handles, roots))
    }

    fn state(&self) -> Result<WorkspaceState, FusionError> {
        let path = self.control_root.join("state.json");
        if !path
            .try_exists()
            .map_err(|error| fs_safety::io_error("Inspect workspace state", &path, error))?
        {
            return Ok(WorkspaceState::default());
        }
        read_json(&path)
    }

    fn read_snapshot(&self, reference: &SnapshotRef) -> Result<WorkspaceSnapshot, FusionError> {
        fs_safety::validate_relative_path(&reference.id)?;
        let allowed =
            std::iter::once(self.control_root.join("snapshots")).chain(self.external_root.clone());
        if !allowed
            .into_iter()
            .any(|parent| reference.container == parent.join(&reference.id))
        {
            return Err(FusionError::new(
                ErrorCode::InvalidPath,
                "Snapshot location is not an approved owned root.",
            ));
        }
        let _container = fs_safety::guard(&reference.container)?;
        let path = reference.container.join("snapshot.json");
        if hash_file(&path, DigestAlgorithm::Blake3)? != reference.metadata_blake3 {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "Snapshot metadata changed.",
            ));
        }
        let snapshot: WorkspaceSnapshot = read_json(&path)?;
        if let Some(digest) = &snapshot.source_manifest_blake3 {
            super::cas::validate_hash(digest)?;
        }
        if fs_safety::file_identity(&reference.container)? != snapshot.container_identity
            || fs_safety::file_identity(&snapshot.workspace_root)? != snapshot.workspace_identity
        {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "Snapshot directory identity changed.",
            ));
        }
        if snapshot.id != reference.id
            || snapshot.game != self.game
            || snapshot.workspace_root != reference.container.join("workspace")
            || canonical_hash(&snapshot.manifest)? != snapshot.canonical_blake3
        {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "Snapshot metadata identity is invalid.",
            ));
        }
        self.validate_request(&snapshot.manifest, &snapshot.plan, &snapshot.inputs)?;
        Ok(snapshot)
    }

    fn lock(&self) -> Result<File, FusionError> {
        let _parent = fs_safety::guard(&self.control_root)?;
        let path = self.control_root.join("operation.lock");
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            fs_safety::check_metadata(&metadata, &path)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(3).custom_flags(0x00200000);
        }
        let file = options
            .open(&path)
            .map_err(|error| fs_safety::io_error("Open workspace lock", &path, error))?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|_| {
            FusionError::new(
                ErrorCode::OperationInProgress,
                "A workspace lease or update is already active.",
            )
        })?;
        Ok(file)
    }
}

fn detach_owned_file(
    destination: &Path,
    recorded: Option<&ProjectedFile>,
    inputs: &BTreeMap<String, LayerInput>,
) -> Result<(), FusionError> {
    let _parent =
        fs_safety::ensure_directory(destination.parent().expect("validated destination"))?;
    if fs::symlink_metadata(destination).is_ok() {
        let entry = recorded.ok_or_else(|| {
            FusionError::new(
                ErrorCode::SnapshotConflict,
                "Refusing to replace an unrecorded workspace file.",
            )
        })?;
        if entry.symlink {
            let source = recorded_local_source(entry, inputs)?;
            if fs::read_link(destination).map_err(|error| {
                fs_safety::io_error("Inspect owned local link", destination, error)
            })? != source
            {
                return Err(FusionError::new(
                    ErrorCode::IntegrityViolation,
                    "Owned local link was replaced.",
                ));
            }
            fs::remove_file(destination).map_err(|error| {
                fs_safety::io_error("Unlink local projection", destination, error)
            })?;
        } else {
            if Some(fs_safety::file_identity(destination)?) != entry.identity {
                return Err(FusionError::new(
                    ErrorCode::SnapshotConflict,
                    "Workspace file identity changed before replacement.",
                ));
            }
            fs_safety::unlink_file(destination)?;
        }
    }
    Ok(())
}

fn recorded_local_source(
    entry: &ProjectedFile,
    inputs: &BTreeMap<String, LayerInput>,
) -> Result<PathBuf, FusionError> {
    let Some(LayerInput::LocalDirectory { root }) = inputs.get(&entry.layer_id) else {
        return Err(FusionError::new(
            ErrorCode::IntegrityViolation,
            "Local link has no owned base binding.",
        ));
    };
    let relative = entry.local_source_relative.as_ref().ok_or_else(|| {
        FusionError::new(
            ErrorCode::IntegrityViolation,
            "Local link has no source path.",
        )
    })?;
    Ok(root.join(fs_safety::validate_relative_path(relative)?))
}

fn resolve_owned_source(
    root: &Path,
    relative: &str,
    files: &BTreeMap<String, ProjectedFile>,
    inputs: &BTreeMap<String, LayerInput>,
) -> Result<PathBuf, FusionError> {
    let path = root.join(fs_safety::validate_relative_path(relative)?);
    if let Some(entry) = files
        .get(&relative.to_lowercase())
        .filter(|entry| entry.symlink)
    {
        let source = recorded_local_source(entry, inputs)?;
        if fs::read_link(&path)
            .map_err(|error| fs_safety::io_error("Inspect patch source link", &path, error))?
            != source
        {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "Patch source link target changed.",
            ));
        }
        Ok(source)
    } else {
        Ok(path)
    }
}

fn validate_plan_contract(
    manifest: &CanonicalGameManifest,
    plan: &ExecutionPlan,
    inputs: &BTreeMap<String, LayerInput>,
    library: &Path,
) -> Result<(), FusionError> {
    if plan.steps.iter().any(|step| {
        matches!(
            step,
            PlanStep::Transform(TransformStep::MountOverlay { .. })
                | PlanStep::Env(EnvStep::WasmTransform { .. })
        )
    }) {
        return Err(FusionError::new(
            ErrorCode::MissingCapability,
            "The local executor does not implement overlay mounts or Wasm.",
        ));
    }
    // Reconstruct the planner's feature context for contract comparison only.
    // This never advertises networking/process adapters in the current client.
    let mut features: BTreeSet<String> = [
        "core.schema.v1",
        "core.plan.v1",
        "core.layers.v1",
        "vfs.hardlink.v1",
        "core.config_templates.v1",
        "persistence.saves.v1",
        "proc.supervisor.v1",
        "local_locator.v1",
        "patch.xdelta3.v1",
        "patch.bsdiff.v1",
        "provider.github_releases.v1",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for (capability, decision) in &plan.negotiation.decisions {
        if *decision == CapabilitySelection::Preferred {
            features.insert(capability.clone());
        } else {
            features.remove(capability);
        }
    }
    for step in &plan.steps {
        if let PlanStep::Fetch(FetchStep::AcquireLayer {
            source: LayerSource::Remote { mirrors, .. },
            ..
        }) = step
        {
            for uri in mirrors {
                if let Some((scheme, _)) = uri.split_once(':') {
                    features.insert(format!("resolver.{}.v1", scheme.to_lowercase()));
                }
            }
        }
    }
    let local = plan
        .selected_layers
        .iter()
        .filter_map(|id| inputs.get(id))
        .find_map(|input| match input {
            LayerInput::LocalDirectory { root } => Some(root),
            _ => None,
        });
    let (library_volume, local_base_volume) = if plan.placement.requires_volume_check {
        (None, None)
    } else {
        (
            Some(fs_safety::file_identity(library)?.volume.to_string()),
            local
                .map(|root| fs_safety::file_identity(root).map(|id| id.volume.to_string()))
                .transpose()?,
        )
    };
    let rebuilt = PlanBuilder::build(
        manifest,
        &PlanOptions {
            enabled_addons: Some(
                plan.selected_layers
                    .iter()
                    .filter(|id| manifest.layers[*id].optional)
                    .cloned()
                    .collect(),
            ),
            runtime_channel: plan.runtime_channel,
            library_volume,
            local_base_volume,
        },
        &CapabilityRegistry::new(features),
    )?;
    if rebuilt.selected_layers != plan.selected_layers
        || rebuilt.steps != plan.steps
        || rebuilt.placement != plan.placement
        || rebuilt.runtime_channel != plan.runtime_channel
        || rebuilt.negotiation.decisions != plan.negotiation.decisions
        || plan.negotiation.game != plan.game
    {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Execution plan differs from its validated manifest and selected options.",
        ));
    }
    Ok(())
}

fn check_cleanup_inventory(
    snapshot: &WorkspaceSnapshot,
    directory: &Path,
    prefix: &str,
    directories: &mut Vec<PathBuf>,
) -> Result<(), FusionError> {
    let _directory = fs_safety::guard(directory)?;
    for entry in fs::read_dir(directory)
        .map_err(|error| fs_safety::io_error("Read owned workspace", directory, error))?
    {
        let entry = entry
            .map_err(|error| fs_safety::io_error("Read owned workspace entry", directory, error))?;
        let name = entry.file_name().into_string().map_err(|_| {
            FusionError::new(
                ErrorCode::InvalidPath,
                "Non-Unicode foreign entry is preserved.",
            )
        })?;
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        fs_safety::validate_relative_path(&relative)?;
        if let Some(binding) = snapshot.persistence.bindings.iter().find(|binding| {
            binding
                .workspace_relative
                .as_ref()
                .is_some_and(|path| path.eq_ignore_ascii_case(&relative))
        }) {
            if binding.kind == SavePathKind::Directory {
                let target = fs::read_link(entry.path()).map_err(|_| {
                    FusionError::new(
                        ErrorCode::SnapshotConflict,
                        "A replaced save directory is preserved.",
                    )
                })?;
                let expected = fs::canonicalize(&binding.persistent_path).map_err(|error| {
                    fs_safety::io_error(
                        "Inspect persistent save target",
                        &binding.persistent_path,
                        error,
                    )
                })?;
                if target != expected && fs::canonicalize(target).ok().as_ref() != Some(&expected) {
                    return Err(FusionError::new(
                        ErrorCode::SnapshotConflict,
                        "A foreign save junction is preserved.",
                    ));
                }
            }
            continue; // The persistence manager validates and unlinks this binding.
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| fs_safety::io_error("Inspect owned entry", &path, error))?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            fs_safety::check_metadata(&metadata, &path)?;
            let known = snapshot
                .files
                .values()
                .any(|file| path_is_inside(&file.workspace_path, &relative))
                || snapshot
                    .persistence
                    .bindings
                    .iter()
                    .filter_map(|binding| binding.workspace_relative.as_ref())
                    .any(|path| path_is_inside(path, &relative));
            if !known {
                return Err(FusionError::new(
                    ErrorCode::SnapshotConflict,
                    "Unknown directory is preserved.",
                ));
            }
            check_cleanup_inventory(snapshot, &path, &relative, directories)?;
            directories.push(path);
        } else {
            let file = snapshot
                .files
                .get(&relative.to_lowercase())
                .ok_or_else(|| {
                    FusionError::new(ErrorCode::SnapshotConflict, "Unknown file is preserved.")
                })?;
            if file.symlink {
                if fs::read_link(&path).map_err(|error| {
                    fs_safety::io_error("Inspect owned local link", &path, error)
                })? != recorded_local_source(file, &snapshot.inputs)?
                {
                    return Err(FusionError::new(
                        ErrorCode::SnapshotConflict,
                        "Replaced local link is preserved.",
                    ));
                }
            } else if Some(fs_safety::file_identity(&path)?) != file.identity {
                return Err(FusionError::new(
                    ErrorCode::SnapshotConflict,
                    "Replaced file is preserved.",
                ));
            }
        }
    }
    Ok(())
}

fn canonical_hash(manifest: &CanonicalGameManifest) -> Result<String, FusionError> {
    let bytes = serde_json::to_vec(manifest)
        .map_err(|error| FusionError::new(ErrorCode::InvalidJson, error.to_string()))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, FusionError> {
    let file = fs_safety::open_read(path)?;
    let mut bytes = Vec::new();
    file.take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| fs_safety::io_error("Read metadata", path, error))?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(FusionError::new(
            ErrorCode::InvalidJson,
            "Snapshot metadata exceeds its size limit.",
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| FusionError::new(ErrorCode::InvalidJson, error.to_string()))
}

fn matches_writable(relative: &str, patterns: &[String]) -> Result<bool, FusionError> {
    for pattern in patterns {
        let matcher = glob::Pattern::new(pattern)
            .map_err(|error| FusionError::new(ErrorCode::InvalidPath, error.to_string()))?;
        if matcher.matches_with(
            relative,
            glob::MatchOptions {
                case_sensitive: false,
                require_literal_separator: true,
                require_literal_leading_dot: false,
            },
        ) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn verify_locator_keys(
    layer: &super::canonical::CanonicalLayer,
    root: &Path,
) -> Result<(), FusionError> {
    if let LayerSource::LocalLocator { locator } = &layer.source {
        for file in &locator.key_files {
            let path = root.join(fs_safety::validate_relative_path(&file.path)?);
            let actual = hash_file(&path, DigestAlgorithm::Blake3).map_err(|_| {
                FusionError::new(
                    ErrorCode::LocalBaseModified,
                    "A local base key file is unavailable.",
                )
                .with_detail("path", &file.path)
            })?;
            if actual != file.blake3 {
                return Err(FusionError::new(
                    ErrorCode::LocalBaseModified,
                    "The located game does not match its key-file hashes.",
                )
                .with_detail("path", &file.path));
            }
        }
    }
    Ok(())
}

fn validate_runtime_lock(
    layer: &super::canonical::CanonicalLayer,
    lock: &ResolvedRuntimeLock,
    object: &super::cas::CasObject,
) -> Result<(), FusionError> {
    let asset_url = lock.asset_url.as_deref().ok_or_else(|| {
        FusionError::new(
            ErrorCode::InvalidRuntimeChannel,
            "A runtime lock must pin the exact asset URL.",
        )
    })?;
    super::adapters::https_url(asset_url)?;
    let tag = lock
        .release_tag
        .as_deref()
        .filter(|tag| {
            !tag.trim().is_empty() && tag.len() <= 256 && !tag.chars().any(char::is_control)
        })
        .ok_or_else(|| {
            FusionError::new(
                ErrorCode::InvalidRuntimeChannel,
                "A runtime lock must pin the release tag.",
            )
        })?;
    if layer
        .dynamic_provider
        .as_ref()
        .and_then(|provider| provider.tag.as_ref())
        .is_some_and(|expected| expected != tag)
    {
        return Err(FusionError::new(
            ErrorCode::InvalidRuntimeChannel,
            "The runtime lock tag differs from the declared provider tag.",
        ));
    }
    if lock.release_id.trim().is_empty()
        || lock.asset_id.trim().is_empty()
        || lock.transport_digest.value.len() != 64
        || !lock
            .transport_digest
            .value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(FusionError::new(
            ErrorCode::InvalidRuntimeChannel,
            "A dynamic runtime lock is incomplete.",
        ));
    }
    for expected in &layer.critical_files {
        let pinned = lock
            .critical_files
            .iter()
            .find(|file| file.path == expected.path && file.role == expected.role)
            .ok_or_else(|| {
                FusionError::new(
                    ErrorCode::InvalidRuntimeChannel,
                    "Dynamic lock omits a critical runtime file.",
                )
            })?;
        if !object
            .files
            .iter()
            .any(|file| file.path == pinned.path && file.blake3 == pinned.blake3)
        {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "Resolved runtime binary hashes differ from the CAS tree.",
            ));
        }
    }
    Ok(())
}

fn verify_critical_files(snapshot: &WorkspaceSnapshot) -> Result<(), FusionError> {
    super::integrity::PreLaunchIntegrityGuard::verify_integrity(snapshot)
}

fn path_is_inside(path: &str, parent: &str) -> bool {
    let path = path.to_lowercase();
    let parent = parent.to_lowercase();
    path == parent || path.starts_with(&(parent + "/"))
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    let left = left.to_string_lossy().replace('\\', "/").to_lowercase();
    let right = right.to_string_lossy().replace('\\', "/").to_lowercase();
    path_is_inside(&left, &right) || path_is_inside(&right, &left)
}

fn create_local_file_link(source: &Path, target: &Path) -> Result<(), FusionError> {
    #[cfg(windows)]
    let result = std::os::windows::fs::symlink_file(source, target);
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(source, target);
    #[cfg(not(any(windows, unix)))]
    let result: std::io::Result<()> = Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "File links are unavailable",
    ));
    result.map_err(|error| {
        FusionError::new(
            ErrorCode::CrossVolumeLinkNotSupported,
            format!("Explicit local-base symlink projection is unavailable: {error}"),
        )
    })
}
