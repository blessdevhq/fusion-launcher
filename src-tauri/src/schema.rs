use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryMetadata {
    pub id: String,
    pub name: String,
    pub version: String,
    pub schema_version: u8,
    pub maintainer: Option<String>,
    pub homepage_url: Option<String>,
    pub license: Option<String>,
    pub trust_level: Option<String>,
    pub content_hash: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RepositorySchema {
    pub metadata: RepositoryMetadata,
    pub system_files: Vec<RepositoryAsset>,
    pub catalog: Vec<RepositoryGame>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SourceUri {
    Http {
        url: String,
        sha256: String,
        #[serde(rename = "sizeBytes")]
        size_bytes: Option<u64>,
    },
    Magnet {
        uri: String,
        #[serde(rename = "infoHash")]
        info_hash: Option<String>,
        #[serde(rename = "sizeBytes")]
        size_bytes: Option<u64>,
    },
    #[serde(rename = "user_provided")]
    UserProvided {
        instructions: Option<String>,
        sha256: Option<String>,
        #[serde(rename = "sizeBytes")]
        size_bytes: Option<u64>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Emulator,
    Bios,
    Firmware,
    Keys,
    Patch,
    Runtime,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallTarget {
    AppSystem,
    EmulatorDir,
    UserSelected,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallHint {
    pub target: InstallTarget,
    pub relative_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryAsset {
    pub id: String,
    pub platform: String,
    pub asset_kind: AssetKind,
    pub display_name: String,
    pub sources: Vec<SourceUri>,
    pub install_hint: Option<InstallHint>,
    #[serde(default)]
    pub executable: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryGame {
    pub id: String,
    pub platform: String,
    pub title: String,
    pub description: Option<String>,
    pub cover_image_url: Option<String>,
    pub trailer_url: Option<String>,
    pub downloads: Vec<SourceUri>,
    pub expected_extensions: Vec<String>,
    #[serde(default)]
    pub required_system_file_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub url: String,
    pub connected_at: String,
    pub catalog_count: usize,
    pub system_file_count: usize,
    pub maintainer: Option<String>,
    pub homepage_url: Option<String>,
    pub license: Option<String>,
    pub trust_level: String,
    pub content_hash: Option<String>,
    pub last_refreshed_at: Option<String>,
    pub has_executable_assets: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryPreview {
    pub url: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub maintainer: Option<String>,
    pub homepage_url: Option<String>,
    pub license: Option<String>,
    pub trust_level: String,
    pub catalog_count: usize,
    pub system_file_count: usize,
    pub has_executable_assets: bool,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogGameView {
    pub id: String,
    pub source_id: String,
    pub repository_id: String,
    pub repository_name: String,
    pub platform: String,
    pub title: String,
    pub description: Option<String>,
    pub cover_image_url: Option<String>,
    pub trailer_url: Option<String>,
    pub downloads: Vec<SourceUri>,
    pub expected_extensions: Vec<String>,
    pub required_system_file_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetView {
    pub id: String,
    pub source_id: String,
    pub repository_id: String,
    pub platform: String,
    pub asset_kind: AssetKind,
    pub display_name: String,
    pub sources: Vec<SourceUri>,
    pub install_hint: Option<InstallHint>,
    pub executable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementItem {
    pub asset: AssetView,
    pub status: String,
    pub downloaded: bool,
    pub trusted: bool,
    pub local_path: Option<String>,
    pub target_path: Option<String>,
    pub sha256: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementsReport {
    pub game_id: String,
    pub ready: bool,
    pub game_downloaded: bool,
    pub requirements: Vec<RequirementItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryGameStatus {
    pub game_id: String,
    pub installed: bool,
    pub system_requirements_ready: bool,
    pub missing_requirements: Vec<String>,
    pub download: Option<TorrentDownloadRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRecord {
    pub subject_id: String,
    pub subject_type: String,
    pub status: String,
    pub local_path: Option<String>,
    pub sha256: Option<String>,
    pub message: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameDownloadStartReport {
    pub game_id: String,
    pub source_kind: String,
    pub save_dir: String,
    pub record: Option<DownloadRecord>,
    pub torrent: Option<TorrentDownloadRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentDownloadRecord {
    pub game_id: String,
    pub magnet_uri: String,
    pub save_dir: String,
    pub status: String,
    pub progress_percent: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub download_speed_bytes_per_sec: u64,
    pub upload_speed_bytes_per_sec: u64,
    pub peers_count: usize,
    pub torrent_id: Option<i64>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedExecutable {
    pub asset_id: String,
    pub local_path: String,
    pub sha256: String,
    pub trusted_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmulatorConfig {
    pub platform: String,
    pub exe_path: Option<String>,
    pub status: String,
    pub last_validated_at: Option<String>,
    pub version: Option<String>,
    pub launch_args_template: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetInstallation {
    pub asset_id: String,
    pub target_path: Option<String>,
    pub status: String,
    pub sha256: Option<String>,
    pub verified_at: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingState {
    pub step: String,
    pub repositories_configured: bool,
    pub emulators_configured: bool,
    pub catalog_count: usize,
    pub valid_emulator_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub generated_at: String,
    pub emulators: Vec<HealthCheckItem>,
    pub system_files: Vec<HealthCheckItem>,
    pub game_files: Vec<HealthCheckItem>,
    pub repositories: Vec<HealthCheckItem>,
    pub downloader: HealthCheckItem,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheckItem {
    pub id: String,
    pub label: String,
    pub status: String,
    pub message: Option<String>,
    pub action: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsBundle {
    pub generated_at: String,
    pub app_version: String,
    pub os: String,
    pub data_dir: String,
    pub log_path: String,
    pub health: HealthReport,
    pub downloads: Vec<TorrentDownloadRecord>,
    pub logs: Vec<String>,
}
