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
    Bundled {
        path: String,
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
    #[serde(default)]
    pub artwork: Option<GameArtwork>,
    #[serde(default)]
    pub metadata: Option<GameMetadata>,
    #[serde(default)]
    pub content_mode: Option<String>,
    /// Reserved for the future Platform Setup Profiles step.
    #[serde(default, alias = "platformProfileId")]
    pub setup_profile_id: Option<String>,
    pub downloads: Vec<SourceUri>,
    #[serde(default)]
    pub expected_extensions: Vec<String>,
    #[serde(default)]
    pub required_system_file_ids: Vec<String>,
    #[serde(default)]
    pub launch: Option<GameLaunchConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameLaunchConfig {
    pub args_template: Option<String>,
    pub preferred_file: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameArtwork {
    pub cover: Option<String>,
    pub hero: Option<String>,
    pub logo: Option<String>,
    #[serde(default)]
    pub screenshots: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameMetadata {
    pub release_year: Option<u16>,
    pub developer: Option<String>,
    pub publisher: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub players: Option<String>,
    pub series: Option<String>,
    #[serde(default)]
    pub external_ids: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualGameMetadataInput {
    pub title: Option<String>,
    pub description: Option<String>,
    pub cover: Option<String>,
    pub hero: Option<String>,
    pub logo: Option<String>,
    #[serde(default)]
    pub screenshots: Vec<String>,
    pub metadata: Option<GameMetadata>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapedGamePayload {
    pub provider: String,
    pub provider_game_id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub cover: Option<String>,
    #[serde(default)]
    pub hero: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub screenshots: Vec<String>,
    pub metadata: GameMetadata,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeCandidate {
    pub provider: String,
    pub provider_game_id: String,
    pub title: String,
    pub platform: Option<String>,
    pub release_year: Option<u16>,
    pub developer: Option<String>,
    pub cover: Option<String>,
    pub match_kind: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeStateView {
    pub game_id: String,
    pub status: String,
    pub match_kind: Option<String>,
    pub candidates: Vec<ScrapeCandidate>,
    pub message: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenScraperStatus {
    pub configured: bool,
    pub ssid: Option<String>,
    pub region: String,
    pub daily_requests: u32,
    pub daily_limit: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamGridDbStatus {
    pub configured: bool,
    pub key_source: String,
    pub daily_requests: u32,
    pub daily_limit: u32,
    pub pending_batch: usize,
    pub batch_running: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryScrapeStatus {
    pub running: bool,
    pub pending: usize,
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
    pub artwork: Option<GameArtwork>,
    pub metadata: Option<GameMetadata>,
    pub content_mode: Option<String>,
    /// Reserved for the future Platform Setup Profiles step.
    pub setup_profile_id: Option<String>,
    pub downloads: Vec<SourceUri>,
    pub expected_extensions: Vec<String>,
    pub required_system_file_ids: Vec<String>,
    pub launch: Option<GameLaunchConfig>,
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
    pub checksum: Option<String>,
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
    pub source: String,
    pub magnet_uri: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAssetFileReport {
    pub status: String,
    pub installed_path: String,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGameFileReport {
    pub status: String,
    pub game_id: String,
    pub installed_path: String,
    pub sha256: Option<String>,
    pub error_code: Option<String>,
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
pub struct ProfileEmulatorConfig {
    pub profile_id: String,
    pub platform: String,
    pub exe_path: Option<String>,
    pub status: String,
    pub last_validated_at: Option<String>,
    pub version: Option<String>,
    pub launch_args_template: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSystemFileImport {
    pub profile_id: String,
    pub requirement_id: String,
    pub target_path: Option<String>,
    pub status: String,
    pub sha256: Option<String>,
    pub verified_at: String,
    pub message: Option<String>,
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
    pub platform_setup: Vec<HealthCheckItem>,
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
pub struct PlatformSetupProfile {
    pub id: String,
    pub platform: String,
    pub display_name: String,
    pub emulator: PlatformSetupEmulator,
    pub game_files: PlatformSetupGameFiles,
    pub system_files: Vec<ProfileSystemFileRequirement>,
    pub launch: PlatformSetupLaunch,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformSetupEmulator {
    pub install_mode: String,
    pub emulator_name: String,
    pub executable_name: Option<String>,
    pub executable_candidates: Vec<String>,
    pub download: Option<ProfileEmulatorDownload>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileEmulatorDownload {
    pub url: String,
    pub sha256: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformSetupGameFiles {
    pub expected_extensions: Vec<String>,
    pub allow_directory: bool,
    pub preferred_file_patterns: Vec<String>,
    pub validators: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSystemFileRequirement {
    pub id: String,
    pub label: String,
    pub asset_kind: String,
    pub required: bool,
    pub extensions: Vec<String>,
    pub target_name: Option<String>,
    pub checksum: Option<String>,
    pub source_mode: String,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformSetupLaunch {
    pub args_template: String,
    pub working_directory: Option<String>,
    pub preferred_file: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSetupState {
    pub game_id: String,
    pub profile_id: Option<String>,
    pub profile_display_name: Option<String>,
    pub unsupported_profile_id: Option<String>,
    pub emulator: GameSetupEmulatorState,
    pub system_files: Vec<GameSetupSystemFileState>,
    pub repository_requirements: Vec<RequirementItem>,
    pub game_file: GameSetupGameFileState,
    pub launch: GameSetupLaunchState,
    pub primary_action: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSetupEmulatorState {
    pub status: String,
    pub profile_id: Option<String>,
    pub platform: String,
    pub emulator_name: String,
    pub install_mode: String,
    pub executable_path: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSetupSystemFileState {
    pub id: String,
    pub label: String,
    pub asset_kind: String,
    pub required: bool,
    pub status: String,
    pub installed_path: Option<String>,
    pub expected_extensions: Vec<String>,
    pub checksum: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSetupGameFileState {
    pub status: String,
    pub installed_path: Option<String>,
    pub expected_extensions: Vec<String>,
    pub allow_directory: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSetupLaunchState {
    pub status: String,
    pub blockers: Vec<String>,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsPaths {
    pub data_dir: String,
    pub log_path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scraped_payload_deserializes_without_new_artwork_fields() {
        let payload: ScrapedGamePayload = serde_json::from_str(
            r#"{
              "provider": "screenscraper",
              "providerGameId": "123",
              "title": "Game",
              "description": null,
              "cover": "https://example.com/cover.jpg",
              "metadata": {
                "releaseYear": null,
                "developer": null,
                "publisher": null,
                "genres": [],
                "tags": [],
                "players": null,
                "series": null,
                "externalIds": {}
              }
            }"#,
        )
        .unwrap();

        assert!(payload.hero.is_none());
        assert!(payload.logo.is_none());
        assert!(payload.screenshots.is_empty());
    }
}
