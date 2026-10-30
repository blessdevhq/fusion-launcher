//! Tauri-independent integration facade. Storage, transport and progress are injected.

use super::adapters::{
    ArtifactTransport, Cancellation, TransferLimits, TransferProgress, TransferProgressSink,
};
use super::canonical::{CanonicalGameManifest, CanonicalSourceIndex, QualifiedGameId};
use super::capabilities::{CapabilityNegotiator, CapabilityRegistry};
use super::config_templates::TemplateContext;
use super::dto::*;
use super::errors::{ErrorCode, FusionError};
use super::evolution::SchemaUpcaster;
use super::integrity::{ExecutableTrust, ManifestSyncVerifier, PreLaunchIntegrityGuard};
use super::layers::{LayerCache, LayerInput};
use super::persistence::PersistenceRoots;
use super::plan::{FetchStep, PlanBuilder, PlanOptions, PlanStep};
use super::resolver::{
    BoundManifestSyncVerifier, CloudUriResolver, DynamicAssetAttestation, DynamicProviderResolver,
    GithubReleaseClient, LocalLocatorResolver, ManifestFetcher, MirrorCascadeResolver,
    SecureGithubReleaseClient, StorefrontInventory, TorrentAdapter,
};
use super::schema::{ArchiveFormat, GameManifest, LayerKind, LayerSource, RuntimeChannel};
use super::supervisor::{ProcessSupervisor, SupervisorOptions};
use super::telemetry::SanitizedCrashReport;
use super::transforms::PatchToolRegistry;
use super::workspace::{WorkspaceAssembler, WorkspaceBuildOptions, WorkspaceSnapshot};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Synchronous, bounded database operations. Implementations transact source/catalog
/// replacement, activation and operation transitions. No network or filesystem IO here.
pub trait FusionStorage: Send + Sync {
    /// Reserved identity lookup includes disconnected sources, independently of
    /// canonical index JSON. Match ID or URL to reject identity substitution.
    fn pinned_source_identity(
        &self,
        source_id: &str,
        index_url: &str,
    ) -> Result<Option<super::signatures::PinnedSourceIdentity>, FusionError>;
    fn add_source(
        &self,
        index: &CanonicalSourceIndex,
        url: &str,
    ) -> Result<FusionSourceRecord, FusionError>;
    fn refresh_source(
        &self,
        index: &CanonicalSourceIndex,
        url: &str,
    ) -> Result<FusionSourceRecord, FusionError>;
    fn list_sources(&self) -> Result<Vec<FusionSourceRecord>, FusionError>;
    fn remove_source(&self, source_id: &str) -> Result<(), FusionError>;
    fn list_catalog(&self) -> Result<Vec<FusionCatalogRecord>, FusionError>;
    fn catalog_local_state(&self) -> Result<CatalogLocalState, FusionError>;
    fn put_manifest(&self, record: &FusionManifestRecord) -> Result<(), FusionError>;
    fn get_manifest(
        &self,
        game: &QualifiedGameId,
        digest: &str,
    ) -> Result<Option<FusionManifestRecord>, FusionError>;
    fn get_installation(
        &self,
        game: &QualifiedGameId,
    ) -> Result<Option<FusionInstallationRecord>, FusionError>;
    fn list_installations(&self) -> Result<Vec<FusionInstallationRecord>, FusionError>;
    fn set_options(&self, game: &QualifiedGameId, options: &GameOptions)
        -> Result<(), FusionError>;
    fn bind_installation_root(
        &self,
        game: &QualifiedGameId,
        library_root: &Path,
        external_root: Option<&Path>,
    ) -> Result<(), FusionError>;
    fn begin_operation(
        &self,
        game: &QualifiedGameId,
        kind: &str,
    ) -> Result<FusionOperationRecord, FusionError>;
    fn progress_operation(&self, id: &str, sequence: u64, phase: &str)
        -> Result<bool, FusionError>;
    fn finish_operation(&self, id: &str, status: &str) -> Result<bool, FusionError>;
    fn activate_snapshot(
        &self,
        snapshot: &WorkspaceSnapshot,
        library_root: &Path,
        external_root: Option<&Path>,
    ) -> Result<(), FusionError>;
    fn get_active_snapshot(
        &self,
        game: &QualifiedGameId,
    ) -> Result<Option<FusionWorkspaceRecord>, FusionError>;
    fn list_snapshots(
        &self,
        game: &QualifiedGameId,
    ) -> Result<Vec<FusionWorkspaceRecord>, FusionError>;
    fn remove_installation(&self, game: &QualifiedGameId) -> Result<(), FusionError>;
    fn is_trusted(
        &self,
        game: &QualifiedGameId,
        path: &str,
        blake3: &str,
    ) -> Result<bool, FusionError>;
    fn trust_executable(
        &self,
        game: &QualifiedGameId,
        path: &str,
        blake3: &str,
    ) -> Result<(), FusionError>;
    fn put_crash(&self, report: &SanitizedCrashReport) -> Result<FusionCrashRecord, FusionError>;
    fn get_crash(&self, id: &str) -> Result<Option<FusionCrashRecord>, FusionError>;
    fn list_crashes(&self, game: &QualifiedGameId) -> Result<Vec<FusionCrashRecord>, FusionError>;
    fn set_crash_status(&self, id: &str, status: &str) -> Result<(), FusionError>;
}

pub trait ProgressSink: Send + Sync {
    fn emit(&self, event: &OperationProgress);
}
pub struct NoProgress;
impl ProgressSink for NoProgress {
    fn emit(&self, _: &OperationProgress) {}
}

/// Trusted host setup runs under the launch operation and workspace lease.
/// It can prepare mutable profiles; publisher JSON cannot register this hook.
pub trait LaunchPreparation: Send + Sync {
    fn prepare(&self, workspace: &super::workspace::AssembledWorkspace) -> Result<(), FusionError>;
}

struct OperationTransferSink {
    service: FusionService,
    game: QualifiedGameId,
    operation_id: String,
    layer_id: String,
}

impl TransferProgressSink for OperationTransferSink {
    fn emit(&self, progress: TransferProgress) -> Result<(), FusionError> {
        if progress.total_bytes != 0 && progress.downloaded_bytes > progress.total_bytes {
            return Err(FusionError::new(
                ErrorCode::TransferLimitExceeded,
                "Transfer progress exceeds its declared size.",
            ));
        }
        self.service.progress_with_transfer(
            &self.game,
            &self.operation_id,
            "fetch",
            Some(&self.layer_id),
            Some(progress),
        )
    }
}

struct StorageTrust(Arc<dyn FusionStorage>);
struct StorageIdentityLookup(Arc<dyn FusionStorage>);
impl super::signatures::SourceIdentityLookup for StorageIdentityLookup {
    fn pinned_identity(
        &self,
        source_id: &str,
        index_url: &str,
    ) -> Result<Option<super::signatures::PinnedSourceIdentity>, FusionError> {
        self.0.pinned_source_identity(source_id, index_url)
    }
}
impl ExecutableTrust for StorageTrust {
    fn is_trusted(
        &self,
        game: &QualifiedGameId,
        path: &str,
        blake3: &str,
    ) -> Result<bool, FusionError> {
        self.0.is_trusted(game, path, blake3)
    }
}

#[derive(Clone)]
struct ActiveOperation {
    cancel: Cancellation,
    progress: OperationProgress,
}

/// Runtime authority is shared by every command, so cancellation and stale event
/// protection survive cloned facades. The root is fixed for an instance's lifetime.
#[derive(Clone)]
pub struct FusionService {
    inner: Arc<ServiceInner>,
}

/// Only the trusted host constructs this configuration. Source JSON and IPC
/// cannot install bridges or grant patch-tool execution authority.
#[derive(Clone, Default)]
pub struct FusionHostAdapters {
    pub launch_preparation: Option<Arc<dyn LaunchPreparation>>,
    pub storefront: Option<Arc<dyn StorefrontInventory>>,
    pub torrent: Option<Arc<dyn TorrentAdapter>>,
    pub cloud: Vec<Arc<dyn CloudUriResolver>>,
    pub patch_tools: PatchToolRegistry,
}

impl FusionHostAdapters {
    fn mirrors(&self, transport: Arc<dyn ArtifactTransport>) -> MirrorCascadeResolver {
        let mut resolver = MirrorCascadeResolver::new(transport);
        if let Some(torrent) = &self.torrent {
            resolver = resolver.with_torrent(torrent.clone());
        }
        for provider in &self.cloud {
            resolver = resolver.with_cloud(provider.clone());
        }
        resolver
    }
}

