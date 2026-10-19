import { create } from 'zustand';
import { DEFAULT_SETTINGS, type AppSettings } from '../lib/settings.ts';
import { combineCatalog, type CatalogEntry, type FusionCatalogEntry } from '../types/fusionSource.ts';
import type {
  CatalogGame,
  DownloadProgressEvent,
  DownloadRemovedEvent,
  LibraryGameStatus,
  RepositorySummary,
  TorrentDownloadRecord
} from '../types/repository.ts';

export type LauncherView = 'home' | 'library' | 'downloads';

export interface ActivityEvent {
  id: string;
  title: string;
  detail: string;
  timestamp: string;
  gameId?: string;
  tone: 'info' | 'success' | 'warning' | 'error';
}

interface LauncherState {
  catalog: CatalogGame[];
  fusionCatalog: FusionCatalogEntry[];
  catalogEntries: CatalogEntry[];
  repositories: RepositorySummary[];
  libraryStatuses: LibraryGameStatus[];
  downloads: TorrentDownloadRecord[];
  settings: AppSettings;
  activeView: LauncherView;
  focusedItemId: string | null;
  selectedGameId: string | null;
  activityEvents: ActivityEvent[];
  removedDownloads: DownloadRemovedEvent[];
  setCatalog: (catalog: CatalogGame[]) => void;
  setFusionCatalog: (catalog: FusionCatalogEntry[]) => void;
  setRepositories: (repositories: RepositorySummary[]) => void;
  setLibraryStatuses: (statuses: LibraryGameStatus[]) => void;
  setDownloads: (downloads: TorrentDownloadRecord[]) => void;
  setSettings: (settings: AppSettings) => void;
  setActiveView: (view: LauncherView) => void;
  setFocusedItemId: (focusedItemId: string | null) => void;
  setSelectedGameId: (selectedGameId: string | null) => void;
  mergeDownloadEvent: (event: DownloadProgressEvent) => void;
  removeDownload: (event: DownloadRemovedEvent) => void;
  addActivityEvent: (event: Omit<ActivityEvent, 'id' | 'timestamp'> & { timestamp?: string }) => void;
}

export const useLauncherStore = create<LauncherState>((set) => ({
  catalog: [],
  fusionCatalog: [],
  catalogEntries: [],
  repositories: [],
  libraryStatuses: [],
  downloads: [],
  settings: DEFAULT_SETTINGS,
  activeView: 'home',
  focusedItemId: null,
  selectedGameId: null,
  activityEvents: [],
  removedDownloads: [],
  setCatalog: (catalog) => set((state) => ({ catalog, catalogEntries: combineCatalog(catalog, state.fusionCatalog) })),
  setFusionCatalog: (fusionCatalog) => set((state) => ({ fusionCatalog, catalogEntries: combineCatalog(state.catalog, fusionCatalog) })),
  setRepositories: (repositories) => set({ repositories }),
  setLibraryStatuses: (libraryStatuses) => set((state) => ({
    libraryStatuses: libraryStatuses.map((status) => {
      const existing = state.libraryStatuses.find((item) => item.gameId === status.gameId);
      const current = existing?.download ?? state.downloads.find((item) => item.gameId === status.gameId);
      if (status.download && (isRemoved(status.download, state.removedDownloads) || isStale(current, status.download))) {
        return existing ?? { ...status, installed: false, download: null };
      }
      return status.download && current
        ? { ...status, download: mergeDownloadSnapshotRecord(current, status.download) }
        : status;
    })
  })),
  setDownloads: (downloads) => set((state) => {
    const merged = mergeDownloadSnapshotIntoList(state.downloads, downloads, state.removedDownloads);
    return {
      downloads: merged,
      removedDownloads: retireChangedAttempts(state.removedDownloads, state.downloads, merged)
    };
  }),
  setSettings: (settings) => set({ settings }),
  setActiveView: (activeView) => set({ activeView }),
  setFocusedItemId: (focusedItemId) => set({ focusedItemId }),
  setSelectedGameId: (selectedGameId) => set({ selectedGameId }),
  mergeDownloadEvent: (event) => set((state) => {
    const previous = state.downloads.find((download) => download.gameId === event.gameId);
    const downloads = mergeDownloadEventIntoList(state.downloads, event, state.removedDownloads);
    if (downloads === state.downloads) return state;
    const libraryStatuses = state.libraryStatuses.map((status) => {
      if (status.gameId !== event.gameId) return status;

      return {
        ...status,
        installed: status.installed || event.finished || event.status === 'completed',
        download: mergeDownloadRecord(status.download, event)
      };
    });

    return {
      downloads,
      removedDownloads: retireChangedAttempts(state.removedDownloads, state.downloads, downloads),
      libraryStatuses,
      activityEvents: event.finished && !(previous?.status === 'completed'
        && previous.attemptId === event.attemptId && previous.runId === event.runId)
        ? prependActivity(state.activityEvents, {
            title: 'Download completed',
            detail: event.gameId,
            gameId: event.gameId,
            tone: 'success'
          })
        : state.activityEvents
    };
  }),
  removeDownload: (event) => set((state) => {
    const current = state.downloads.find((item) => item.gameId === event.gameId)
      ?? state.libraryStatuses.find((item) => item.gameId === event.gameId)?.download;
    const removed = { gameId: event.gameId, attemptId: event.attemptId ?? current?.attemptId };
    const matches = (record: TorrentDownloadRecord | null | undefined) => (
      !removed.attemptId || !record?.attemptId || record.attemptId === removed.attemptId
    );
    return {
      removedDownloads: addTombstone(state.removedDownloads, removed),
      downloads: state.downloads.filter((item) => item.gameId !== event.gameId || !matches(item)),
      libraryStatuses: state.libraryStatuses.map((status) => (
        status.gameId === event.gameId && matches(status.download)
          ? { ...status, installed: false, download: null }
          : status
      ))
    };
  }),
  addActivityEvent: (event) => set((state) => ({
    activityEvents: prependActivity(state.activityEvents, event)
  }))
}));

