//! Desktop-owned directory bindings. Only an explicit host import grants local
//! transport authority; publisher URLs never select arbitrary filesystem paths.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;

use crate::fusion_source::adapters::{
    ArtifactTransport, Cancellation, SecureHttpsTransport, TransferLimits, TransferProgressSink,
    TransferReceipt,
};
use crate::fusion_source::dto::{SourcePreview, SourceSummary};
use crate::fusion_source::offline::OfflineTransport;
use crate::fusion_source::service::{
    FusionService, LaunchPreparation, NoProgress, ServiceEnvironment,
};
use crate::fusion_source::{fs_safety, ErrorCode, FusionError};
use crate::storage::fusion_source::SqliteFusionStorage;

#[derive(Clone, Debug)]
pub struct LocalSourceBinding {
    pub directory: String,
    pub prepare_eden: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSourceSummary {
    #[serde(flatten)]
    pub source: SourceSummary,
    pub local_directory: Option<String>,
    pub prepare_eden: bool,
}
impl DesktopSourceSummary {
    pub fn new(source: SourceSummary, storage: &SqliteFusionStorage) -> Result<Self, FusionError> {
        let binding = storage.local_source(&source.source_id)?;
        Ok(Self {
            source,
            local_directory: binding.as_ref().map(|b| b.directory.clone()),
            prepare_eden: binding.is_some_and(|b| b.prepare_eden),
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSourcePreview {
    #[serde(flatten)]
    pub source: SourcePreview,
    pub local_directory: String,
}

pub struct LocalSourceCandidate {
    pub binding: LocalSourceBinding,
    pub url: String,
    service: FusionService,
}
impl LocalSourceCandidate {
    pub fn open(
        directory: &Path,
        library: &Path,
        storage: Arc<SqliteFusionStorage>,
        environment: ServiceEnvironment,
    ) -> Result<Self, FusionError> {
        let directory = std::path::absolute(directory)
            .map_err(|_| invalid("Invalid local source directory."))?;
        let _guard = fs_safety::guard(&directory)?;
        let mut bytes = Vec::new();
        fs_safety::open_read(&directory.join("source-index.json"))?
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid("Cannot read source-index.json."))?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(invalid("Local source index is too large."));
        }
        // This header selects only the synthetic identity. The service fetcher
        // still validates the full index/signature against stored pinned keys.
        let header: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid source-index.json."))?;
        let source_id = header
            .pointer("/source_info/id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| invalid("Missing source ID."))?;
        let transport = Arc::new(OfflineTransport::open(&directory, source_id)?);
        let url = transport.index_url();
        let service = FusionService::open(
            library,
            storage,
            transport,
            Arc::new(NoProgress),
            environment,
        )?;
        Ok(Self {
            binding: LocalSourceBinding {
                directory: directory.to_string_lossy().into_owned(),
                prepare_eden: false,
            },
            url,
            service,
        })
    }
    pub async fn preview(&self) -> Result<LocalSourcePreview, FusionError> {
        let source = self.service.source_preview(&self.url).await?;
        self.check_identity(&source.source_id)?;
        Ok(LocalSourcePreview {
            source,
            local_directory: self.binding.directory.clone(),
        })
    }
    pub async fn connect(
        mut self,
        storage: &SqliteFusionStorage,
        expected_source_id: &str,
        prepare_eden: bool,
    ) -> Result<DesktopSourceSummary, FusionError> {
        let index = self.service.resolved_index(&self.url).await?;
        self.check_identity(&index.source_info.id)?;
        if index.source_info.id != expected_source_id {
            return Err(FusionError::new(ErrorCode::SnapshotConflict, "The selected directory belongs to a different source. Preview it as a new source instead."));
        }
        self.binding.prepare_eden = prepare_eden;
        let record = storage.connect_local_source(&index, &self.binding)?;
        DesktopSourceSummary::new(
            SourceSummary {
                source_id: record.index.source_info.id,
                name: record.index.source_info.name,
                url: record.url,
                game_count: record.index.games.len(),
                public_key: record.index.source_info.public_key,
                refreshed_at: record.refreshed_at,
            },
            storage,
        )
    }
    fn check_identity(&self, source_id: &str) -> Result<(), FusionError> {
        if self.url != format!("https://offline.fusion.invalid/{source_id}/source-index.json") {
            return Err(FusionError::new(
                ErrorCode::SnapshotConflict,
                "The source identity changed while reading the selected folder. Choose it again.",
            ));
        }
        Ok(())
    }
}

fn invalid(message: &str) -> FusionError {
    FusionError::new(ErrorCode::InvalidManifest, message)
}

pub struct DesktopSourceTransport {
    storage: Arc<SqliteFusionStorage>,
    network: Arc<dyn ArtifactTransport>,
}
impl DesktopSourceTransport {
    pub fn new(storage: Arc<SqliteFusionStorage>) -> Self {
        Self {
            storage,
            network: Arc::new(SecureHttpsTransport),
        }
    }
    fn local(&self, value: &str) -> Result<Option<OfflineTransport>, FusionError> {
        let url = url::Url::parse(value).map_err(|_| invalid("Invalid source URL."))?;
        if url.host_str().is_none_or(|host| {
            !host
                .trim_end_matches('.')
                .eq_ignore_ascii_case("offline.fusion.invalid")
        }) {
            return Ok(None);
        }
        let source_id = url
            .path_segments()
            .and_then(|mut s| s.next())
            .ok_or_else(|| invalid("Missing local source identity."))?;
        let binding = self.storage.local_source(source_id)?.ok_or_else(|| {
            FusionError::new(
                ErrorCode::InvalidUri,
                "This local source is not connected. Choose its folder in Sources.",
            )
        })?;
        Ok(Some(OfflineTransport::open(
            &PathBuf::from(binding.directory),
            source_id,
        )?))
    }
}
#[async_trait]
impl ArtifactTransport for DesktopSourceTransport {
    async fn fetch(
        &self,
        url: &str,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
    ) -> Result<TransferReceipt, FusionError> {
        self.fetch_with_progress(url, destination, limits, cancel, None)
            .await
    }
    async fn fetch_with_progress(
        &self,
        url: &str,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
        progress: Option<Arc<dyn TransferProgressSink>>,
    ) -> Result<TransferReceipt, FusionError> {
        match self.local(url)? {
            Some(local) => {
                local
                    .fetch_with_progress(url, destination, limits, cancel, progress)
                    .await
            }
            None => {
                self.network
                    .fetch_with_progress(url, destination, limits, cancel, progress)
                    .await
            }
        }
    }
}

pub struct LocalProfilePreparation(pub Arc<SqliteFusionStorage>);
impl LaunchPreparation for LocalProfilePreparation {
    fn prepare(
        &self,
        workspace: &crate::fusion_source::workspace::AssembledWorkspace,
    ) -> Result<(), FusionError> {
        if self
            .0
            .local_source(&workspace.snapshot.game.source_id)?
            .is_some_and(|b| b.prepare_eden)
        {
            crate::fusion_source::offline::prepare_eden_workspace(workspace)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "local_sources_tests.rs"]
mod tests;