struct ServiceInner {
    library_root: PathBuf,
    storage: Arc<dyn FusionStorage>,
    transport: Arc<dyn ArtifactTransport>,
    progress: Arc<dyn ProgressSink>,
    operations: Mutex<BTreeMap<QualifiedGameId, ActiveOperation>>,
    event_order: Mutex<()>,
    environment: ServiceEnvironment,
    adapters: FusionHostAdapters,
    attestations: BTreeMap<QualifiedGameId, DynamicAssetAttestation>,
}

#[derive(Clone)]
pub struct ServiceEnvironment {
    pub appdata: PathBuf,
    pub local_appdata: PathBuf,
    pub display_width: u32,
    pub display_height: u32,
    pub refresh_rate: u32,
}

impl ServiceEnvironment {
    pub fn from_process() -> Result<Self, FusionError> {
        let environment_path = |key| {
            std::env::var_os(key).map(PathBuf::from).ok_or_else(|| {
                FusionError::new(
                    ErrorCode::InvalidEnvironment,
                    format!("{key} is unavailable."),
                )
            })
        };
        Ok(Self {
            appdata: environment_path("APPDATA")?,
            local_appdata: environment_path("LOCALAPPDATA")?,
            display_width: 1920,
            display_height: 1080,
            refresh_rate: 60,
        })
    }
}

impl FusionService {
    pub fn open(
        library_root: &Path,
        storage: Arc<dyn FusionStorage>,
        transport: Arc<dyn ArtifactTransport>,
        progress: Arc<dyn ProgressSink>,
        environment: ServiceEnvironment,
    ) -> Result<Self, FusionError> {
        super::fs_safety::ensure_directory(library_root)?;
        Ok(Self {
            inner: Arc::new(ServiceInner {
                library_root: library_root.to_owned(),
                storage,
                transport,
                progress,
                operations: Mutex::new(BTreeMap::new()),
                event_order: Mutex::new(()),
                environment,
                adapters: FusionHostAdapters::default(),
                attestations: BTreeMap::new(),
            }),
        })
    }

    /// Configured by the trusted host, never by a downloaded manifest or IPC body.
    pub fn with_tools(
        mut self,
        tools: PatchToolRegistry,
        exact_hash_trusted: bool,
    ) -> Result<Self, FusionError> {
        let inner = Arc::get_mut(&mut self.inner).ok_or_else(|| {
            FusionError::new(
                ErrorCode::OperationInProgress,
                "Configure tools before sharing the service.",
            )
        })?;
        inner.adapters.patch_tools = tools.verify_explicit_trust(exact_hash_trusted)?;
        Ok(self)
    }

    pub fn with_host_adapters(mut self, adapters: FusionHostAdapters) -> Result<Self, FusionError> {
        Arc::get_mut(&mut self.inner)
            .ok_or_else(|| {
                FusionError::new(
                    ErrorCode::OperationInProgress,
                    "Configure adapters before sharing the service.",
                )
            })?
            .adapters = adapters;
        Ok(self)
    }

    pub fn with_dynamic_attestations(
        mut self,
        attestations: BTreeMap<QualifiedGameId, DynamicAssetAttestation>,
    ) -> Result<Self, FusionError> {
        Arc::get_mut(&mut self.inner)
            .ok_or_else(|| {
                FusionError::new(
                    ErrorCode::OperationInProgress,
                    "Configure attestations before sharing the service.",
                )
            })?
            .attestations = attestations;
        Ok(self)
    }

    pub fn library_root(&self) -> &Path {
        &self.inner.library_root
    }
    pub fn has_active_operations(&self) -> bool {
        self.inner
            .operations
            .lock()
            .map(|ops| !ops.is_empty())
            .unwrap_or(true)
    }
    pub fn has_active_game(&self, game: &QualifiedGameId) -> bool {
        self.inner
            .operations
            .lock()
            .map(|operations| operations.contains_key(game))
            .unwrap_or(true)
    }

    fn fetcher(&self) -> Result<ManifestFetcher, FusionError> {
        ManifestFetcher::open(&self.inner.library_root, self.inner.transport.clone()).map(
            |fetcher| {
                fetcher.with_identity_lookup(Arc::new(StorageIdentityLookup(
                    self.inner.storage.clone(),
                )))
            },
        )
    }

    pub async fn source_preview(&self, url: &str) -> Result<SourcePreview, FusionError> {
        let verified = self
            .fetcher()?
            .fetch_verified_index(url, &Cancellation::default())
            .await?;
        let index = verified.canonical;
        Ok(SourcePreview {
            source_id: index.source_info.id,
            name: index.source_info.name,
            url: url.to_owned(),
            game_count: index.games.len(),
            public_key: index.source_info.public_key,
            signature_verified: verified.signature_verified,
        })
    }

    pub async fn source_add(&self, url: &str) -> Result<SourceSummary, FusionError> {
        let index = self.resolved_index(url).await?;
        let record = self.inner.storage.add_source(&index, url)?;
        Ok(source_summary(record))
    }

    pub fn source_list(&self) -> Result<Vec<SourceSummary>, FusionError> {
        Ok(self
            .inner
            .storage
            .list_sources()?
            .into_iter()
            .map(source_summary)
            .collect())
    }

    pub async fn source_refresh(&self, source_id: &str) -> Result<SourceSummary, FusionError> {
        let existing = self.source(source_id)?;
        let index = self.resolved_index(&existing.url).await?;
        if index.source_info.id != source_id
            || index.source_info.public_key != existing.index.source_info.public_key
        {
            return Err(FusionError::new(
                ErrorCode::SnapshotConflict,
                "Refreshing cannot replace the source identity or public key.",
            ));
        }
        Ok(source_summary(
            self.inner.storage.refresh_source(&index, &existing.url)?,
        ))
    }

    pub fn source_remove(&self, source_id: &str) -> Result<(), FusionError> {
        if self
            .inner
            .operations
            .lock()
            .map_err(|_| internal_error())?
            .keys()
            .any(|game| game.source_id == source_id)
        {
            return Err(FusionError::new(
                ErrorCode::OperationInProgress,
                "The source has active operations.",
            ));
        }
        self.inner.storage.remove_source(source_id)
    }

    pub(crate) async fn resolved_index(
        &self,
        url: &str,
    ) -> Result<CanonicalSourceIndex, FusionError> {
        let mut index = self
            .fetcher()?
            .fetch_index(url, &Cancellation::default())
            .await?;
        let base = super::adapters::https_url(url)?;
        for entry in &mut index.games {
            if !entry.manifest_url.contains(':') {
                entry.manifest_url = base
                    .join(&entry.manifest_url)
                    .map_err(|_| {
                        FusionError::new(ErrorCode::InvalidUri, "Cannot resolve manifest URL.")
                    })?
                    .to_string();
            }
            super::adapters::https_url(&entry.manifest_url)?;
        }
        Ok(index)
    }

    fn source(&self, id: &str) -> Result<FusionSourceRecord, FusionError> {
        self.inner
            .storage
            .list_sources()?
            .into_iter()
            .find(|source| source.index.source_info.id == id)
            .ok_or_else(|| {
                FusionError::new(
                    ErrorCode::MissingParent,
                    "The required source is not connected.",
                )
                .with_detail("sourceId", id)
            })
    }

    pub fn source_list_catalog(&self) -> Result<Vec<FusionCatalogEntry>, FusionError> {
        let local = self.inner.storage.catalog_local_state()?;
        let operations: BTreeSet<_> = self
            .inner
            .operations
            .lock()
            .map_err(|_| internal_error())?
            .keys()
            .cloned()
            .collect();
        let registry = self.registry();
        self.inner
            .storage
            .list_catalog()?
            .into_iter()
            .map(|row| {
                let game = QualifiedGameId {
                    source_id: row.source_id.clone(),
                    game_id: row.entry.id.clone(),
                };
                let active = local
                    .installations
                    .get(&game)
                    .filter(|record| record.active_snapshot_id.is_some());
                let active_version = active.and_then(|record| record.active_version.clone());
                let snapshot = local.snapshots.get(&game).filter(|snapshot| {
                    active.and_then(|record| record.active_snapshot_id.as_ref())
                        == Some(&snapshot.id)
                });
                let manifest = local
                    .manifests
                    .get(&game)
                    .or_else(|| snapshot.map(|snapshot| &snapshot.manifest));
                let ready_to_play = snapshot.is_some_and(|snapshot| {
                    snapshot.verify_directory_identity().is_ok()
                        && !operations.contains(&game)
                        && !local.active_operations.contains(&game)
                        && CapabilityNegotiator::negotiate(&snapshot.manifest, &registry)
                            .blockers
                            .is_empty()
                        && critical_inventory(snapshot).iter().all(|(path, file)| {
                            local.trusted_executables.get(&game).is_some_and(|trusted| {
                                trusted.contains(&(path.clone(), file.blake3.clone()))
                            })
                        })
                });
                Ok(FusionCatalogEntry {
                    kind: "fusion_source",
                    source_id: row.source_id,
                    source_name: row.source_name,
                    game_id: row.entry.id,
                    title: row.entry.title,
                    platform: row
                        .entry
                        .platform
                        .or_else(|| manifest.and_then(|manifest| manifest.platform.clone())),
                    cover_url: row
                        .entry
                        .cover_url
                        .or_else(|| manifest.and_then(|manifest| manifest.cover_url.clone())),
                    update_available: active.is_some_and(|record| {
                        record
                            .active_manifest_blake3
                            .as_ref()
                            .is_some_and(|digest| {
                                !digest.eq_ignore_ascii_case(&row.entry.manifest_blake3)
                            })
                    }),
                    version: row.entry.version,
                    installed: active.is_some(),
                    active_version,
                    active_snapshot_id: active.and_then(|record| record.active_snapshot_id.clone()),
                    ready_to_play,
                })
            })
            .collect()
    }
}

