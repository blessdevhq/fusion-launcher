import { invoke } from '@tauri-apps/api/core';
import { previewApi } from './previewData.ts';
import { isPreviewRuntime, isTauriRuntime, requireDesktopBridge } from './runtime.ts';
import type {
  EmulatorInstallResult,
  EmulatorStatus,
  InstallResult
} from '@/types/emulatorProfile';
import type {
  CatalogGame,
  DiagnosticsBundle,
  DiagnosticsPaths,
  DownloadRecord,
  EmulatorConfig,
  GameSetupState,
  GameDownloadStartReport,
  HealthReport,
  ImportAssetFileReport,
  ImportGameFileReport,
  LibraryScrapeStatus,
  LibraryGameStatus,
  ManualGameMetadataInput,
  LaunchReport,
  OnboardingState,
  PlatformSetupProfile,
  ProfileEmulatorConfig,
  RepairLibraryReport,
  RepositoryPreview,
  RepositorySummary,
  RequirementsReport,
  ScrapeCandidate,
  ScrapeState,
  ScreenScraperStatus,
  SteamGridDbStatus,
  TorrentDownloadRecord,
  TorrentStartReport,
  TorrentStatus,
  TrustedExecutable,
  UpdateCheckReport
} from '@/types/repository';

type PreviewHandler = (args: Record<string, unknown>) => Promise<unknown>;

