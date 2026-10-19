use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use crate::fusion_source::adapters::Cancellation;
use crate::fusion_source::cas::validate_hash;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::fs_safety;
use crate::fusion_source::layers::LayerInput;
use crate::fusion_source::schema::{LocalLocatorSpec, LocalStore};

struct CancelHashOnDrop(Cancellation);
impl Drop for CancelHashOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[async_trait]
pub trait StorefrontInventory: Send + Sync {
    /// Read-only inventory integration. Never grants write authority over any
    /// candidate root and never starts the storefront or installs a game.
    async fn candidates(
        &self,
        store: LocalStore,
        app_id: Option<&str>,
        cancel: &Cancellation,
    ) -> Result<Vec<PathBuf>, FusionError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatedBase {
    pub root: PathBuf,
    pub volume_id: String,
    pub library_volume_id: String,
}

impl LocatedBase {
    pub fn layer_input(&self) -> LayerInput {
        LayerInput::LocalDirectory {
            root: self.root.clone(),
        }
    }
}

pub struct LocalLocatorResolver {
    inventory: Option<Arc<dyn StorefrontInventory>>,
}

impl Default for LocalLocatorResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalLocatorResolver {
    pub fn new() -> Self {
        Self { inventory: None }
    }
    pub fn with_inventory(mut self, inventory: Arc<dyn StorefrontInventory>) -> Self {
        self.inventory = Some(inventory);
        self
    }

    pub async fn resolve(
        &self,
        locator: &LocalLocatorSpec,
        library_root: &Path,
        cancel: &Cancellation,
    ) -> Result<LocatedBase, FusionError> {
        cancel.check()?;
        if locator.key_files.is_empty() {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Local discovery requires a critical key-file inventory.",
            ));
        }
        let _library = fs_safety::guard(library_root)?;
        let library_volume_id = fs_safety::file_identity(library_root)?.volume.to_string();
        let mut candidates = locator
            .path_hints
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        if let Some(inventory) = &self.inventory {
            candidates.extend(
                inventory
                    .candidates(locator.store, locator.app_id.as_deref(), cancel)
                    .await?,
            );
        }
        let mut seen = BTreeSet::new();
        let mut failures = 0usize;
        for candidate in candidates {
            cancel.check()?;
            if !seen.insert(candidate.to_string_lossy().to_ascii_lowercase()) {
                continue;
            }
            let keys = locator.key_files.clone();
            let cancellation = cancel.clone();
            let library_volume = library_volume_id.clone();
            let abort = Cancellation::default();
            let abort_guard = CancelHashOnDrop(abort.clone());
            let result =
                tokio::task::spawn_blocking(move || -> Result<LocatedBase, FusionError> {
                    let _root = fs_safety::guard(&candidate)?;
                    for key in &keys {
                        cancellation.check()?;
                        abort.check()?;
                        let relative = fs_safety::validate_relative_path(&key.path)?;
                        validate_hash(&key.blake3)?;
                        let path = candidate.join(relative);
                        let mut file = fs_safety::open_read(&path)?;
                        let mut hash = blake3::Hasher::new();
                        let mut buffer = [0u8; 64 * 1024];
                        loop {
                            cancellation.check()?;
                            abort.check()?;
                            let size = file.read(&mut buffer).map_err(|error| {
                                fs_safety::io_error("Verify local installation", &path, error)
                            })?;
                            if size == 0 {
                                break;
                            }
                            hash.update(&buffer[..size]);
                        }
                        if !hash
                            .finalize()
                            .to_hex()
                            .as_str()
                            .eq_ignore_ascii_case(&key.blake3)
                        {
                            return Err(FusionError::new(
                                ErrorCode::LocalBaseModified,
                                "The local installation's key-file digest changed.",
                            ));
                        }
                    }
                    Ok(LocatedBase {
                        volume_id: fs_safety::file_identity(&candidate)?.volume.to_string(),
                        root: candidate,
                        library_volume_id: library_volume,
                    })
                })
                .await
                .map_err(|_| {
                    FusionError::new(
                        ErrorCode::Io,
                        "The local installation verification worker failed.",
                    )
                })?;
            drop(abort_guard);
            match result {
                Ok(found) => return Ok(found),
                Err(error) if error.code == ErrorCode::Cancelled => return Err(error),
                Err(_) => {
                    failures += 1;
                }
            }
        }
        Err(FusionError::new(
            ErrorCode::LocalBaseModified,
            "No discovered local installation matches the required key files.",
        )
        .with_detail("candidatesRejected", failures))
    }
}