fn source_summary(record: FusionSourceRecord) -> SourceSummary {
    SourceSummary {
        source_id: record.index.source_info.id,
        name: record.index.source_info.name,
        url: record.url,
        game_count: record.index.games.len(),
        public_key: record.index.source_info.public_key,
        refreshed_at: record.refreshed_at,
    }
}
fn internal_error() -> FusionError {
    FusionError::new(ErrorCode::Io, "The Fusion service worker failed.")
}

struct ResolvedManifest {
    raw_blake3: String,
    canonical: CanonicalGameManifest,
    binding: super::workspace::VerifiedManifestBinding,
}

struct PrivateStage {
    root: PathBuf,
}
impl PrivateStage {
    fn create(library: &Path) -> Result<Self, FusionError> {
        let parent = library.join(".fusion-source/service-staging");
        let _guard = super::fs_safety::ensure_directory(&parent)?;
        let root = parent.join(super::fs_safety::unique_name("operation"));
        std::fs::create_dir(&root)
            .map_err(|error| super::fs_safety::io_error("Create service staging", &root, error))?;
        Ok(Self { root })
    }
}
impl Drop for PrivateStage {
    fn drop(&mut self) {
        let _ = super::fs_safety::safe_remove_tree(&self.root);
    }
}

impl FusionService {
    async fn resolve_manifest(
        &self,
        game: &QualifiedGameId,
        cancel: &Cancellation,
    ) -> Result<ResolvedManifest, FusionError> {
        self.resolve_manifest_with_freshness(game, cancel, false)
            .await
    }