const previewHandlers: Record<string, PreviewHandler> = {
  preview_repository: ({ url }) => previewApi.previewRepository(String(url ?? 'preview://fusion-launcher')),
  preview_repository_file: ({ path }) => previewApi.previewRepositoryFile(String(path ?? '')),
  preview_builtin_demo_repository: () => previewApi.previewBuiltInDemoRepository(),
  connect_repository: ({ url }) => previewApi.connectRepository(String(url ?? 'preview://fusion-launcher')),
  connect_repository_file: ({ path }) => previewApi.connectRepositoryFile(String(path ?? '')),
  connect_builtin_demo_repository: () => previewApi.connectBuiltInDemoRepository(),
  repair_library: () => previewApi.repairLibrary(),
  refresh_repository: ({ repositoryId }) => previewApi.refreshRepository(String(repositoryId ?? '')),
  get_onboarding_state: () => previewApi.getOnboardingState(),
  list_repositories: () => previewApi.listRepositories(),
  disconnect_repository: ({ repositoryId }) => previewApi.disconnectRepository(String(repositoryId ?? '')),
  get_catalog: () => previewApi.getCatalog(),
  get_game: ({ gameId }) => previewApi.getGame(String(gameId ?? '')),
  scrape_game: ({ gameId }) => previewApi.scrapeGame(String(gameId ?? '')),
  get_scrape_state: ({ gameId }) => previewApi.getScrapeState(String(gameId ?? '')),
  list_scrape_candidates: ({ gameId }) => previewApi.listScrapeCandidates(String(gameId ?? '')),
  apply_scrape_override: ({ gameId, providerGameId }) => previewApi.applyScrapeOverride(
    String(gameId ?? ''),
    String(providerGameId ?? '')
  ),
  save_manual_metadata: ({ gameId, metadata }) => previewApi.saveManualMetadata(
    String(gameId ?? ''),
    metadata as ManualGameMetadataInput
  ),
  clear_scrape_override: ({ gameId }) => previewApi.clearScrapeOverride(String(gameId ?? '')),
  save_screenscraper_credentials: ({ ssid, sspassword, region }) => previewApi.saveScreenscraperCredentials(
    String(ssid ?? ''),
    String(sspassword ?? ''),
    typeof region === 'string' ? region : undefined
  ),
  get_screenscraper_status: () => previewApi.getScreenscraperStatus(),
  save_steamgriddb_key: ({ apiKey }) => previewApi.saveSteamgriddbKey(String(apiKey ?? '')),
  get_steamgriddb_status: () => previewApi.getSteamgriddbStatus(),
  scrape_library: () => previewApi.scrapeLibrary(),
  cancel_library_scrape: () => previewApi.cancelLibraryScrape(),
  check_requirements: ({ gameId }) => previewApi.checkRequirements(String(gameId ?? '')),
  get_library_statuses: () => previewApi.getLibraryStatuses(),
  list_platform_setup_profiles: () => previewApi.listPlatformSetupProfiles(),
  get_game_setup_state: ({ gameId }) => previewApi.getGameSetupState(String(gameId ?? '')),
  install_game: ({ gameId }) => previewApi.installGame(String(gameId ?? '')),
  install_emulator: ({ platform }) => previewApi.installEmulator(String(platform ?? '')),
  get_emulator_status: ({ platform }) => previewApi.getEmulatorStatus(String(platform ?? '')),
  get_emulator_install_status: ({ platform }) => previewApi.getEmulatorStatus(String(platform ?? '')),
  install_profile_emulator: ({ profileId }) => previewApi.installProfileEmulator(String(profileId ?? '')),
  select_profile_emulator: ({ profileId, executablePath }) => previewApi.selectProfileEmulator(
    String(profileId ?? ''),
    String(executablePath ?? '')
  ),
  import_profile_system_file: ({ gameId, requirementId, sourcePath }) => previewApi.importProfileSystemFile(
    String(gameId ?? ''),
    String(requirementId ?? ''),
    String(sourcePath ?? '')
  ),
  list_emulator_configs: () => previewApi.listEmulatorConfigs(),
  save_emulator_config: ({ platform, exePath, launchArgsTemplate }) => previewApi.saveEmulatorConfig(
    String(platform ?? ''),
    String(exePath ?? ''),
    typeof launchArgsTemplate === 'string' ? launchArgsTemplate : undefined
  ),
  validate_emulator_config: ({ platform }) => previewApi.validateEmulatorConfig(String(platform ?? '')),
  delete_emulator_config: ({ platform }) => previewApi.deleteEmulatorConfig(String(platform ?? '')),
  download_asset: ({ assetId }) => previewApi.downloadAsset(String(assetId ?? '')),
  import_asset_file: ({ assetId, sourcePath }) => previewApi.importAssetFile(
    String(assetId ?? ''),
    String(sourcePath ?? '')
  ),
  import_game_file: ({ gameId, sourcePath }) => previewApi.importGameFile(
    String(gameId ?? ''),
    String(sourcePath ?? '')
  ),
  download_game: ({ gameId }) => previewApi.downloadGame(String(gameId ?? '')),
  start_game_download: ({ gameId }) => previewApi.startGameDownload(String(gameId ?? '')),
  trust_executable: ({ assetId }) => previewApi.trustExecutable(String(assetId ?? '')),
  get_download_root: () => previewApi.getDownloadRoot(),
  set_download_root: ({ path }) => previewApi.setDownloadRoot(String(path ?? '')),
  remove_game: ({ gameId, deleteFiles }) => previewApi.removeGame(String(gameId ?? ''), Boolean(deleteFiles)),
  redownload_asset: ({ assetId }) => previewApi.redownloadAsset(String(assetId ?? '')),
  open_game_folder: ({ gameId }) => previewApi.openGameFolder(String(gameId ?? '')),
  open_emulator_folder: ({ platform }) => previewApi.openEmulatorFolder(String(platform ?? '')),
  open_logs_folder: () => previewApi.openLogsFolder(),
  run_health_check: () => previewApi.runHealthCheck(),
  get_diagnostics_paths: () => previewApi.getDiagnosticsPaths(),
  get_diagnostics_bundle: () => previewApi.getDiagnosticsBundle(),
  start_magnet_download: ({ gameId, magnetUri, saveDir }) => previewApi.startMagnetDownload(
    String(gameId ?? ''),
    String(magnetUri ?? ''),
    String(saveDir ?? '')
  ),
  get_torrent_status: ({ gameId }) => previewApi.getTorrentStatus(String(gameId ?? '')),
  get_game_download: ({ gameId }) => previewApi.getGameDownload(String(gameId ?? '')),
  list_torrent_downloads: () => previewApi.listTorrentDownloads(),
  pause_download: ({ gameId }) => previewApi.pauseDownload(String(gameId ?? '')),
  resume_download: ({ gameId }) => previewApi.resumeDownload(String(gameId ?? '')),
  cancel_download: ({ gameId }) => previewApi.cancelDownload(String(gameId ?? '')),
  launch_game: ({ gameId }) => previewApi.launchGame(String(gameId ?? '')),
  check_app_update: () => previewApi.checkAppUpdate(),
  install_app_update: () => previewApi.installAppUpdate()
};

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauriRuntime()) {
    if (isPreviewRuntime()) {
      const handler = previewHandlers[command];
      if (!handler) throw new Error(`No browser preview handler is available for ${command}.`);
      return handler(args ?? {}) as Promise<T>;
    }

    return requireDesktopBridge('Fusion Launcher API');
  }

  return invoke<T>(command, args);
}

