use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, State};
use url::Url;

use crate::builtin_demo;
use crate::downloads::{
    destination_for_source, download_source_to_file, file_name_for_source, hash_file,
};
use crate::game_files;
use crate::logging;
use crate::schema::{
    AssetView, CatalogGameView, DiagnosticsBundle, DiagnosticsPaths, DownloadRecord,
    EmulatorConfig, GameDownloadStartReport, GameSetupEmulatorState, GameSetupGameFileState,
    GameSetupLaunchState, GameSetupState, GameSetupSystemFileState, HealthCheckItem, HealthReport,
    ImportAssetFileReport, ImportGameFileReport, InstallTarget, LibraryGameStatus,
    LibraryScrapeStatus, ManualGameMetadataInput, OnboardingState, PlatformSetupProfile,
    ProfileEmulatorConfig, ProfileSystemFileRequirement, RepositoryGame, RepositoryMetadata,
    RepositoryPreview, RepositorySchema, RepositorySummary, RequirementItem, RequirementsReport,
    ScrapeCandidate, ScrapeStateView, ScreenScraperStatus, SourceUri, SteamGridDbStatus,
    TorrentDownloadRecord, TrustedExecutable,
};
use crate::security::{validate_platform, validate_repository_schema, validate_repository_url};
use crate::setup_profiles;
use crate::storage::RepositoryStore;
use crate::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairLibraryReport {
    pub repaired: bool,
    pub repository_id: Option<String>,
    pub removed_paths: Vec<String>,
}

include!("reports.rs");
include!("setup_helpers.rs");
include!("status_helpers.rs");
include!("import_helpers.rs");
include!("repository_helpers.rs");
include!("runtime_helpers.rs");
include!("diagnostics_helpers.rs");
include!("shared.rs");
include!("library.rs");
include!("metadata.rs");
include!("setup.rs");
include!("emulators.rs");
include!("downloads.rs");
include!("settings.rs");
include!("diagnostics.rs");
include!("smoke.rs");

#[cfg(test)]
mod tests;