export function mergeDownloadEventIntoList(
  downloads: TorrentDownloadRecord[],
  event: DownloadProgressEvent,
  removed: DownloadRemovedEvent[] = []
): TorrentDownloadRecord[] {
  const existingIndex = downloads.findIndex((download) => download.gameId === event.gameId);
  const existing = existingIndex >= 0 ? downloads[existingIndex] : null;
  if (isRemoved(event, removed) || isStale(existing, event)) return downloads;
  if (existing?.attemptId && !event.attemptId) return downloads;
  if (existing?.runId != null && event.runId == null
    && (!event.attemptId || event.attemptId === existing.attemptId)) return downloads;
  if (existing?.runId != null && existing.attemptId === event.attemptId
    && existing.runId === event.runId
    && ['paused', 'cancelling', 'cancelled', 'completed'].includes(existing.status)
    && existing.status !== event.status) return downloads;
  const merged = mergeDownloadRecord(existing ?? null, event);
  if (existingIndex < 0) return [merged, ...downloads];
  return downloads.map((download, index) => index === existingIndex ? merged : download);
}

export function mergeDownloadSnapshotIntoList(
  current: TorrentDownloadRecord[],
  incoming: TorrentDownloadRecord[],
  removed: DownloadRemovedEvent[] = []
): TorrentDownloadRecord[] {
  if (current.length === 0) return incoming.filter((download) => !isRemoved(download, removed));

  const currentIds = new Set(current.map((download) => download.gameId));
  const incomingById = new Map(incoming.map((download) => [download.gameId, download]));
  const updatedCurrent = current.flatMap((download) => {
    const next = incomingById.get(download.gameId);
    if (!next) return [];
    if (isRemoved(next, removed) || isStale(download, next)) return [download];
    return [mergeDownloadSnapshotRecord(download, next)];
  });
  const newDownloads = incoming.filter((download) => !currentIds.has(download.gameId) && !isRemoved(download, removed));

  return [...newDownloads, ...updatedCurrent];
}