export const api = {
  previewRepository(url: string) {
    return call<RepositoryPreview>('preview_repository', { url });
  },
  previewRepositoryFile(path: string) {
    return call<RepositoryPreview>('preview_repository_file', { path });
  },
  previewBuiltInDemoRepository() {
    return call<RepositoryPreview>('preview_builtin_demo_repository');
  },
  connectRepository(url: string) {
    return call<RepositorySummary>('connect_repository', { url });
  },
  connectRepositoryFile(path: string) {
    return call<RepositorySummary>('connect_repository_file', { path });
  },
  connectBuiltInDemoRepository() {
    return call<RepositorySummary>('connect_builtin_demo_repository');
  },
  repairLibrary() {
    return call<RepairLibraryReport>('repair_library');
  },
  refreshRepository(repositoryId: string) {
    return call<RepositorySummary>('refresh_repository', { repositoryId });
  },
  getOnboardingState() {
    return call<OnboardingState>('get_onboarding_state');
  },
  listRepositories() {
    return call<RepositorySummary[]>('list_repositories');
  },
  disconnectRepository(repositoryId: string) {
    return call<boolean>('disconnect_repository', { repositoryId });
  },
  getCatalog() {
    return call<CatalogGame[]>('get_catalog');
  },
  getGame(gameId: string) {
    return call<CatalogGame | null>('get_game', { gameId });
  },
  scrapeGame(gameId: string) {
    return call<void>('scrape_game', { gameId });
  },
  getScrapeState(gameId: string) {
    return call<ScrapeState>('get_scrape_state', { gameId });
  },
  listScrapeCandidates(gameId: string) {
    return call<ScrapeCandidate[]>('list_scrape_candidates', { gameId });
  },
  applyScrapeOverride(gameId: string, providerGameId: string) {
    return call<void>('apply_scrape_override', { gameId, providerGameId });
  },
  saveManualMetadata(gameId: string, metadata: ManualGameMetadataInput) {
    return call<void>('save_manual_metadata', { gameId, metadata });
  },
  clearScrapeOverride(gameId: string) {
    return call<boolean>('clear_scrape_override', { gameId });
  },
  saveScreenscraperCredentials(ssid: string, sspassword: string, region?: string) {
    return call<ScreenScraperStatus>('save_screenscraper_credentials', { ssid, sspassword, region });
  },
  getScreenscraperStatus() {
    return call<ScreenScraperStatus>('get_screenscraper_status');
  },
  saveSteamgriddbKey(apiKey: string) {
    return call<SteamGridDbStatus>('save_steamgriddb_key', { apiKey });
  },
  getSteamgriddbStatus() {
    return call<SteamGridDbStatus>('get_steamgriddb_status');
  },
  scrapeLibrary() {
    return call<LibraryScrapeStatus>('scrape_library');
  },
  cancelLibraryScrape() {
    return call<LibraryScrapeStatus>('cancel_library_scrape');
  },
  checkRequirements(gameId: string) {
    return call<RequirementsReport>('check_requirements', { gameId });
  },
  getLibraryStatuses() {
    return call<LibraryGameStatus[]>('get_library_statuses');
  },
  listPlatformSetupProfiles() {
    return call<PlatformSetupProfile[]>('list_platform_setup_profiles');
  },
  getGameSetupState(gameId: string) {
    return call<GameSetupState>('get_game_setup_state', { gameId });
  },
  installGame(gameId: string) {
    return call<InstallResult>('install_game', { gameId });
  },
  installEmulator(platform: string) {
    return call<EmulatorInstallResult>('install_emulator', { platform });
  },
  getEmulatorStatus(platform: string) {
    return call<EmulatorStatus>('get_emulator_status', { platform });
  },
  installProfileEmulator(profileId: string) {
    return call<ProfileEmulatorConfig>('install_profile_emulator', { profileId });
  },
  selectProfileEmulator(profileId: string, executablePath: string) {
    return call<ProfileEmulatorConfig>('select_profile_emulator', { profileId, executablePath });
  },
  importProfileSystemFile(gameId: string, requirementId: string, sourcePath: string) {
    return call<ImportAssetFileReport>('import_profile_system_file', { gameId, requirementId, sourcePath });
  },
  listEmulatorConfigs() {
    return call<EmulatorConfig[]>('list_emulator_configs');
  },
  saveEmulatorConfig(platform: string, exePath: string, launchArgsTemplate?: string) {
    return call<EmulatorConfig>('save_emulator_config', { platform, exePath, launchArgsTemplate });
  },
  validateEmulatorConfig(platform: string) {
    return call<EmulatorConfig>('validate_emulator_config', { platform });
  },
  deleteEmulatorConfig(platform: string) {
    return call<boolean>('delete_emulator_config', { platform });
  },
  downloadAsset(assetId: string) {
    return call<DownloadRecord>('download_asset', { assetId });
  },
  importAssetFile(assetId: string, sourcePath: string) {
    return call<ImportAssetFileReport>('import_asset_file', { assetId, sourcePath });
  },
  importGameFile(gameId: string, sourcePath: string) {
    return call<ImportGameFileReport>('import_game_file', { gameId, sourcePath });
  },
  downloadGame(gameId: string) {
    return call<DownloadRecord>('download_game', { gameId });
  },
  startGameDownload(gameId: string) {
    return call<GameDownloadStartReport>('start_game_download', { gameId });
  },
  trustExecutable(assetId: string) {
    return call<TrustedExecutable>('trust_executable', { assetId });
  },
  getDownloadRoot() {
    return call<string>('get_download_root');
  },
  setDownloadRoot(path: string) {
    return call<string>('set_download_root', { path });
  },
  removeGame(gameId: string, deleteFiles: boolean) {
    return call<boolean>('remove_game', { gameId, deleteFiles });
  },
  redownloadAsset(assetId: string) {
    return call<DownloadRecord>('redownload_asset', { assetId });
  },
  openGameFolder(gameId: string) {
    return call<void>('open_game_folder', { gameId });
  },
  openEmulatorFolder(platform: string) {
    return call<void>('open_emulator_folder', { platform });
  },
  openLogsFolder() {
    return call<void>('open_logs_folder');
  },
  runHealthCheck() {
    return call<HealthReport>('run_health_check');
  },
  getDiagnosticsPaths() {
    return call<DiagnosticsPaths>('get_diagnostics_paths');
  },
  getDiagnosticsBundle() {
    return call<DiagnosticsBundle>('get_diagnostics_bundle');
  },
  startMagnetDownload(gameId: string, magnetUri: string, saveDir: string) {
    return call<TorrentStartReport>('start_magnet_download', { gameId, magnetUri, saveDir });
  },
  getTorrentStatus(gameId: string) {
    return call<TorrentStatus>('get_torrent_status', { gameId });
  },
  getGameDownload(gameId: string) {
    return call<TorrentDownloadRecord | null>('get_game_download', { gameId });
  },
  listTorrentDownloads() {
    return call<TorrentDownloadRecord[]>('list_torrent_downloads');
  },
  pauseDownload(gameId: string) {
    return call<TorrentDownloadRecord>('pause_download', { gameId });
  },
  resumeDownload(gameId: string) {
    return call<TorrentDownloadRecord>('resume_download', { gameId });
  },
  cancelDownload(gameId: string) {
    return call<TorrentDownloadRecord>('cancel_download', { gameId });
  },
  launchGame(gameId: string) {
    return call<LaunchReport>('launch_game', { gameId });
  },
  checkAppUpdate() {
    return call<UpdateCheckReport>('check_app_update');
  },
  installAppUpdate() {
    return call<void>('install_app_update');
  }
};