    async fn resolve_manifest_with_freshness(
        &self,
        game: &QualifiedGameId,
        cancel: &Cancellation,
        fresh: bool,
    ) -> Result<ResolvedManifest, FusionError> {
        let mut current = game.clone();
        let mut visited = BTreeSet::new();
        let mut refreshed_sources = BTreeMap::<String, FusionSourceRecord>::new();
        let mut parents = BTreeMap::<QualifiedGameId, GameManifest>::new();
        let mut documents = BTreeMap::<QualifiedGameId, (Vec<u8>, String)>::new();
        for _ in 0..64 {
            cancel.check()?;
            if !visited.insert(current.clone()) {
                return Err(FusionError::new(
                    ErrorCode::ExtendsCycle,
                    "The connected manifests contain an extends cycle.",
                ));
            }
            let mut source = self.source(&current.source_id)?;
            if let Some(refreshed) = refreshed_sources.get(&current.source_id) {
                source = refreshed.clone();
            } else if fresh {
                let mut index = self.fetcher()?.fetch_index(&source.url, cancel).await?;
                if index.source_info.id != source.index.source_info.id
                    || index.source_info.public_key != source.index.source_info.public_key
                {
                    return Err(FusionError::new(
                        ErrorCode::StrictSyncRequired,
                        "A source changed identity during strict synchronization.",
                    ));
                }
                let base = super::adapters::https_url(&source.url)?;
                for entry in &mut index.games {
                    if !entry.manifest_url.contains(':') {
                        entry.manifest_url = base
                            .join(&entry.manifest_url)
                            .map_err(|_| {
                                FusionError::new(
                                    ErrorCode::InvalidUri,
                                    "Cannot resolve a strict-sync manifest URL.",
                                )
                            })?
                            .to_string();
                    }
                }
                source.index = index;
                refreshed_sources.insert(current.source_id.clone(), source.clone());
            }
            let mut entry = source
                .index
                .games
                .iter()
                .find(|entry| entry.id == current.game_id)
                .cloned()
                .ok_or_else(|| {
                    FusionError::new(
                        ErrorCode::MissingParent,
                        "The required game is absent from its connected source.",
                    )
                })?;
            if !entry.manifest_url.contains(':') {
                entry.manifest_url = super::adapters::https_url(&source.url)?
                    .join(&entry.manifest_url)
                    .map_err(|_| {
                        FusionError::new(ErrorCode::InvalidUri, "Cannot resolve manifest URL.")
                    })?
                    .to_string();
            }
            let cached = self
                .inner
                .storage
                .get_manifest(&current, &entry.manifest_blake3)?;
            let bytes = if let Some(cached) = cached.filter(|cached| {
                blake3::hash(&cached.raw_bytes)
                    .to_hex()
                    .as_str()
                    .eq_ignore_ascii_case(&entry.manifest_blake3)
            }) {
                cached.raw_bytes
            } else {
                let stage = PrivateStage::create(&self.inner.library_root)?;
                let path = stage.root.join("manifest.json");
                self.inner
                    .transport
                    .fetch(&entry.manifest_url, &path, &TransferLimits::json(), cancel)
                    .await?;
                cancel.check()?;
                let mut handle = super::fs_safety::open_read(&path)?;
                let mut bytes = Vec::new();
                use std::io::Read;
                (&mut handle)
                    .take(crate::net::JSON_BODY_LIMIT as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| internal_error())?;
                if bytes.len() > crate::net::JSON_BODY_LIMIT {
                    return Err(FusionError::new(
                        ErrorCode::TransferLimitExceeded,
                        "The manifest exceeds its metadata budget.",
                    ));
                }
                bytes
            };
            if !blake3::hash(&bytes)
                .to_hex()
                .as_str()
                .eq_ignore_ascii_case(&entry.manifest_blake3)
            {
                return Err(FusionError::new(
                    ErrorCode::ManifestDigestMismatch,
                    "The raw manifest differs from its index digest.",
                ));
            }
            let json = std::str::from_utf8(&bytes).map_err(|_| {
                FusionError::new(ErrorCode::InvalidJson, "Manifest JSON must use UTF-8.")
            })?;
            let wire = SchemaUpcaster::parse_game_manifest(json)?;
            if wire.id != current.game_id || wire.version != entry.version {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "The manifest identity/version differs from its index entry.",
                ));
            }
            let parent = wire.extends.as_ref().map(|reference| {
                let (source_id, game_id) = reference
                    .split_once("::")
                    .unwrap_or((&current.source_id, reference));
                QualifiedGameId {
                    source_id: source_id.to_owned(),
                    game_id: game_id.to_owned(),
                }
            });
            documents.insert(
                current.clone(),
                (bytes, entry.manifest_blake3.to_ascii_lowercase()),
            );
            parents.insert(current.clone(), wire);
            if let Some(parent) = parent {
                current = parent;
            } else {
                break;
            }
        }
        if documents.len() == 64
            && parents
                .get(&current)
                .is_some_and(|wire| wire.extends.is_some())
        {
            return Err(FusionError::new(
                ErrorCode::ExtendsCycle,
                "The inheritance chain exceeds the supported depth.",
            ));
        }
        let (raw, digest) = documents.get(game).ok_or_else(internal_error)?;
        let canonical = SchemaUpcaster::upcast_game_manifest(
            &game.source_id,
            std::str::from_utf8(raw).map_err(|_| internal_error())?,
            &parents,
        )?;
        // Validate every source in the ancestry before modifying any catalog.
        // A bad parent signature must not leave a partially refreshed child.
        for source in refreshed_sources.values() {
            self.inner
                .storage
                .refresh_source(&source.index, &source.url)?;
        }
        for (key, (bytes, hash)) in &documents {
            let resolved = SchemaUpcaster::upcast_game_manifest(
                &key.source_id,
                std::str::from_utf8(bytes).map_err(|_| internal_error())?,
                &parents,
            )?;
            self.inner.storage.put_manifest(&FusionManifestRecord {
                game: key.clone(),
                digest: hash.clone(),
                raw_bytes: bytes.clone(),
                canonical: resolved,
            })?;
        }
        let binding = super::workspace::VerifiedManifestBinding::from_verified(&canonical, digest)?;
        Ok(ResolvedManifest {
            raw_blake3: digest.clone(),
            canonical,
            binding,
        })
    }

    fn options(&self, game: &QualifiedGameId) -> Result<GameOptions, FusionError> {
        Ok(self
            .inner
            .storage
            .get_installation(game)?
            .map(|record| record.options)
            .unwrap_or_default())
    }
    fn plan_options(options: &GameOptions) -> PlanOptions {
        PlanOptions {
            build_profile_id: options.build_profile_id.clone(),
            controls_profile_id: options.controls_profile_id.clone(),
            enabled_addons: options.enabled_addons.clone(),
            runtime_channel: options.runtime_channel,
            ..PlanOptions::default()
        }
    }
    fn registry(&self) -> CapabilityRegistry {
        let mut registry = CapabilityRegistry::current_client();
        for capability in self
            .inner
            .adapters
            .mirrors(self.inner.transport.clone())
            .configured_capabilities()
        {
            registry = registry.with(capability);
        }
        for format in self.inner.adapters.patch_tools.verified_formats() {
            registry = registry.with(match format {
                super::schema::PatchFormat::Xdelta3 => "patch.xdelta3.v1",
                super::schema::PatchFormat::Bsdiff => "patch.bsdiff.v1",
            });
        }
        registry
    }

    pub fn configured_capabilities(&self) -> BTreeSet<String> {
        self.registry().iter().map(str::to_owned).collect()
    }

    fn assembler(
        &self,
        game: &QualifiedGameId,
        external: Option<&str>,
    ) -> Result<WorkspaceAssembler, FusionError> {
        let assembler = WorkspaceAssembler::open(&self.inner.library_root, game)?
            .with_patch_tools(self.inner.adapters.patch_tools.clone());
        if let Some(root) = external {
            assembler.with_external_workspace_root(Path::new(root))
        } else {
            Ok(assembler)
        }
    }

    async fn inspect_snapshot(
        &self,
        game: &QualifiedGameId,
        clear_staging: bool,
    ) -> Result<(Option<WorkspaceSnapshot>, bool), FusionError> {
        let record = self.inner.storage.get_installation(game)?;
        let external = record
            .as_ref()
            .and_then(|record| record.external_workspace_root.clone());
        let assembler = self.assembler(game, external.as_deref())?;
        let state_path = self
            .inner
            .library_root
            .join(".fusion-source/workspaces")
            .join(&game.source_id)
            .join(&game.game_id)
            .join("state.json");
        let storage = self.inner.storage.clone();
        let library = self.inner.library_root.clone();
        let game = game.clone();
        tokio::task::spawn_blocking(move || {
            assembler.inspect_active(|lease, can_rollback| {
                // The same FS lock covers this DB transition for Some and None.
                if let Some(lease) = lease {
                    storage.activate_snapshot(
                        &lease.snapshot,
                        &library,
                        external.as_deref().map(Path::new),
                    )?;
                    Ok((Some(lease.snapshot.clone()), can_rollback))
                } else {
                    let current = storage.get_installation(&game)?;
                    if current
                        .as_ref()
                        .is_some_and(|record| record.active_snapshot_id.is_some())
                    {
                        super::fs_safety::open_read(&state_path).map_err(|_| {
                            FusionError::new(
                                ErrorCode::SnapshotNotFound,
                                "The installed workspace state is missing.",
                            )
                        })?;
                        storage.remove_installation(&game)?;
                    } else if clear_staging
                        && current
                            .as_ref()
                            .is_some_and(|record| record.status == "staging")
                    {
                        storage.remove_installation(&game)?;
                    }
                    Ok((None, can_rollback))
                }
            })
        })
        .await
        .map_err(|_| internal_error())?
    }

    pub async fn game_get_state(
        &self,
        game: &QualifiedGameId,
    ) -> Result<FusionGameState, FusionError> {
        let options = self.options(game)?;
        let active = self.inspect_snapshot(game, false).await;
        let mut blockers = Vec::new();
        let mut warnings = Vec::new();
        let installed = self
            .inner
            .storage
            .get_installation(game)?
            .is_some_and(|record| record.active_snapshot_id.is_some());
        let (active, rollback_available) = match active {
            Ok(snapshot) => snapshot,
            Err(error) if error.code == ErrorCode::OperationInProgress => (
                self.inner
                    .storage
                    .get_active_snapshot(game)?
                    .map(|record| {
                        serde_json::from_str(&record.snapshot_json).map_err(|_| internal_error())
                    })
                    .transpose()?,
                false,
            ),
            Err(error) => {
                let repairable = matches!(
                    error.code,
                    ErrorCode::MissingInjectorDll
                        | ErrorCode::CorruptedCriticalBinary
                        | ErrorCode::LocalBaseModified
                );
                blockers.push(error);
                if repairable {
                    let external = self
                        .inner
                        .storage
                        .get_installation(game)?
                        .and_then(|record| record.external_workspace_root);
                    let assembler = self.assembler(game, external.as_deref())?;
                    let metadata =
                        tokio::task::spawn_blocking(move || assembler.snapshot_for_update())
                            .await
                            .map_err(|_| internal_error())??;
                    (metadata, false)
                } else {
                    (None, false)
                }
            }
        };
        let resolved = self.resolve_manifest(game, &Cancellation::default()).await;
        let (manifest, current_digest) = match resolved {
            Ok(resolved) => (resolved.canonical, Some(resolved.raw_blake3)),
            Err(error) => {
                if let Some(snapshot) = &active {
                    warnings.push(error.message);
                    (snapshot.manifest.clone(), None)
                } else {
                    return Err(error);
                }
            }
        };
        // Readiness is the installed snapshot's contract. A new unsupported
        // source version does not disable a previously working installation.
        let report = CapabilityNegotiator::negotiate(
            active
                .as_ref()
                .map(|snapshot| &snapshot.manifest)
                .unwrap_or(&manifest),
            &self.registry(),
        );
        blockers.extend(report.blockers);
        let plan = PlanBuilder::build(&manifest, &Self::plan_options(&options), &self.registry());
        let selected = match plan {
            Ok(plan) => plan.selected_layers.into_iter().collect::<BTreeSet<_>>(),
            Err(error) => {
                if active.is_some() {
                    warnings.push(error.message);
                } else {
                    blockers.push(error);
                }
                BTreeSet::new()
            }
        };
        let mut executables = Vec::new();
        if let Some(snapshot) = &active {
            let checked = snapshot.clone();
            if let Err(error) = tokio::task::spawn_blocking(move || {
                PreLaunchIntegrityGuard::verify_integrity(&checked)
            })
            .await
            .map_err(|_| internal_error())?
            {
                if !blockers.iter().any(|existing| existing == &error) {
                    blockers.push(error);
                }
            }
            for (path, file) in critical_inventory(snapshot) {
                let trusted = self.inner.storage.is_trusted(game, &path, &file.blake3)?;
                if !trusted {
                    blockers.push(
                        FusionError::new(
                            ErrorCode::UntrustedExecutable,
                            "Explicit trust is required for the displayed digest.",
                        )
                        .with_detail("path", &path)
                        .with_detail("blake3", &file.blake3),
                    );
                }
                executables.push(ExecutableTrustState {
                    path,
                    blake3: file.blake3,
                    role: file.role,
                    trusted,
                });
            }
        }
        let operation = self
            .inner
            .operations
            .lock()
            .map_err(|_| internal_error())?
            .get(game)
            .map(|operation| operation.progress.clone());
        let addons = manifest
            .layers
            .values()
            .filter(|layer| layer.optional)
            .map(|layer| AddonState {
                id: layer.id.clone(),
                title: layer.title.clone(),
                description: layer.description.clone(),
                kind: enum_string(&layer.kind),
                enabled: selected.contains(&layer.id),
            })
            .collect();
        let user_layers = manifest
            .layers
            .values()
            .filter(|layer| selected.contains(&layer.id))
            .filter_map(|layer| match &layer.source {
                LayerSource::UserProvided {
                    instructions,
                    archive,
                } => Some(UserLayerRequirement {
                    layer_id: layer.id.clone(),
                    instructions: instructions.clone(),
                    archive: enum_string(archive),
                    raw_path_required: *archive == ArchiveFormat::Raw,
                }),
                _ => None,
            })
            .collect();
        let runtime_channels = match &manifest.runtime {
            Some(runtime)
                if manifest
                    .layers
                    .get(&runtime.layer_id)
                    .is_some_and(|layer| layer.dynamic_provider.is_some()) =>
            {
                vec![RuntimeChannel::Pinned, RuntimeChannel::Dynamic]
            }
            Some(_) => vec![RuntimeChannel::Pinned],
            None => Vec::new(),
        };
        let active_version = active
            .as_ref()
            .map(|snapshot| snapshot.manifest.version.clone());
        let current_canonical =
            blake3::hash(&serde_json::to_vec(&manifest).map_err(|_| internal_error())?)
                .to_hex()
                .to_string();
        let update_available = active.as_ref().is_some_and(|snapshot| {
            current_digest
                .as_ref()
                .is_some_and(|digest| snapshot.source_manifest_blake3.as_ref() != Some(digest))
                || snapshot.canonical_blake3 != current_canonical
                || options
                    .runtime_channel
                    .is_some_and(|channel| snapshot.plan.runtime_channel != Some(channel))
                || options.enabled_addons.as_ref().is_some_and(|addons| {
                    addons
                        != &snapshot
                            .plan
                            .selected_layers
                            .iter()
                            .filter(|id| {
                                snapshot
                                    .manifest
                                    .layers
                                    .get(*id)
                                    .is_some_and(|layer| layer.optional)
                            })
                            .cloned()
                            .collect()
                })
        });
        let crash_reports: Vec<_> = self
            .inner
            .storage
            .list_crashes(game)?
            .into_iter()
            .filter(|report| report.status == "pending")
            .map(|report| CrashSummary {
                report_id: report.id,
                created_at: report.created_at,
                exit_code: report
                    .report
                    .get("exitCode")
                    .and_then(serde_json::Value::as_i64),
                crash_signature_hash: report
                    .report
                    .get("crashSignatureHash")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
            })
            .collect();
        let crash_report_ids = crash_reports
            .iter()
            .map(|report| report.report_id.clone())
            .collect();
        let applied_options = active.as_ref().map(|snapshot| GameOptions {
            enabled_addons: Some(
                snapshot
                    .plan
                    .selected_layers
                    .iter()
                    .filter(|id| {
                        snapshot.manifest.layers.get(*id).is_some_and(|layer| {
                            layer.optional
                                && matches!(
                                    layer.kind,
                                    LayerKind::Addon | LayerKind::Mod | LayerKind::ShaderCache
                                )
                        })
                    })
                    .cloned()
                    .collect(),
            ),
            runtime_channel: snapshot.plan.runtime_channel,
            build_profile_id: snapshot.plan.build_profile_id.clone(),
            controls_profile_id: snapshot.plan.controls_profile_id.clone(),
        });
        let default_runtime_channel = manifest
            .runtime
            .as_ref()
            .map(|runtime| runtime.default_channel);
        let external_workspace_root = self
            .inner
            .storage
            .get_installation(game)?
            .and_then(|record| record.external_workspace_root);
        warnings.extend(report.warnings.into_iter().map(|warning| warning.message));
        Ok(FusionGameState {
            source_id: game.source_id.clone(),
            game_id: game.game_id.clone(),
            title: manifest.title,
            version: manifest.version,
            platform: manifest.platform,
            installed,
            active_version,
            active_snapshot_id: active.as_ref().map(|snapshot| snapshot.id.clone()),
            rollback_available,
            update_available,
            ready: installed && blockers.is_empty() && operation.is_none(),
            options,
            applied_options,
            default_runtime_channel,
            external_workspace_root,
            addons,
            runtime_channels,
            user_layers,
            executables,
            warnings,
            blockers,
            operation,
            crash_report_ids,
            crash_reports,
        })
    }

    pub async fn game_set_options(
        &self,
        game: &QualifiedGameId,
        options: GameOptions,
    ) -> Result<FusionGameState, FusionError> {
        if self
            .inner
            .operations
            .lock()
            .map_err(|_| internal_error())?
            .contains_key(game)
        {
            return Err(FusionError::new(
                ErrorCode::OperationInProgress,
                "Options cannot change during a game operation.",
            ));
        }
        let manifest = self
            .resolve_manifest(game, &Cancellation::default())
            .await?;
        PlanBuilder::build(
            &manifest.canonical,
            &Self::plan_options(&options),
            &self.registry(),
        )?;
        self.inner.storage.set_options(game, &options)?;
        self.game_get_state(game).await
    }
}

