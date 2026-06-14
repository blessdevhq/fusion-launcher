import { mergeRailItems, type HomeRail } from '@/components/shell/CockpitPanels';
import { updateErrorText, type UiText } from '@/lib/i18n';
import { filterLibraryItems, type GameLibraryItem, type LibraryFilter } from '@/lib/libraryStatus';
import type { TorrentDownloadRecord, UpdateCheckError } from '@/types/repository';
import { ACTIVE_DOWNLOAD_STATUSES } from './constants';
import type { UpdatePanelState } from './types';

export function updateNotificationText(state: UpdatePanelState, t: UiText, locale: string) {
  if (state.phase === 'idle') return t.dashboard.topbar.notifications.updateIdle;
  if (state.phase === 'checking') return t.settings.updates.checking;
  if (state.phase === 'installing') return t.settings.updates.installing;
  if (state.phase === 'up-to-date') return t.settings.updates.upToDate(state.report?.currentVersion);
  if (state.phase === 'available') return t.settings.updates.available(state.report?.version);
  return updateErrorText(state.error, locale);
}

export function composeHomeRails(
  items: GameLibraryItem[],
  t: UiText
): { heroItem: GameLibraryItem | null; rails: HomeRail[] } {
  const ready = items.filter((item) => item.readyToPlay);
  const downloading = items.filter((item) => item.isDownloading || item.isPaused || item.hasError);
  const needsSetup = items.filter((item) => item.missingRequirements.length > 0);
  const heroItem = ready[0] ?? downloading[0] ?? items[0] ?? null;

  const rails: HomeRail[] = [
    {
      title: t.dashboard.rails.continuePlaying,
      testId: 'home-rail-ready',
      zone: 'ready',
      items: mergeRailItems(ready, [], 12)
    },
    {
      title: t.dashboard.rails.downloads,
      testId: 'home-rail-downloads',
      zone: 'downloads',
      items: mergeRailItems(downloading, [], 12)
    },
    {
      title: t.dashboard.rails.needsSetup,
      testId: 'home-rail-setup',
      zone: 'setup',
      items: mergeRailItems(needsSetup, [], 12)
    },
    {
      title: t.dashboard.rails.recentlyAdded,
      testId: 'home-rail-recent',
      zone: 'recent',
      items: mergeRailItems(items, [], 12)
    }
  ];

  return { heroItem, rails };
}

export function displayActivityTitle(title: string, t: UiText) {
  if (title === 'Download completed') return t.dashboard.messages.downloadCompleted;
  return title;
}

export function normalizeUpdateCheckError(error: unknown): UpdateCheckError {
  if (isUpdateCheckError(error)) return error;
  if (typeof error === 'object' && error !== null && 'kind' in error) {
    const kind = String((error as { kind?: unknown }).kind);
    if (kind === 'endpointUnreachable' || kind === 'parseError' || kind === 'signatureInvalid') {
      const message = 'message' in error ? String((error as { message?: unknown }).message ?? '') : undefined;
      return { kind, message };
    }
  }

  const message = error instanceof Error ? error.message : String(error);
  return { kind: 'parseError', message };
}

function isUpdateCheckError(error: unknown): error is UpdateCheckError {
  return Boolean(
    error &&
    typeof error === 'object' &&
    'kind' in error &&
    ['endpointUnreachable', 'parseError', 'signatureInvalid'].includes(String((error as { kind?: unknown }).kind))
  );
}

export function formatEventTime(timestamp: string, locale: string) {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return '';
  return date.toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit' });
}

export function formatBytes(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 MB';
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export function formatSpeed(bytesPerSecond: number) {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) return '0.00 MB/s';
  return `${(bytesPerSecond / 1024 / 1024).toFixed(2)} MB/s`;
}

export function formatClock(date: Date, locale: string) {
  return date.toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit' });
}

export function filterCountLabel(filter: LibraryFilter, totalCount: number, allItems: GameLibraryItem[]) {
  if (filter === 'all') return totalCount;
  return filterLibraryItems(allItems, filter).length;
}

export function summarizeDownloads(downloads: TorrentDownloadRecord[]) {
  return downloads.reduce((summary, download) => {
    if (ACTIVE_DOWNLOAD_STATUSES.includes(download.status)) summary.active += 1;
    if (download.status === 'paused' || download.status === 'interrupted') summary.paused += 1;
    if (download.status === 'error') summary.errors += 1;
    summary.downloadedBytes += download.downloadedBytes;
    return summary;
  }, {
    active: 0,
    paused: 0,
    errors: 0,
    downloadedBytes: 0
  });
}

export function downloadStatusHint(download: TorrentDownloadRecord, t: UiText) {
  if (download.status === 'interrupted') return t.dashboard.downloads.statusHints.interrupted;
  if (download.status === 'paused') return t.dashboard.downloads.statusHints.paused;
  if (download.status === 'resolving') return t.dashboard.downloads.statusHints.resolving;
  if (download.status === 'cancelling') return t.dashboard.downloads.statusHints.cancelling;
  if (download.status === 'cancelled') return t.dashboard.downloads.statusHints.cancelled;
  if (download.status === 'error' && !download.errorMessage) return t.dashboard.downloads.statusHints.error;
  return null;
}

export function safeDecodeURIComponent(value: string) {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

export function cssEscape(value: string) {
  if (typeof CSS !== 'undefined' && CSS.escape) {
    return CSS.escape(value);
  }

  return value.replace(/"/g, '\\"');
}
