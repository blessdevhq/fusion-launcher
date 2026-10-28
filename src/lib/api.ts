import { invoke } from '@tauri-apps/api/core';
import { previewApi } from './previewData.ts';
import { fusionPreviewApi } from './fusionPreviewData.ts';
import type { FusionCatalogEntry, FusionSourcePreview, FusionSourceSummary, FusionGameState, FusionGameOptions, FusionInstallInput, FusionOperationProgress, FusionCrashReport, SourcePreview, SourceSummary } from '../types/fusionSource.ts';
import { isPreviewRuntime, isTauriRuntime, requireDesktopBridge } from './runtime.ts';
import type {
  EmulatorInstallResult,
  EmulatorStatus,
  InstallResult
} from '@/types/emulatorProfile';
import type { Manifest } from '@/types/manifest';
import type {
  CatalogGame,
  DeepLinkAction,
  DiagnosticsBundle,
  DiagnosticsPaths,
  CustomGameInput,
  GameFileInspection,
  DownloadRecord,
  EmulatorConfig,
  GameSetupState,
  GameDownloadStartReport,
  HealthReport,
  ImportAssetFileReport,
  ImportGameFileReport,
  SmartImportReport,
  LibraryScrapeStatus,
  LibraryGameStatus,
  ManualGameMetadataInput,
  LaunchReport,
  OnboardingState,
  PlatformSetupProfile,
  ProfileEmulatorConfig,
  ProfileEmulatorRemovalReport,
  RepairLibraryReport,
  RemovalReport,
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
type FusionGameIdentity = { sourceId: string; gameId: string };

export interface InstallGameOptions {
  gameTargetDir?: string;
  emulatorTargetDir?: string;
}

const previewHandlers: Record<string, PreviewHandler> = {
  fusion_source_preview_local: ({ directory }) => fusionPreviewApi.previewLocal(String(directory)),
  fusion_source_add_local: ({ directory, expectedSourceId, prepareEden }) => fusionPreviewApi.addLocal(String(directory), String(expectedSourceId), Boolean(prepareEden)),
  fusion_source_preview: ({ url }) => fusionPreviewApi.preview(String(url ?? '')),
  fusion_source_add: ({ url }) => fusionPreviewApi.add(String(url ?? '')),
  fusion_source_list: () => fusionPreviewApi.list(),
  fusion_source_refresh: ({ sourceId }) => fusionPreviewApi.refresh(String(sourceId)),
  fusion_source_remove: ({ sourceId }) => fusionPreviewApi.remove(String(sourceId)),
  fusion_source_list_catalog: () => fusionPreviewApi.catalog(),
  fusion_game_get_state: ({ game }) => fusionPreviewApi.state((game as FusionGameIdentity).sourceId, (game as FusionGameIdentity).gameId),
  fusion_game_set_options: ({ game, options }) => fusionPreviewApi.setOptions((game as FusionGameIdentity).sourceId, (game as FusionGameIdentity).gameId, options as FusionGameOptions),
  fusion_game_install: ({ game, request }) => fusionPreviewApi.install((game as FusionGameIdentity).sourceId, (game as FusionGameIdentity).gameId, request as FusionInstallInput),
  fusion_game_update: ({ game, request }) => fusionPreviewApi.install((game as FusionGameIdentity).sourceId, (game as FusionGameIdentity).gameId, request as FusionInstallInput),
  fusion_game_cancel: ({ game }) => fusionPreviewApi.cancel((game as FusionGameIdentity).sourceId, (game as FusionGameIdentity).gameId),
  fusion_game_rollback: ({ game }) => fusionPreviewApi.rollback((game as FusionGameIdentity).sourceId, (game as FusionGameIdentity).gameId),
  fusion_game_launch: ({ game }) => fusionPreviewApi.launch((game as FusionGameIdentity).sourceId, (game as FusionGameIdentity).gameId),
  fusion_game_remove: ({ game }) => fusionPreviewApi.removeGame((game as FusionGameIdentity).sourceId, (game as FusionGameIdentity).gameId),
  fusion_game_trust_executable: ({ game, path, blake3 }) => fusionPreviewApi.trust((game as FusionGameIdentity).sourceId, (game as FusionGameIdentity).gameId, String(path), String(blake3)),
  fusion_crash_preview: ({ reportId }) => fusionPreviewApi.crash(String(reportId)),
  fusion_crash_send: ({ reportId, expectedEndpoint }) => fusionPreviewApi.sendCrash(String(reportId), String(expectedEndpoint)),
  fusion_crash_discard: ({ reportId }) => fusionPreviewApi.discardCrash(String(reportId)),
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
  cache_artwork: ({ url }) => previewApi.cacheArtwork(String(url ?? '')),
  check_requirements: ({ gameId }) => previewApi.checkRequirements(String(gameId ?? '')),
  get_library_statuses: () => previewApi.getLibraryStatuses(),
  list_platform_setup_profiles: () => previewApi.listPlatformSetupProfiles(),
  get_game_setup_state: ({ gameId }) => previewApi.getGameSetupState(String(gameId ?? '')),
  install_game: ({ gameId, options }) => previewApi.installGame(
    String(gameId ?? ''),
    options as InstallGameOptions | undefined
  ),
  fetch_manifest: () =>
    Promise.reject(new Error('Manifest fetching is available in the desktop app.')),
  install_game_from_manifest: ({ titleId }) => previewApi.installGame(String(titleId ?? '')),
  add_manifest_source: () =>
    Promise.reject(new Error('Adding a manifest source is available in the desktop app.')),
  preview_source: ({ input }) => previewApi.previewSource(String(input ?? '')),
  add_source: ({ input }) => previewApi.addSource(String(input ?? '')),
  install_emulator: ({ platform }) => previewApi.installEmulator(String(platform ?? '')),
  get_emulator_status: ({ platform }) => previewApi.getEmulatorStatus(String(platform ?? '')),
  get_emulator_install_status: ({ platform }) => previewApi.getEmulatorStatus(String(platform ?? '')),
  install_profile_emulator: ({ profileId, targetDir }) => previewApi.installProfileEmulator(
    String(profileId ?? ''),
    typeof targetDir === 'string' ? targetDir : undefined
  ),
  remove_profile_emulator: ({ profileId }) => previewApi.removeProfileEmulator(String(profileId ?? '')),
  select_profile_emulator: ({ profileId, executablePath }) => previewApi.selectProfileEmulator(
    String(profileId ?? ''),
    String(executablePath ?? '')
  ),
  import_profile_system_file: ({ gameId, requirementId, sourcePath }) => previewApi.importProfileSystemFile(
    String(gameId ?? ''),
    String(requirementId ?? ''),
    String(sourcePath ?? '')
  ),
  detect_system_file: ({ sourcePath }) => previewApi.detectSystemFile(String(sourcePath ?? '')),
  auto_import_system_file: ({ sourcePath }) => previewApi.autoImportSystemFile(String(sourcePath ?? '')),
  parse_deep_link: ({ uri }) => previewApi.parseDeepLink(String(uri ?? '')),
  register_deep_link_protocol: () => previewApi.registerDeepLinkProtocol(),
  inspect_game_file: ({ path }) => Promise.resolve({
    path: String(path ?? ''),
    fileName: String(path ?? '').split(/[\\/]/).pop() ?? 'game',
    suggestedTitle: 'Imported Game',
    platform: 'ps2',
    setupProfileId: 'ps2-pcsx2',
    expectedExtensions: ['.iso'],
    sizeBytes: 1048576,
    sha256: '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
    crc32: '01234567'
  }),
  add_custom_game: () => Promise.reject(new Error('Adding custom games is available in desktop mode.')),
  export_game_package: ({ exportPath }) => Promise.resolve(String(exportPath ?? '')),
  list_emulator_configs: () => previewApi.listEmulatorConfigs(),
  save_emulator_config: ({ platform, exePath, launchArgsTemplate }) => previewApi.saveEmulatorConfig(
    String(platform ?? ''),
    String(exePath ?? ''),
    typeof launchArgsTemplate === 'string' ? launchArgsTemplate : undefined
  ),
  validate_emulator_config: ({ platform }) => previewApi.validateEmulatorConfig(String(platform ?? '')),
  delete_emulator_config: ({ platform }) => previewApi.deleteEmulatorConfig(String(platform ?? '')),
  download_asset: ({ assetId, targetDir }) => previewApi.downloadAsset(
    String(assetId ?? ''),
    typeof targetDir === 'string' ? targetDir : undefined
  ),
  import_asset_file: ({ assetId, sourcePath }) => previewApi.importAssetFile(
    String(assetId ?? ''),
    String(sourcePath ?? '')
  ),
  import_game_file: ({ gameId, sourcePath }) => previewApi.importGameFile(
    String(gameId ?? ''),
    String(sourcePath ?? '')
  ),
  download_game: ({ gameId, targetDir }) => previewApi.downloadGame(
    String(gameId ?? ''),
    typeof targetDir === 'string' ? targetDir : undefined
  ),
  start_game_download: ({ gameId, targetDir }) => previewApi.startGameDownload(
    String(gameId ?? ''),
    typeof targetDir === 'string' ? targetDir : undefined
  ),
  trust_executable: ({ assetId }) => previewApi.trustExecutable(String(assetId ?? '')),
  get_download_root: () => previewApi.getDownloadRoot(),
  set_download_root: ({ path }) => previewApi.setDownloadRoot(String(path ?? '')),
  get_library_root: () => previewApi.getLibraryRoot(),
  set_library_root: ({ path }) => previewApi.setLibraryRoot(String(path ?? '')),
  remove_game: ({ gameId, deleteFiles }) => previewApi.removeGame(String(gameId ?? ''), Boolean(deleteFiles)),
  remove_download: ({ downloadId, deleteFiles }) => previewApi.removeDownload(String(downloadId ?? ''), Boolean(deleteFiles)),
  redownload_asset: ({ assetId, targetDir }) => previewApi.redownloadAsset(
    String(assetId ?? ''),
    typeof targetDir === 'string' ? targetDir : undefined
  ),
  open_game_folder: ({ gameId }) => previewApi.openGameFolder(String(gameId ?? '')),
  open_download_folder: ({ downloadId }) => previewApi.openDownloadFolder(String(downloadId ?? '')),
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
  fusionSourcePreviewLocal(directory: string) {
    return call<FusionSourcePreview>('fusion_source_preview_local', { directory });
  },
  fusionSourceAddLocal(directory: string, expectedSourceId: string, prepareEden = false) {
    return call<FusionSourceSummary>('fusion_source_add_local', { directory, expectedSourceId, prepareEden });
  },
  fusionSourcePreview(input: string) {
    return call<FusionSourcePreview>('fusion_source_preview', { url: input });
  },
  fusionSourceAdd(input: string) {
    return call<FusionSourceSummary>('fusion_source_add', { url: input });
  },
  fusionSourceList() { return call<FusionSourceSummary[]>('fusion_source_list'); },
  fusionSourceRefresh(sourceId: string) { return call<FusionSourceSummary>('fusion_source_refresh', { sourceId }); },
  fusionSourceRemove(sourceId: string) { return call<void>('fusion_source_remove', { sourceId }); },
  fusionSourceListCatalog() { return call<FusionCatalogEntry[]>('fusion_source_list_catalog'); },
  fusionGameGetState(sourceId: string, gameId: string) { return call<FusionGameState>('fusion_game_get_state', { game: { sourceId, gameId } }); },
  fusionGameSetOptions(sourceId: string, gameId: string, options: FusionGameOptions) {
    return call<FusionGameState>('fusion_game_set_options', { game: { sourceId, gameId }, options });
  },
  fusionGameInstall(sourceId: string, gameId: string, input: FusionInstallInput) {
    return call<FusionOperationProgress>('fusion_game_install', { game: { sourceId, gameId }, request: input });
  },
  fusionGameUpdate(sourceId: string, gameId: string, input: FusionInstallInput) {
    return call<FusionOperationProgress>('fusion_game_update', { game: { sourceId, gameId }, request: input });
  },
  fusionGameCancel(sourceId: string, gameId: string) { return call<void>('fusion_game_cancel', { game: { sourceId, gameId } }); },
  fusionGameRollback(sourceId: string, gameId: string) { return call<FusionOperationProgress>('fusion_game_rollback', { game: { sourceId, gameId } }); },
  fusionGameLaunch(sourceId: string, gameId: string) { return call<FusionOperationProgress>('fusion_game_launch', { game: { sourceId, gameId } }); },
  fusionGameRemove(sourceId: string, gameId: string) { return call<FusionOperationProgress>('fusion_game_remove', { game: { sourceId, gameId } }); },
  fusionGameTrustExecutable(sourceId: string, gameId: string, path: string, blake3: string) {
    return call<void>('fusion_game_trust_executable', { game: { sourceId, gameId }, path, blake3 });
  },
  fusionCrashPreview(reportId: string) { return call<FusionCrashReport>('fusion_crash_preview', { reportId }); },
  fusionCrashSend(reportId: string, expectedEndpoint: string) { return call<void>('fusion_crash_send', { reportId, expectedEndpoint }); },
  fusionCrashDiscard(reportId: string) { return call<void>('fusion_crash_discard', { reportId }); },
  async previewAnySource(input: string): Promise<SourcePreview> {
    if (input.trim().startsWith('{')) {
      const json: unknown = JSON.parse(input);
      if (typeof json === 'object' && json !== null && 'source_info' in json) {
        throw new Error('Host source-index.json at an HTTPS URL, then paste that URL here.');
      }
      return { kind: 'legacy', source: await api.previewSource(input) };
    }
    try {
      return { kind: 'fusion_source', source: await api.fusionSourcePreview(input) };
    } catch (error) {
      if (typeof error !== 'object' || error === null || !('code' in error) || error.code !== 'unsupported_schema') throw error;
      return { kind: 'legacy', source: await api.previewSource(input) };
    }
  },
  async addAnySource(input: string, preview?: SourcePreview): Promise<SourceSummary> {
    const checked = preview ?? await api.previewAnySource(input);
    return checked.kind === 'fusion_source'
      ? { kind: 'fusion_source', source: await api.fusionSourceAdd(input) }
      : { kind: 'legacy', source: await api.addSource(input) };
  },
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
  cacheArtwork(url: string) {
    return call<string>('cache_artwork', { url });
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
  installGame(gameId: string, options?: InstallGameOptions) {
    return call<InstallResult>('install_game', { gameId, options });
  },
  fetchManifest(url: string) {
    return call<Manifest>('fetch_manifest', { url });
  },
  installGameFromManifest(url: string, titleId: string) {
    return call<InstallResult>('install_game_from_manifest', { url, titleId });
  },
  addManifestSource(url: string) {
    return call<RepositorySummary>('add_manifest_source', { url });
  },
  previewSource(input: string) {
    return call<RepositoryPreview>('preview_source', { input });
  },
  addSource(input: string) {
    return call<RepositorySummary>('add_source', { input });
  },
  installEmulator(platform: string) {
    return call<EmulatorInstallResult>('install_emulator', { platform });
  },
  getEmulatorStatus(platform: string) {
    return call<EmulatorStatus>('get_emulator_status', { platform });
  },
  installProfileEmulator(profileId: string, targetDir?: string) {
    return call<ProfileEmulatorConfig>('install_profile_emulator', { profileId, targetDir });
  },
  removeProfileEmulator(profileId: string) {
    return call<ProfileEmulatorRemovalReport>('remove_profile_emulator', { profileId });
  },
  selectProfileEmulator(profileId: string, executablePath: string) {
    return call<ProfileEmulatorConfig>('select_profile_emulator', { profileId, executablePath });
  },
  importProfileSystemFile(gameId: string, requirementId: string, sourcePath: string) {
    return call<ImportAssetFileReport>('import_profile_system_file', { gameId, requirementId, sourcePath });
  },
  detectSystemFile(sourcePath: string) {
    return call<SmartImportReport>('detect_system_file', { sourcePath });
  },
  autoImportSystemFile(sourcePath: string) {
    return call<SmartImportReport>('auto_import_system_file', { sourcePath });
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
  downloadAsset(assetId: string, targetDir?: string) {
    return call<DownloadRecord>('download_asset', { assetId, targetDir });
  },
  importAssetFile(assetId: string, sourcePath: string) {
    return call<ImportAssetFileReport>('import_asset_file', { assetId, sourcePath });
  },
  importGameFile(gameId: string, sourcePath: string) {
    return call<ImportGameFileReport>('import_game_file', { gameId, sourcePath });
  },
  downloadGame(gameId: string, targetDir?: string) {
    return call<DownloadRecord>('download_game', { gameId, targetDir });
  },
  startGameDownload(gameId: string, targetDir?: string) {
    return call<GameDownloadStartReport>('start_game_download', { gameId, targetDir });
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
  getLibraryRoot() {
    return call<string>('get_library_root');
  },
  setLibraryRoot(path: string) {
    return call<string>('set_library_root', { path });
  },
  removeGame(gameId: string, deleteFiles: boolean) {
    return call<RemovalReport>('remove_game', { gameId, deleteFiles });
  },
  removeDownload(downloadId: string, deleteFiles: boolean) {
    return call<RemovalReport>('remove_download', { downloadId, deleteFiles });
  },
  redownloadAsset(assetId: string, targetDir?: string) {
    return call<DownloadRecord>('redownload_asset', { assetId, targetDir });
  },
  openGameFolder(gameId: string) {
    return call<void>('open_game_folder', { gameId });
  },
  openDownloadFolder(downloadId: string) {
    return call<void>('open_download_folder', { downloadId });
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
  },
  parseDeepLink(uri: string) {
    return call<DeepLinkAction>('parse_deep_link', { uri });
  },
  registerDeepLinkProtocol() {
    return call<void>('register_deep_link_protocol');
  },
  inspectGameFile(path: string, computeHash = true) {
    return call<GameFileInspection>('inspect_game_file', { path, computeHash });
  },
  addCustomGame(input: CustomGameInput) {
    return call<CatalogGame>('add_custom_game', { input });
  },
  exportGamePackage(input: CustomGameInput, exportPath: string) {
    return call<string>('export_game_package', { input, exportPath });
  }
};