fn enum_string<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

pub(crate) fn critical_inventory(
    snapshot: &WorkspaceSnapshot,
) -> Vec<(String, super::schema::CriticalFileSpec)> {
    let mut files = BTreeMap::new();
    for id in &snapshot.plan.selected_layers {
        if let Some(layer) = snapshot.manifest.layers.get(id) {
            let locator_files = match &layer.source {
                LayerSource::LocalLocator { locator } => locator.key_files.as_slice(),
                _ => &[],
            };
            let critical = match snapshot.inputs.get(id) {
                Some(LayerInput::Cas {
                    runtime_lock: Some(lock),
                    ..
                }) => &lock.critical_files,
                _ => &layer.critical_files,
            };
            for file in locator_files.iter().chain(critical) {
                let path = super::evolution::mounted_path(&layer.mount_path, &file.path);
                files.insert(path.to_lowercase(), (path, file.clone()));
            }
        }
    }
    files.into_values().collect()
}

struct OperationOwner {
    service: FusionService,
    game: QualifiedGameId,
    id: String,
    finished: bool,
}
impl OperationOwner {
    fn finish(mut self, result: Result<(), FusionError>) -> Result<OperationProgress, FusionError> {
        let progress =
            self.service
                .finish_operation(&self.game, &self.id, result.as_ref().err().cloned());
        self.finished = true;
        result?;
        progress
    }
}
impl Drop for OperationOwner {
    fn drop(&mut self) {
        if !self.finished {
            if let Ok(operations) = self.service.inner.operations.lock() {
                if let Some(operation) = operations
                    .get(&self.game)
                    .filter(|operation| operation.progress.operation_id == self.id)
                {
                    operation.cancel.cancel();
                }
            }
            let _ = self
                .service
                .finish_operation(&self.game, &self.id, Some(internal_error()));
        }
    }
}

impl FusionService {
    fn begin_operation(
        &self,
        game: &QualifiedGameId,
        kind: &str,
    ) -> Result<(OperationOwner, Cancellation), FusionError> {
        let _event_order = self
            .inner
            .event_order
            .lock()
            .map_err(|_| internal_error())?;
        let mut operations = self.inner.operations.lock().map_err(|_| internal_error())?;
        if operations.contains_key(game) {
            return Err(FusionError::new(
                ErrorCode::OperationInProgress,
                "This game already has an active operation.",
            ));
        }
        let record = self.inner.storage.begin_operation(game, kind)?;
        let cancel = Cancellation::default();
        let progress = OperationProgress {
            operation_id: record.id.clone(),
            source_id: game.source_id.clone(),
            game_id: game.game_id.clone(),
            layer_id: None,
            phase: kind.to_owned(),
            sequence: record.sequence,
            status: "running".into(),
            error: None,
            transfer: None,
        };
        operations.insert(
            game.clone(),
            ActiveOperation {
                cancel: cancel.clone(),
                progress: progress.clone(),
            },
        );
        drop(operations);
        self.inner.progress.emit(&progress);
        Ok((
            OperationOwner {
                service: self.clone(),
                game: game.clone(),
                id: record.id,
                finished: false,
            },
            cancel,
        ))
    }

    fn progress(
        &self,
        game: &QualifiedGameId,
        id: &str,
        phase: &str,
        layer: Option<&str>,
    ) -> Result<(), FusionError> {
        self.progress_with_transfer(game, id, phase, layer, None)
    }

    fn progress_with_transfer(
        &self,
        game: &QualifiedGameId,
        id: &str,
        phase: &str,
        layer: Option<&str>,
        transfer: Option<TransferProgress>,
    ) -> Result<(), FusionError> {
        let _event_order = self
            .inner
            .event_order
            .lock()
            .map_err(|_| internal_error())?;
        let event = {
            let mut operations = self.inner.operations.lock().map_err(|_| internal_error())?;
            let operation = operations
                .get_mut(game)
                .filter(|operation| operation.progress.operation_id == id)
                .ok_or_else(|| {
                    FusionError::new(ErrorCode::Cancelled, "The operation was superseded.")
                })?;
            if transfer.is_some() {
                operation.cancel.check()?;
            }
            let sequence = operation
                .progress
                .sequence
                .checked_add(1)
                .ok_or_else(internal_error)?;
            if !self.inner.storage.progress_operation(id, sequence, phase)? {
                return Err(FusionError::new(
                    ErrorCode::Cancelled,
                    "The operation is no longer current.",
                ));
            }
            operation.progress.sequence = sequence;
            operation.progress.phase = phase.to_owned();
            operation.progress.layer_id = layer.map(str::to_owned);
            operation.progress.transfer = transfer;
            operation.progress.clone()
        };
        self.inner.progress.emit(&event);
        Ok(())
    }

