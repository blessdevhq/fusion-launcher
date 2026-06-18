import { create } from 'zustand';
import { DEFAULT_SETTINGS, type AppSettings } from '../lib/settings.ts';
import type {
  CatalogGame,
  DownloadProgressEvent,
  LibraryGameStatus,
  RepositorySummary,
  TorrentDownloadRecord
} from '../types/repository.ts';

export type LauncherView = 'home' | 'library' | 'explore' | 'downloads' | 'collections';

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
  repositories: RepositorySummary[];
  libraryStatuses: LibraryGameStatus[];
  downloads: TorrentDownloadRecord[];
  settings: AppSettings;
  activeView: LauncherView;
  focusedItemId: string | null;
  selectedGameId: string | null;
  activityEvents: ActivityEvent[];
  setCatalog: (catalog: CatalogGame[]) => void;
  setRepositories: (repositories: RepositorySummary[]) => void;
  setLibraryStatuses: (statuses: LibraryGameStatus[]) => void;
  setDownloads: (downloads: TorrentDownloadRecord[]) => void;
  setSettings: (settings: AppSettings) => void;
  setActiveView: (view: LauncherView) => void;
  setFocusedItemId: (focusedItemId: string | null) => void;
  setSelectedGameId: (selectedGameId: string | null) => void;
  mergeDownloadEvent: (event: DownloadProgressEvent) => void;
  addActivityEvent: (event: Omit<ActivityEvent, 'id' | 'timestamp'> & { timestamp?: string }) => void;
}

export const useLauncherStore = create<LauncherState>((set) => ({
  catalog: [],
  repositories: [],
  libraryStatuses: [],
  downloads: [],
  settings: DEFAULT_SETTINGS,
  activeView: 'home',
  focusedItemId: null,
  selectedGameId: null,
  activityEvents: [],
  setCatalog: (catalog) => set({ catalog }),
  setRepositories: (repositories) => set({ repositories }),
  setLibraryStatuses: (libraryStatuses) => set({ libraryStatuses }),
  setDownloads: (downloads) => set((state) => ({
    downloads: mergeDownloadSnapshotIntoList(state.downloads, downloads)
  })),
  setSettings: (settings) => set({ settings }),
  setActiveView: (activeView) => set({ activeView }),
  setFocusedItemId: (focusedItemId) => set({ focusedItemId }),
  setSelectedGameId: (selectedGameId) => set({ selectedGameId }),
  mergeDownloadEvent: (event) => set((state) => {
    const downloads = mergeDownloadEventIntoList(state.downloads, event);
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
      libraryStatuses,
      activityEvents: event.finished
        ? prependActivity(state.activityEvents, {
            title: 'Download completed',
            detail: event.gameId,
            gameId: event.gameId,
            tone: 'success'
          })
        : state.activityEvents
    };
  }),
  addActivityEvent: (event) => set((state) => ({
    activityEvents: prependActivity(state.activityEvents, event)
  }))
}));

export function mergeDownloadEventIntoList(
  downloads: TorrentDownloadRecord[],
  event: DownloadProgressEvent
): TorrentDownloadRecord[] {
  const existingIndex = downloads.findIndex((download) => download.gameId === event.gameId);
  const existing = existingIndex >= 0 ? downloads[existingIndex] : null;
  const merged = mergeDownloadRecord(existing ?? null, event);
  if (existingIndex < 0) return [merged, ...downloads];
  return downloads.map((download, index) => index === existingIndex ? merged : download);
}

export function mergeDownloadSnapshotIntoList(
  current: TorrentDownloadRecord[],
  incoming: TorrentDownloadRecord[]
): TorrentDownloadRecord[] {
  if (current.length === 0) return incoming;

  const currentIds = new Set(current.map((download) => download.gameId));
  const incomingById = new Map(incoming.map((download) => [download.gameId, download]));
  const updatedCurrent = current.flatMap((download) => {
    const next = incomingById.get(download.gameId);
    return next ? [mergeDownloadSnapshotRecord(download, next)] : [];
  });
  const newDownloads = incoming.filter((download) => !currentIds.has(download.gameId));

  return [...newDownloads, ...updatedCurrent];
}

export function mergeDownloadRecord(
  record: TorrentDownloadRecord | null,
  event: DownloadProgressEvent
): TorrentDownloadRecord {
  const now = new Date().toISOString();
  const progress = stableProgressValues(record, {
    progressPercent: event.progressPercent,
    downloadedBytes: event.downloadedBytes,
    totalBytes: event.totalBytes
  });

  return {
    gameId: event.gameId,
    subjectType: event.subjectType ?? record?.subjectType ?? null,
    displayName: event.displayName ?? record?.displayName ?? null,
    magnetUri: record?.magnetUri ?? '',
    saveDir: event.saveDir || record?.saveDir || '',
    status: event.status,
    progressPercent: progress.progressPercent,
    downloadedBytes: progress.downloadedBytes,
    totalBytes: progress.totalBytes,
    downloadSpeedBytesPerSec: event.downloadSpeedBytesPerSec,
    uploadSpeedBytesPerSec: event.uploadSpeedBytesPerSec,
    peersCount: event.peersCount,
    torrentId: record?.torrentId ?? null,
    errorMessage: event.error ?? null,
    createdAt: record?.createdAt ?? now,
    updatedAt: now,
    completedAt: event.finished ? now : record?.completedAt ?? null
  };
}

function mergeDownloadSnapshotRecord(
  current: TorrentDownloadRecord,
  incoming: TorrentDownloadRecord
): TorrentDownloadRecord {
  const progress = stableProgressValues(current, incoming);

  return {
    ...incoming,
    progressPercent: progress.progressPercent,
    downloadedBytes: progress.downloadedBytes,
    totalBytes: progress.totalBytes
  };
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