export function mergeDownloadRecord(
  record: TorrentDownloadRecord | null,
  event: DownloadProgressEvent
): TorrentDownloadRecord {
  const now = new Date().toISOString();
  const sameAttempt = !record?.attemptId || !event.attemptId || record.attemptId === event.attemptId;
  const previous = sameAttempt ? record : null;
  const progress = stableProgressValues(previous, {
    progressPercent: event.progressPercent,
    downloadedBytes: event.downloadedBytes,
    totalBytes: event.totalBytes
  });

  return {
    gameId: event.gameId,
    attemptId: event.attemptId ?? record?.attemptId,
    runId: event.runId ?? (sameAttempt ? record?.runId : undefined),
    subjectType: event.subjectType ?? record?.subjectType ?? null,
    displayName: event.displayName ?? record?.displayName ?? null,
    parentGameId: event.parentGameId ?? record?.parentGameId ?? null,
    magnetUri: previous?.magnetUri ?? '',
    saveDir: event.saveDir || record?.saveDir || '',
    status: event.status,
    progressPercent: progress.progressPercent,
    downloadedBytes: progress.downloadedBytes,
    totalBytes: progress.totalBytes,
    downloadSpeedBytesPerSec: event.downloadSpeedBytesPerSec,
    uploadSpeedBytesPerSec: event.uploadSpeedBytesPerSec,
    peersCount: event.peersCount,
    torrentId: previous?.torrentId ?? null,
    errorMessage: event.error ?? null,
    createdAt: previous?.createdAt ?? now,
    updatedAt: now,
    completedAt: event.finished ? now : previous?.completedAt ?? null
  };
}

function mergeDownloadSnapshotRecord(
  current: TorrentDownloadRecord,
  incoming: TorrentDownloadRecord
): TorrentDownloadRecord {
  const sameAttempt = !current.attemptId || !incoming.attemptId || current.attemptId === incoming.attemptId;
  const progress = stableProgressValues(sameAttempt ? current : null, incoming);

  return {
    ...incoming,
    attemptId: incoming.attemptId ?? current.attemptId,
    runId: incoming.runId ?? (sameAttempt ? current.runId : undefined),
    progressPercent: progress.progressPercent,
    downloadedBytes: progress.downloadedBytes,
    totalBytes: progress.totalBytes
  };
}

function isRemoved(
  event: Pick<DownloadProgressEvent, 'gameId' | 'attemptId'>,
  removed: DownloadRemovedEvent[]
): boolean {
  return removed.some((item) => item.gameId === event.gameId
    && (!event.attemptId || item.attemptId === event.attemptId));
}

function isStale(
  current: Pick<TorrentDownloadRecord, 'attemptId' | 'runId'> | null | undefined,
  incoming: Pick<TorrentDownloadRecord, 'attemptId' | 'runId'>
): boolean {
  if (!current) return false;
  if (current.attemptId && !incoming.attemptId) {
    // Identity-free snapshots remain supported; identity-free events are blocked
    // separately when a removed subject has a tombstone.
    return false;
  }
  if (current.attemptId !== incoming.attemptId) return false;
  return current.runId != null && incoming.runId != null && incoming.runId < current.runId;
}

function addTombstone(
  removed: DownloadRemovedEvent[],
  event: DownloadRemovedEvent
): DownloadRemovedEvent[] {
  return [event, ...removed.filter((item) => item.gameId !== event.gameId || item.attemptId !== event.attemptId)].slice(0, 256);
}

function retireChangedAttempts(
  removed: DownloadRemovedEvent[],
  current: TorrentDownloadRecord[],
  next: TorrentDownloadRecord[]
): DownloadRemovedEvent[] {
  return next.reduce((result, incoming) => {
    const previous = current.find((item) => item.gameId === incoming.gameId);
    return previous?.attemptId && incoming.attemptId && previous.attemptId !== incoming.attemptId
      ? addTombstone(result, { gameId: previous.gameId, attemptId: previous.attemptId })
      : result;
  }, removed);
}

function stableProgressValues(
  current: Pick<TorrentDownloadRecord, 'status' | 'magnetUri' | 'progressPercent' | 'downloadedBytes' | 'totalBytes'> | null,
  next: Pick<TorrentDownloadRecord, 'progressPercent' | 'downloadedBytes' | 'totalBytes'>
) {
  if (!current || current.status === 'completed' || current.magnetUri.startsWith('direct:')) {
    return next;
  }

  return {
    progressPercent: Math.max(next.progressPercent, current.progressPercent),
    downloadedBytes: Math.max(next.downloadedBytes, current.downloadedBytes),
    totalBytes: Math.max(next.totalBytes, current.totalBytes)
  };
}

function prependActivity(
  events: ActivityEvent[],
  event: Omit<ActivityEvent, 'id' | 'timestamp'> & { timestamp?: string }
) {
  const timestamp = event.timestamp ?? new Date().toISOString();
  return [
    {
      ...event,
      id: `${timestamp}:${event.title}:${event.gameId ?? event.detail}`,
      timestamp
    },
    ...events
  ].slice(0, 12);
}