    fn finish_operation(
        &self,
        game: &QualifiedGameId,
        id: &str,
        error: Option<FusionError>,
    ) -> Result<OperationProgress, FusionError> {
        let _event_order = self
            .inner
            .event_order
            .lock()
            .map_err(|_| internal_error())?;
        let event = {
            let mut operations = self.inner.operations.lock().map_err(|_| internal_error())?;
            let Some(operation) = operations
                .get(game)
                .filter(|operation| operation.progress.operation_id == id)
            else {
                return Err(FusionError::new(
                    ErrorCode::Cancelled,
                    "The operation is no longer current.",
                ));
            };
            let status = match &error {
                Some(error) if error.code == ErrorCode::Cancelled => "cancelled",
                Some(_) => "failed",
                None => "completed",
            };
            self.inner.storage.finish_operation(id, status)?;
            let mut event = operation.progress.clone();
            event.sequence = event.sequence.saturating_add(1);
            event.phase = status.into();
            event.status = status.into();
            event.layer_id = None;
            event.transfer = None;
            event.error = error;
            operations.remove(game);
            event
        };
        self.inner.progress.emit(&event);
        Ok(event)
    }

    pub fn game_cancel(&self, game: &QualifiedGameId) -> Result<(), FusionError> {
        let current = self
            .inner
            .operations
            .lock()
            .map_err(|_| internal_error())?
            .get(game)
            .cloned();
        if let Some(operation) = current {
            operation.cancel.cancel();
            self.progress(game, &operation.progress.operation_id, "cancelling", None)?;
        }
        Ok(())
    }

    pub async fn game_install(
        &self,
        game: &QualifiedGameId,
        request: InstallRequest,
    ) -> Result<OperationProgress, FusionError> {
        self.install_operation(game, request, "install").await
    }
    pub async fn game_update(
        &self,
        game: &QualifiedGameId,
        request: InstallRequest,
    ) -> Result<OperationProgress, FusionError> {
        self.install_operation(game, request, "update").await
    }
    async fn install_operation(
        &self,
        game: &QualifiedGameId,
        request: InstallRequest,
        kind: &str,
    ) -> Result<OperationProgress, FusionError> {
        let (owner, cancel) = self.begin_operation(game, kind)?;
        // Persist root authority before the first network await. Commands routed
        // after a concurrent config change can still find this pending install.
        let old_record = self.inner.storage.get_installation(game)?;
        let external = request.external_workspace_root.as_deref().or_else(|| {
            old_record
                .as_ref()
                .and_then(|record| record.external_workspace_root.as_deref())
        });
        if let Err(error) = self.inner.storage.bind_installation_root(
            game,
            &self.inner.library_root,
            external.map(Path::new),
        ) {
            return owner.finish(Err(error));
        }
        let service = self.clone();
        let game = game.clone();
        // Dropping an IPC waiter cannot drop a blocking writer's ownership. The
        // dedicated runtime token remains available through game_cancel.
        tokio::spawn(async move {
            let mut result = service
                .run_install(&game, &owner.id, request, &cancel)
                .await;
            if let Err(error) = result {
                result = match service.repair_failed_install(&game).await {
                    Ok(()) => Err(error),
                    Err(repair) => Err(error.with_detail("recoveryError", repair)),
                };
            }
            owner.finish(result)
        })
        .await
        .map_err(|_| internal_error())?
    }

    async fn repair_failed_install(&self, game: &QualifiedGameId) -> Result<(), FusionError> {
        let Some(record) = self.inner.storage.get_installation(game)? else {
            return Ok(());
        };
        if record.status != "staging" && record.active_snapshot_id.is_none() {
            return Ok(());
        }
        // Every writer was awaited. Clear a pending root only under the same FS
        // lock that proves no snapshot was committed; keep candidate directories.
        self.inspect_snapshot(game, true).await?;
        Ok(())
    }

