import { getEmulatorConfig, getEmulatorPath, type AppSettings } from './settings.ts';
import { getUiText } from './i18n.ts';
import { PLATFORM_LABELS } from '../types/platform.ts';
import type {
  CatalogGame,
  LibraryGameStatus,
  TorrentDownloadRecord,
  TorrentDownloadStatus
} from '../types/repository.ts';

export type LibraryFilter = 'all' | 'installed' | 'downloading' | 'missing';
export type LibrarySort = 'title' | 'platform' | 'status' | 'repository';
export type PrimaryGameAction = 'play' | 'download' | 'import' | 'resume' | 'retry' | 'details';
export type StatusTone = 'ready' | 'active' | 'paused' | 'error' | 'missing' | 'idle';

const ACTIVE_DOWNLOAD_STATUSES: TorrentDownloadStatus[] = ['queued', 'resolving', 'downloading', 'cancelling'];
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
    const text = getUiText(settings.language).gameDetails.setup;
    missingRequirements.push(
      emulatorPath
        ? text.chooseEmulatorAgain(PLATFORM_LABELS[game.platform])
        : text.configureEmulator(PLATFORM_LABELS[game.platform])
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
    game,
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

export function searchAndSortLibraryItems(
  items: GameLibraryItem[],
  filter: LibraryFilter,
  query: string,
  sort: LibrarySort
): GameLibraryItem[] {
  const normalizedQuery = normalizeSearchText(query);
  const filtered = filterLibraryItems(items, filter);
  const searched = normalizedQuery
    ? filtered.filter((item) => matchesLibraryQuery(item, normalizedQuery))
    : filtered;

  return [...searched].sort((left, right) => compareLibraryItems(left, right, sort));
}

function matchesLibraryQuery(item: GameLibraryItem, query: string) {
  const terms = query.split(/\s+/).filter(Boolean);
  if (terms.length === 0) return true;

  const searchableFields = [
    item.game.title,
    item.game.platform,
    item.game.repositoryName,
    item.game.sourceId,
    item.game.metadata?.developer ?? '',
    item.game.metadata?.publisher ?? '',
    item.game.metadata?.series ?? '',
    item.game.metadata?.releaseYear ? String(item.game.metadata.releaseYear) : '',
    ...(item.game.metadata?.genres ?? []),
    ...(item.game.metadata?.tags ?? []),
    item.statusLabel,
    ...item.missingRequirements
  ];

  const combined = normalizeSearchText(searchableFields.join(' '));
  return terms.every((term) => combined.includes(term));
}

function compareLibraryItems(left: GameLibraryItem, right: GameLibraryItem, sort: LibrarySort) {
  if (sort === 'platform') {
    return compareStrings(left.game.platform, right.game.platform)
      || compareStrings(left.game.title, right.game.title);
  }
  if (sort === 'status') {
    return statusRank(left) - statusRank(right)
      || compareStrings(left.game.title, right.game.title);
  }
  if (sort === 'repository') {
    return compareStrings(left.game.repositoryName, right.game.repositoryName)
      || compareStrings(left.game.title, right.game.title);
  }
  return compareStrings(left.game.title, right.game.title);
}

function statusRank(item: GameLibraryItem) {
  if (item.readyToPlay) return 0;
  if (item.isDownloading) return 1;
  if (item.isPaused) return 2;
  if (item.hasError) return 3;
  if (item.installed) return 4;
  return 5;
}

function compareStrings(left: string, right: string) {
  return left.localeCompare(right, undefined, { sensitivity: 'base', numeric: true });
}

function normalizeSearchText(value: string) {
  return value.trim().toLocaleLowerCase();
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
  if (installed && missingRequirements.some(isGameFileRequirement)) {
    return { statusLabel: 'Game File Issue', statusTone: 'error' };
  }
  if (installed && missingRequirements.length > 0) return { statusLabel: 'Missing Requirements', statusTone: 'missing' };
  if (hasError) return { statusLabel: 'Download Error', statusTone: 'error' };
  if (downloadStatus === 'interrupted') return { statusLabel: 'Interrupted', statusTone: 'paused' };
  if (isPaused) return { statusLabel: 'Paused', statusTone: 'paused' };
  if (downloadStatus === 'queued') return { statusLabel: 'Queued', statusTone: 'active' };
  if (downloadStatus === 'resolving') return { statusLabel: 'Resolving Magnet', statusTone: 'active' };
  if (downloadStatus === 'cancelling') return { statusLabel: 'Cancelling', statusTone: 'paused' };
  if (isDownloading) return { statusLabel: 'Downloading', statusTone: 'active' };
  if (installed) return { statusLabel: 'Installed', statusTone: 'ready' };
  if (downloadStatus === 'cancelled') return { statusLabel: 'Cancelled', statusTone: 'idle' };
  return { statusLabel: 'Not Installed', statusTone: 'idle' };
}

function derivePrimaryAction({
  game,
  downloadStatus,
  readyToPlay,
  isPaused,
  hasError,
  installed,
  missingRequirements
}: {
  game: CatalogGame;
  downloadStatus: TorrentDownloadStatus | null;
  readyToPlay: boolean;
  isPaused: boolean;
  hasError: boolean;
  installed: boolean;
  missingRequirements: string[];
}): { primaryAction: PrimaryGameAction; primaryActionLabel: string } {
  const userProvided = isUserProvidedGame(game);
  if (readyToPlay) return { primaryAction: 'play', primaryActionLabel: 'Play' };
  if (hasError) return { primaryAction: 'retry', primaryActionLabel: 'Retry' };
  if (isPaused) return { primaryAction: 'resume', primaryActionLabel: 'Resume' };
  if (
    downloadStatus === 'queued' ||
    downloadStatus === 'resolving' ||
    downloadStatus === 'downloading' ||
    downloadStatus === 'cancelling'
  ) {
    return { primaryAction: 'details', primaryActionLabel: 'Details' };
  }
  if (installed && missingRequirements.some(isGameFileRequirement)) {
    if (userProvided) return { primaryAction: 'import', primaryActionLabel: 'Import' };
    return { primaryAction: 'download', primaryActionLabel: 'Re-download' };
  }
  if (installed && missingRequirements.length > 0) {
    return { primaryAction: 'details', primaryActionLabel: 'Fix Requirements' };
  }
  if (userProvided) return { primaryAction: 'import', primaryActionLabel: 'Import' };
  if (game.contentMode === 'metadata_only') return { primaryAction: 'details', primaryActionLabel: 'Details' };
  return { primaryAction: 'download', primaryActionLabel: 'Install' };
}

function isGameFileRequirement(message: string) {
  return message.toLowerCase().startsWith('game file:');
}

function isUserProvidedGame(game: CatalogGame) {
  return game.contentMode === 'user_provided' || game.downloads.some((source) => source.kind === 'user_provided');
}

function clampPercent(value: number) {
  if (!Number.isFinite(value)) return 0;
  return Math.min(Math.max(value, 0), 100);
}
