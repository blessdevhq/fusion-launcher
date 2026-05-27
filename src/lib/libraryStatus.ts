import { getEmulatorConfig, getEmulatorPath, type AppSettings } from './settings.ts';
import { PLATFORM_LABELS } from '../types/platform.ts';
import type {
  CatalogGame,
  LibraryGameStatus,
  TorrentDownloadRecord,
  TorrentDownloadStatus
} from '../types/repository.ts';

export type LibraryFilter = 'all' | 'installed' | 'downloading' | 'missing';
export type PrimaryGameAction = 'play' | 'download' | 'resume' | 'retry' | 'details';
export type StatusTone = 'ready' | 'active' | 'paused' | 'error' | 'missing' | 'idle';

const ACTIVE_DOWNLOAD_STATUSES: TorrentDownloadStatus[] = ['resolving', 'downloading', 'cancelling'];
const RESUMABLE_DOWNLOAD_STATUSES: TorrentDownloadStatus[] = ['paused', 'interrupted'];

export interface GameLibraryItem {
  game: CatalogGame;
  backendStatus: LibraryGameStatus | null;
  download: TorrentDownloadRecord | null;
  installed: boolean;
  emulatorConfigured: boolean;
  systemRequirementsReady: boolean;
  missingRequirements: string[];
  readyToPlay: boolean;
  isDownloading: boolean;
  isPaused: boolean;
  hasError: boolean;
  progressPercent: number;
  statusLabel: string;
  statusTone: StatusTone;
  primaryAction: PrimaryGameAction;
  primaryActionLabel: string;
}

export function buildGameLibraryItems(
  catalog: CatalogGame[],
  statuses: LibraryGameStatus[],
  settings: AppSettings
): GameLibraryItem[] {
  const statusesByGameId = new Map(statuses.map((status) => [status.gameId, status]));

  return catalog.map((game) => buildGameLibraryItem(game, statusesByGameId.get(game.id) ?? null, settings));
}

export function buildGameLibraryItem(
  game: CatalogGame,
  backendStatus: LibraryGameStatus | null,
  settings: AppSettings
): GameLibraryItem {
  const download = backendStatus?.download ?? null;
  const downloadStatus = download?.status ?? null;
  const installed = Boolean(backendStatus?.installed || downloadStatus === 'completed');
  const emulatorPath = getEmulatorPath(settings, game.platform);
  const emulatorConfig = getEmulatorConfig(settings, game.platform);
  const emulatorConfigured = Boolean(emulatorPath) && (emulatorConfig?.status ?? 'valid') === 'valid';
  const missingRequirements = installed ? [...(backendStatus?.missingRequirements ?? [])] : [];

  if (installed && !emulatorConfigured) {
    missingRequirements.push(
      emulatorPath
        ? `Re-select ${PLATFORM_LABELS[game.platform]} emulator`
        : `Configure ${PLATFORM_LABELS[game.platform]} emulator`
    );
  }

  const systemRequirementsReady = installed
    ? Boolean(backendStatus?.systemRequirementsReady ?? true) && emulatorConfigured
    : Boolean(backendStatus?.systemRequirementsReady ?? true);
  const readyToPlay = installed && systemRequirementsReady && missingRequirements.length === 0;
  const isDownloading = downloadStatus !== null && ACTIVE_DOWNLOAD_STATUSES.includes(downloadStatus);
  const isPaused = downloadStatus !== null && RESUMABLE_DOWNLOAD_STATUSES.includes(downloadStatus);
  const hasError = downloadStatus === 'error';
  const progressPercent = clampPercent(download?.progressPercent ?? (installed ? 100 : 0));
  const { statusLabel, statusTone } = deriveStatusLabel({
    downloadStatus,
    installed,
    readyToPlay,
    isDownloading,
    isPaused,
    hasError,
    missingRequirements
  });
  const { primaryAction, primaryActionLabel } = derivePrimaryAction({
    downloadStatus,
    readyToPlay,
    isPaused,
    hasError,
    installed,
    missingRequirements
  });

  return {
    game,
    backendStatus,
    download,
    installed,
    emulatorConfigured,
    systemRequirementsReady,
    missingRequirements,
    readyToPlay,
    isDownloading,
    isPaused,
    hasError,
    progressPercent,
    statusLabel,
    statusTone,
    primaryAction,
    primaryActionLabel
  };
}

export function filterLibraryItems(items: GameLibraryItem[], filter: LibraryFilter): GameLibraryItem[] {
  switch (filter) {
    case 'installed':
      return items.filter((item) => item.installed);
    case 'downloading':
      return items.filter((item) => item.isDownloading || item.isPaused || item.hasError);
    case 'missing':
      return items.filter((item) => item.installed && item.missingRequirements.length > 0);
    default:
      return items;
  }
}

function deriveStatusLabel({
  downloadStatus,
  installed,
  readyToPlay,
  isDownloading,
  isPaused,
  hasError,
  missingRequirements
}: {
  downloadStatus: TorrentDownloadStatus | null;
  installed: boolean;
  readyToPlay: boolean;
  isDownloading: boolean;
  isPaused: boolean;
  hasError: boolean;
  missingRequirements: string[];
}): { statusLabel: string; statusTone: StatusTone } {
  if (readyToPlay) return { statusLabel: 'Ready to Play', statusTone: 'ready' };
  if (installed && missingRequirements.length > 0) return { statusLabel: 'Missing Requirements', statusTone: 'missing' };
  if (hasError) return { statusLabel: 'Download Error', statusTone: 'error' };
  if (downloadStatus === 'interrupted') return { statusLabel: 'Interrupted', statusTone: 'paused' };
  if (isPaused) return { statusLabel: 'Paused', statusTone: 'paused' };
  if (downloadStatus === 'resolving') return { statusLabel: 'Resolving Magnet', statusTone: 'active' };
  if (downloadStatus === 'cancelling') return { statusLabel: 'Cancelling', statusTone: 'paused' };
  if (isDownloading) return { statusLabel: 'Downloading', statusTone: 'active' };
  if (installed) return { statusLabel: 'Installed', statusTone: 'ready' };
  if (downloadStatus === 'cancelled') return { statusLabel: 'Cancelled', statusTone: 'idle' };
  return { statusLabel: 'Not Installed', statusTone: 'idle' };
}

function derivePrimaryAction({
  downloadStatus,
  readyToPlay,
  isPaused,
  hasError,
  installed,
  missingRequirements
}: {
  downloadStatus: TorrentDownloadStatus | null;
  readyToPlay: boolean;
  isPaused: boolean;
  hasError: boolean;
  installed: boolean;
  missingRequirements: string[];
}): { primaryAction: PrimaryGameAction; primaryActionLabel: string } {
  if (readyToPlay) return { primaryAction: 'play', primaryActionLabel: 'Play' };
  if (hasError) return { primaryAction: 'retry', primaryActionLabel: 'Retry' };
  if (isPaused) return { primaryAction: 'resume', primaryActionLabel: 'Resume' };
  if (downloadStatus === 'resolving' || downloadStatus === 'downloading' || downloadStatus === 'cancelling') {
    return { primaryAction: 'details', primaryActionLabel: 'Details' };
  }
  if (installed && missingRequirements.length > 0) {
    return { primaryAction: 'details', primaryActionLabel: 'Fix Requirements' };
  }
  return { primaryAction: 'download', primaryActionLabel: 'Download' };
}

function clampPercent(value: number) {
  if (!Number.isFinite(value)) return 0;
  return Math.min(Math.max(value, 0), 100);
}