    async fn run_install(
        &self,
        game: &QualifiedGameId,
        operation_id: &str,
        request: InstallRequest,
        cancel: &Cancellation,
    ) -> Result<(), FusionError> {
        self.progress(game, operation_id, "manifest", None)?;
        let manifest = self.resolve_manifest(game, cancel).await?;
        let options = self.options(game)?;
        let mut plan_options = Self::plan_options(&options);
        let old_record = self.inner.storage.get_installation(game)?;
        let external = request.external_workspace_root.clone().or_else(|| {
            old_record
                .as_ref()
                .and_then(|record| record.external_workspace_root.clone())
        });
        let old_assembler = self.assembler(game, external.as_deref())?;
        let old = tokio::task::spawn_blocking(move || old_assembler.snapshot_for_update())
            .await
            .map_err(|_| internal_error())??;
        let mut plan = PlanBuilder::build(&manifest.canonical, &plan_options, &self.registry())?;
        for id in request.user_files.keys().chain(request.raw_paths.keys()) {
            if !plan.selected_layers.contains(id) {
                return Err(FusionError::new(
                    ErrorCode::UnknownAddon,
                    "Import request references an unselected layer.",
                )
                .with_detail("layerId", id));
            }
        }
        let mut inputs = BTreeMap::new();
        let mut dynamic = BTreeMap::new();
        for step in &plan.steps {
            if let PlanStep::Fetch(FetchStep::ResolveGithubRelease {
                layer_id,
                provider,
                required_lock,
            }) = step
            {
                self.progress(game, operation_id, "resolveRuntime", Some(layer_id))?;
                let client = Arc::new(SecureGithubReleaseClient::open(
                    &self.inner.library_root,
                    self.inner.transport.clone(),
                )?);
                let derived;
                let expected = if let Some(expected) = self.inner.attestations.get(game) {
                    expected
                } else {
                    // The source still supplies every expected digest. Discovery
                    // pins release metadata, never manufactures trust by hashing
                    // a previously unknown downloaded asset.
                    let layer = &manifest.canonical.layers[layer_id];
                    let release = client.release(provider, cancel).await?;
                    let mask = glob::Pattern::new(&provider.asset_mask).map_err(|_| {
                        FusionError::new(
                            ErrorCode::ProviderResolutionFailed,
                            "Invalid release asset pattern.",
                        )
                    })?;
                    let matches = release
                        .assets
                        .iter()
                        .filter(|asset| mask.matches(&asset.name))
                        .collect::<Vec<_>>();
                    if matches.len() != 1 {
                        return Err(FusionError::new(
                            ErrorCode::ProviderResolutionFailed,
                            "The provider must select exactly one asset.",
                        ));
                    }
                    let asset = matches[0];
                    derived = DynamicAssetAttestation {
                        release_id: release.id,
                        tag: release.tag,
                        asset_id: asset.id.clone(),
                        asset_url: asset.url.clone(),
                        transport_digest: layer.transport_digest.clone().ok_or_else(|| {
                            FusionError::new(
                                ErrorCode::InvalidDigest,
                                "The source must supply the expected dynamic transport digest.",
                            )
                        })?,
                        critical_files: layer.critical_files.clone(),
                    };
                    &derived
                };
                let lock = DynamicProviderResolver::new(client)
                    .resolve(provider, required_lock, Some(expected), cancel)
                    .await?;
                dynamic.insert(layer_id.clone(), lock);
            }
        }
        for id in &plan.selected_layers {
            cancel.check()?;
            self.progress(game, operation_id, "fetch", Some(id))?;
            let layer = &manifest.canonical.layers[id];
            let reusable = old
                .as_ref()
                .filter(|old| {
                    old.manifest.layers.get(id) == Some(layer)
                        && (layer.kind != LayerKind::Runtime
                            || old.plan.runtime_channel == plan.runtime_channel)
                })
                .and_then(|old| old.inputs.get(id))
                .filter(|_| !request.user_files.contains_key(id) && !dynamic.contains_key(id));
            if let Some(input) = reusable.filter(|input| matches!(input, LayerInput::Cas { .. })) {
                let input = input.clone();
                let library = self.inner.library_root.clone();
                let audit = input.clone();
                tokio::task::spawn_blocking(move || {
                    LayerCache::open(&library)?.object(&audit).map(drop)
                })
                .await
                .map_err(|_| internal_error())??;
                inputs.insert(id.clone(), input);
                continue;
            }
            match &layer.source {
                LayerSource::LocalLocator { locator } => {
                    let mut resolver = LocalLocatorResolver::new();
                    if let Some(inventory) = &self.inner.adapters.storefront {
                        resolver = resolver.with_inventory(inventory.clone());
                    }
                    let located = resolver
                        .resolve(locator, &self.inner.library_root, cancel)
                        .await?;
                    plan_options.local_base_volume = Some(located.volume_id.clone());
                    plan_options.library_volume = Some(located.library_volume_id.clone());
                    inputs.insert(id.clone(), located.layer_input());
                }
                LayerSource::UserProvided { archive, .. } => {
                    let path = request
                        .user_files
                        .get(id)
                        .map(PathBuf::from)
                        .ok_or_else(|| {
                            FusionError::new(
                                ErrorCode::MissingLayer,
                                "Select a user-owned file or directory for this layer.",
                            )
                            .with_detail("layerId", id)
                        })?;
                    if !path.is_absolute() {
                        return Err(FusionError::new(
                            ErrorCode::InvalidPath,
                            "User imports require an absolute selected path.",
                        ));
                    }
                    let raw = raw_path(layer, &request)?;
                    let layer = layer.clone();
                    let library = self.inner.library_root.clone();
                    let directory = path.is_dir();
                    let archive = *archive;
                    let input = tokio::task::spawn_blocking(move || {
                        let cache = LayerCache::open(&library)?;
                        if directory {
                            if archive != ArchiveFormat::Raw || layer.transport_digest.is_some() {
                                return Err(FusionError::new(
                                    ErrorCode::InvalidManifest,
                                    "A directory import cannot satisfy an archive digest.",
                                ));
                            }
                            cache.prepare_directory(&path)
                        } else {
                            cache.prepare_artifact(&layer, &path, raw.as_deref(), None)
                        }
                    })
                    .await
                    .map_err(|_| internal_error())??;
                    inputs.insert(id.clone(), input);
                }
                LayerSource::Remote { mirrors, .. } => {
                    let lock = dynamic.get(id);
                    let mirrors = lock
                        .map(|lock| vec![lock.asset_url.clone()])
                        .unwrap_or_else(|| mirrors.clone());
                    let digest = lock
                        .map(|lock| &lock.transport_digest)
                        .or(layer.transport_digest.as_ref())
                        .ok_or_else(|| {
                            FusionError::new(
                                ErrorCode::InvalidDigest,
                                "Remote delivery requires a transport digest.",
                            )
                        })?;
                    let stage = PrivateStage::create(&self.inner.library_root)?;
                    let artifact = stage.root.join("payload");
                    let max_bytes = layer
                        .size_bytes
                        .map(|size| size.saturating_add(1024 * 1024))
                        .unwrap_or(1024 * 1024 * 1024 * 1024);
                    self.inner
                        .adapters
                        .mirrors(self.inner.transport.clone())
                        .acquire_with_progress(
                            &mirrors,
                            digest,
                            &artifact,
                            &TransferLimits {
                                max_bytes,
                                timeout: Duration::from_secs(24 * 60 * 60),
                            },
                            cancel,
                            Some(Arc::new(OperationTransferSink {
                                service: self.clone(),
                                game: game.clone(),
                                operation_id: operation_id.to_owned(),
                                layer_id: id.clone(),
                            })),
                        )
                        .await?;
                    cancel.check()?;
                    self.progress(game, operation_id, "transform", Some(id))?;
                    let raw = raw_path(layer, &request)?;
                    let layer = layer.clone();
                    let library = self.inner.library_root.clone();
                    let runtime_lock = lock.map(|lock| lock.snapshot_lock());
                    // The private stage moves into the blocking owner and cannot be
                    // removed while extraction still reads the acquired container.
                    let input = tokio::task::spawn_blocking(move || {
                        let _stage = stage;
                        LayerCache::open(&library)?.prepare_artifact(
                            &layer,
                            &artifact,
                            raw.as_deref(),
                            runtime_lock,
                        )
                    })
                    .await
                    .map_err(|_| internal_error())??;
                    inputs.insert(id.clone(), input);
                }
            }
        }
        cancel.check()?;
        plan = PlanBuilder::build(&manifest.canonical, &plan_options, &self.registry())?;
        self.progress(game, operation_id, "assemble", None)?;
        let assembler = self.assembler(game, external.as_deref())?;
        let environment = &self.inner.environment;
        let context = TemplateContext {
            workspace_dir: self.inner.library_root.clone(),
            appdata_dir: environment.appdata.clone(),
            local_appdata_dir: environment.local_appdata.clone(),
            display_width: environment.display_width,
            display_height: environment.display_height,
            refresh_rate: environment.refresh_rate,
            enabled_addons: options.enabled_addons.unwrap_or_default(),
        };
        let persistence = PersistenceRoots {
            appdata: environment.appdata.clone(),
            local_appdata: environment.local_appdata.clone(),
        };
        let build_options = WorkspaceBuildOptions {
            verified_manifest: Some(manifest.binding),
            cancellation: Some(cancel.clone()),
            ..WorkspaceBuildOptions::default()
        };
        let canonical = manifest.canonical;
        let lease = tokio::task::spawn_blocking(move || {
            assembler.assemble(
                &canonical,
                &plan,
                &inputs,
                &context,
                &persistence,
                &build_options,
            )
        })
        .await
        .map_err(|_| internal_error())??;
        // Cancellation after activation must not claim that the committed version
        // disappeared. Record the completed snapshot before reporting completion.
        self.inner.storage.activate_snapshot(
            &lease.snapshot,
            &self.inner.library_root,
            external.as_deref().map(Path::new),
        )?;
        Ok(())
    }
}

fn raw_path(
    layer: &super::canonical::CanonicalLayer,
    request: &InstallRequest,
) -> Result<Option<String>, FusionError> {
    let archive = match &layer.source {
        LayerSource::Remote { archive, .. } | LayerSource::UserProvided { archive, .. } => *archive,
        _ => return Ok(None),
    };
    if archive != ArchiveFormat::Raw || layer.kind == LayerKind::Patch {
        return Ok(None);
    }
    if let Some(path) = request.raw_paths.get(&layer.id) {
        super::fs_safety::validate_relative_path(path)?;
        return Ok(Some(path.clone()));
    }
    if layer.critical_files.len() == 1 {
        return Ok(Some(layer.critical_files[0].path.clone()));
    }
    // Directory imports do not need a name; a raw file import is rejected by the
    // cache if no explicit destination is provided.
    Ok(None)
}

impl FusionService {
    pub async fn game_rollback(
        &self,
        game: &QualifiedGameId,
    ) -> Result<OperationProgress, FusionError> {
        let (owner, cancel) = self.begin_operation(game, "rollback")?;
        let service = self.clone();
        let game = game.clone();
        tokio::spawn(async move {
            let result = async {
                cancel.check()?;
                let external = service
                    .inner
                    .storage
                    .get_installation(&game)?
                    .and_then(|record| record.external_workspace_root);
                let assembler = service.assembler(&game, external.as_deref())?;
                service.progress(&game, &owner.id, "rollback", None)?;
                let lease = tokio::task::spawn_blocking(move || assembler.rollback())
                    .await
                    .map_err(|_| internal_error())??;
                service.inner.storage.activate_snapshot(
                    &lease.snapshot,
                    &service.inner.library_root,
                    external.as_deref().map(Path::new),
                )?;
                Ok(())
            }
            .await;
            owner.finish(result)
        })
        .await
        .map_err(|_| internal_error())?
    }

    pub async fn game_remove(
        &self,
        game: &QualifiedGameId,
    ) -> Result<OperationProgress, FusionError> {
        let (owner, cancel) = self.begin_operation(game, "remove")?;
        let service = self.clone();
        let game = game.clone();
        tokio::spawn(async move {
            let result = async {
                cancel.check()?;
                let external = service
                    .inner
                    .storage
                    .get_installation(&game)?
                    .and_then(|record| record.external_workspace_root);
                let assembler = service.assembler(&game, external.as_deref())?;
                service.progress(&game, &owner.id, "remove", None)?;
                // Ownership-based pruning can retain unknown files. Disconnection
                // keeps their snapshot references and all saves/CAS objects.
                let retained = tokio::task::spawn_blocking(move || assembler.remove_installation())
                    .await
                    .map_err(|_| internal_error())??;
                if !retained.is_empty() {
                    service.progress(&game, &owner.id, "preservedUnrecognizedFiles", None)?;
                }
                service.inner.storage.remove_installation(&game)?;
                Ok(())
            }
            .await;
            owner.finish(result)
        })
        .await
        .map_err(|_| internal_error())?
    }

    pub async fn game_launch(
        &self,
        game: &QualifiedGameId,
    ) -> Result<OperationProgress, FusionError> {
        let (owner, cancel) = self.begin_operation(game, "launch")?;
        let service = self.clone();
        let game = game.clone();
        tokio::spawn(async move {
            let result = service.run_launch(&game, &owner.id, &cancel).await;
            owner.finish(result)
        })
        .await
        .map_err(|_| internal_error())?
    }

