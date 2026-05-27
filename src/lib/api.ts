import { invoke } from '@tauri-apps/api/core';
import { previewApi } from './previewData.ts';
import { isPreviewRuntime, isTauriRuntime, requireDesktopBridge } from './runtime.ts';
import type {
  CatalogGame,
  DiagnosticsBundle,
  DownloadRecord,
  EmulatorConfig,
  GameDownloadStartReport,
  HealthReport,
  LibraryGameStatus,
  LaunchReport,
  OnboardingState,
  RepositoryPreview,
  RepositorySummary,
  RequirementsReport,
  TorrentDownloadRecord,
  TorrentStartReport,
  TorrentStatus,
  TrustedExecutable
} from '@/types/repository';

type PreviewHandler = (args: Record<string, unknown>) => Promise<unknown>;

const previewHandlers: Record<string, PreviewHandler> = {
  preview_repository: ({ url }) => previewApi.previewRepository(String(url ?? 'preview://retrohydra')),
  connect_repository: ({ url }) => previewApi.connectRepository(String(url ?? 'preview://retrohydra')),
  refresh_repository: ({ repositoryId }) => previewApi.refreshRepository(String(repositoryId ?? '')),
  get_onboarding_state: () => previewApi.getOnboardingState(),
  list_repositories: () => previewApi.listRepositories(),
  disconnect_repository: ({ repositoryId }) => previewApi.disconnectRepository(String(repositoryId ?? '')),
  get_catalog: () => previewApi.getCatalog(),
  get_game: ({ gameId }) => previewApi.getGame(String(gameId ?? '')),
  check_requirements: ({ gameId }) => previewApi.checkRequirements(String(gameId ?? '')),
  get_library_statuses: () => previewApi.getLibraryStatuses(),
  list_emulator_configs: () => previewApi.listEmulatorConfigs(),
  save_emulator_config: ({ platform, exePath, launchArgsTemplate }) => previewApi.saveEmulatorConfig(
    String(platform ?? ''),
    String(exePath ?? ''),
    typeof launchArgsTemplate === 'string' ? launchArgsTemplate : undefined
  ),
  validate_emulator_config: ({ platform }) => previewApi.validateEmulatorConfig(String(platform ?? '')),
  delete_emulator_config: ({ platform }) => previewApi.deleteEmulatorConfig(String(platform ?? '')),
  download_asset: ({ assetId }) => previewApi.downloadAsset(String(assetId ?? '')),
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
  launch_game: ({ gameId }) => previewApi.launchGame(String(gameId ?? ''))
};

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauriRuntime()) {
    if (isPreviewRuntime()) {
      const handler = previewHandlers[command];
      if (!handler) throw new Error(`No browser preview handler is available for ${command}.`);
      return handler(args ?? {}) as Promise<T>;
    }

    return requireDesktopBridge('RetroHydra API');
  }

  return invoke<T>(command, args);
}

export const api = {
  previewRepository(url: string) {
    return call<RepositoryPreview>('preview_repository', { url });
  },
  connectRepository(url: string) {
    return call<RepositorySummary>('connect_repository', { url });
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
  checkRequirements(gameId: string) {
    return call<RequirementsReport>('check_requirements', { gameId });
  },
  getLibraryStatuses() {
    return call<LibraryGameStatus[]>('get_library_statuses');
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
  }
};