    async fn run_launch(
        &self,
        game: &QualifiedGameId,
        operation_id: &str,
        cancel: &Cancellation,
    ) -> Result<(), FusionError> {
        cancel.check()?;
        let external = self
            .inner
            .storage
            .get_installation(game)?
            .and_then(|record| record.external_workspace_root);
        let assembler = self.assembler(game, external.as_deref())?;
        self.progress(game, operation_id, "integrity", None)?;
        let lease = tokio::task::spawn_blocking(move || assembler.active())
            .await
            .map_err(|_| internal_error())??
            .ok_or_else(|| {
                FusionError::new(
                    ErrorCode::SnapshotNotFound,
                    "Install the game before launching it.",
                )
            })?;
        let lease = if let Some(preparation) = self.inner.adapters.launch_preparation.clone() {
            tokio::task::spawn_blocking(move || {
                preparation.prepare(&lease)?;
                Ok::<_, FusionError>(lease)
            })
            .await
            .map_err(|_| internal_error())??
        } else {
            lease
        };
        self.inner.storage.activate_snapshot(
            &lease.snapshot,
            &self.inner.library_root,
            external.as_deref().map(Path::new),
        )?;
        cancel.check()?;
        let source = self.source(&game.source_id)?;
        let sync = Arc::new(ServiceSyncVerifier {
            bound: BoundManifestSyncVerifier {
                fetcher: Arc::new(self.fetcher()?),
                source_id: game.source_id.clone(),
                index_url: source.url,
                cancellation: cancel.clone(),
            },
            service: self.clone(),
            canonical_blake3: lease.snapshot.canonical_blake3.clone(),
        });
        let running = ProcessSupervisor::launch(
            lease,
            Arc::new(StorageTrust(self.inner.storage.clone())),
            Some(sync),
            SupervisorOptions::default(),
        )
        .await?;
        let process_cancel = running.cancellation_handle();
        if let Err(error) = self.progress(game, operation_id, "running", None) {
            process_cancel.cancel();
            // Do not release the service operation while the ownership worker
            // still waits for descendants or reconciles their final save writes.
            let _ = running.wait().await;
            return Err(error);
        }
        let completion = running.wait();
        tokio::pin!(completion);
        let outcome = tokio::select! {
            result = &mut completion => result?,
            () = cancel.cancelled() => { process_cancel.cancel(); completion.await? }
        };
        if let Some(report) = outcome.sanitized_crash_report {
            self.inner.storage.put_crash(&report)?;
        }
        if outcome.user_cancelled {
            return Err(FusionError::new(
                ErrorCode::Cancelled,
                "The process group was stopped and its saves were reconciled.",
            ));
        }
        Ok(())
    }

    pub async fn game_trust_executable(
        &self,
        game: &QualifiedGameId,
        path: &str,
        blake3: &str,
    ) -> Result<(), FusionError> {
        self.game_trust_executables(game, vec![(path.to_owned(), blake3.to_owned())])
            .await
    }

    /// Approve an explicit host-provided list under one verified workspace lease.
    /// Validate every requested digest before persisting any approval; this avoids
    /// rehashing a multi-gigabyte local base once per runtime/key file.
    pub async fn game_trust_executables(
        &self,
        game: &QualifiedGameId,
        files: Vec<(String, String)>,
    ) -> Result<(), FusionError> {
        if files.is_empty() || files.len() > 64 {
            return Err(FusionError::new(
                ErrorCode::UntrustedExecutable,
                "Trust requires 1–64 explicit path/digest pairs.",
            ));
        }
        for (path, digest) in &files {
            super::fs_safety::validate_relative_path(path)?;
            super::cas::validate_hash(digest)?;
        }
        if self
            .inner
            .operations
            .lock()
            .map_err(|_| internal_error())?
            .contains_key(game)
        {
            return Err(FusionError::new(
                ErrorCode::OperationInProgress,
                "Executable trust cannot change during an active operation.",
            ));
        }
        let external = self
            .inner
            .storage
            .get_installation(game)?
            .and_then(|record| record.external_workspace_root);
        let assembler = self.assembler(game, external.as_deref())?;
        let game = game.clone();
        let storage = self.inner.storage.clone();
        tokio::task::spawn_blocking(move || {
            let lease = assembler.active()?.ok_or_else(|| {
                FusionError::new(
                    ErrorCode::SnapshotNotFound,
                    "Trust requires an installed snapshot.",
                )
            })?;
            let inventory = critical_inventory(&lease.snapshot);
            let mut approved_files = Vec::new();
            for (path, hash) in &files {
                let approved = inventory
                    .iter()
                    .find(|(candidate, file)| {
                        candidate.eq_ignore_ascii_case(path)
                            && file.blake3.eq_ignore_ascii_case(hash)
                    })
                    .ok_or_else(|| {
                        FusionError::new(
                            ErrorCode::UntrustedExecutable,
                            "The requested digest is not a current selected critical file.",
                        )
                    })?;
                approved_files.push((approved.0.clone(), hash.to_ascii_lowercase()));
            }
            // The exclusive lease remains live while rehashing and recording the
            // exact user-approved digest, including local-locator projections.
            PreLaunchIntegrityGuard::verify_integrity(&lease.snapshot)?;
            for (path, hash) in approved_files {
                storage.trust_executable(&game, &path, &hash)?;
            }
            Ok(())
        })
        .await
        .map_err(|_| internal_error())?
    }

    pub fn crash_preview(&self, id: &str) -> Result<CrashPreview, FusionError> {
        let record = self.inner.storage.get_crash(id)?.ok_or_else(|| {
            FusionError::new(
                ErrorCode::SnapshotNotFound,
                "The crash report was not found.",
            )
        })?;
        let endpoint = self
            .source(&record.game.source_id)
            .ok()
            .and_then(|source| source.index.source_info.telemetry_endpoint);
        let can_send = record.status == "pending" && endpoint.is_some();
        Ok(CrashPreview {
            report_id: record.id,
            report: record.report,
            status: record.status,
            endpoint,
            can_send,
        })
    }

    /// A host calls this only for the user's explicit Send action. The body is
    /// generated from locally sanitized storage; caller-supplied JSON is absent.
    pub fn crash_send_target(&self, id: &str) -> Result<CrashExport, FusionError> {
        let preview = self.crash_preview(id)?;
        if !preview.can_send {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "This report has no available delivery target or was already handled.",
            ));
        }
        let endpoint = preview.endpoint.ok_or_else(|| {
            FusionError::new(
                ErrorCode::InvalidUri,
                "The source declares no telemetry endpoint.",
            )
        })?;
        super::adapters::https_url(&endpoint)?;
        let body = serde_json::to_vec(&preview.report).map_err(|_| internal_error())?;
        if body.len() > 1024 * 1024 {
            return Err(FusionError::new(
                ErrorCode::TransferLimitExceeded,
                "The sanitized report exceeds the export budget.",
            ));
        }
        Ok(CrashExport {
            report_id: id.to_owned(),
            endpoint,
            body,
        })
    }
    pub fn crash_mark_sent(&self, id: &str) -> Result<(), FusionError> {
        self.inner.storage.set_crash_status(id, "sent")
    }
    pub fn crash_discard(&self, id: &str) -> Result<(), FusionError> {
        self.inner.storage.set_crash_status(id, "discarded")
    }
}

/// Construction is private: the IPC adapter may inspect an authorized export but
/// cannot substitute unsanitized logs, an arbitrary payload or destination.
pub struct CrashExport {
    report_id: String,
    endpoint: String,
    body: Vec<u8>,
}
impl CrashExport {
    pub fn report_id(&self) -> &str {
        &self.report_id
    }
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

struct ServiceSyncVerifier {
    bound: BoundManifestSyncVerifier,
    service: FusionService,
    canonical_blake3: String,
}
#[async_trait::async_trait]
impl ManifestSyncVerifier for ServiceSyncVerifier {
    async fn verify_current(
        &self,
        game: &QualifiedGameId,
        manifest_blake3: &str,
    ) -> Result<(), FusionError> {
        self.bound.verify_current(game, manifest_blake3).await?;
        // Only the root and its ancestors need fresh indices. An unrelated
        // server cannot block synchronization of this game's inherited layers.
        let resolved = self
            .service
            .resolve_manifest_with_freshness(game, &self.bound.cancellation, true)
            .await?;
        let hash =
            blake3::hash(&serde_json::to_vec(&resolved.canonical).map_err(|_| internal_error())?)
                .to_hex()
                .to_string();
        if hash != self.canonical_blake3 {
            return Err(FusionError::new(
                ErrorCode::StrictSyncRequired,
                "The current inherited manifest differs from the active snapshot.",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "service_host_tests.rs"]
mod host_tests;
