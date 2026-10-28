import { PLATFORM_LABELS } from '../types/platform.ts';
import { fusionGameKey, type FusionCatalogEntry, type FusionOperationProgress } from '../types/fusionSource.ts';
import type { GameLibraryItem, LibraryFilter, LibrarySort, StatusTone } from './libraryStatus.ts';

interface LibraryPresentation {
  key: string;
  focusId: string;
  title: string;
  platform: string | null;
  sourceName: string;
  installed: boolean;
  readyToPlay: boolean;
  isDownloading: boolean;
  isPaused: boolean;
  hasError: boolean;
  needsSetup: boolean;
  statusLabel: string;
  statusTone: StatusTone;
  progressPercent: number;
  searchText: string;
}

/** Shared browsing data only. Installation and launch keep their original domains. */
export type UnifiedLibraryItem = LibraryPresentation & (
  | { kind: 'legacy'; item: GameLibraryItem }
  | { kind: 'fusion_source'; game: FusionCatalogEntry }
);

export interface LibraryCollectionCounts {
  all: number;
  ready: number;
  downloading: number;
  missing: number;
  platforms: Array<{ platform: string; count: number }>;
}

export function normalizeLibraryPlatform(platform: string | null | undefined): string | null {
  const normalized = platform?.trim().toLocaleLowerCase();
  if (!normalized) return null;
  return normalized === 'nintendo switch' ? 'switch' : normalized;
}

export function libraryPlatformLabel(platform: string | null | undefined): string {
  const normalized = normalizeLibraryPlatform(platform);
  if (!normalized) return '';
  if (normalized === 'pc') return 'PC';
  return PLATFORM_LABELS[normalized as keyof typeof PLATFORM_LABELS] ?? platform?.trim() ?? '';
}

export function buildUnifiedLibraryItems(
  legacy: GameLibraryItem[],
  fusion: FusionCatalogEntry[],
  operations: Record<string, FusionOperationProgress> = {},
  blockedFusionKeys: ReadonlySet<string> = new Set()
): UnifiedLibraryItem[] {
  const legacyItems: UnifiedLibraryItem[] = legacy.map((item) => ({
    kind: 'legacy', item,
    key: `legacy:${item.game.id}`,
    focusId: `game:library:${encodeURIComponent(item.game.id)}`,
    title: item.game.title,
    platform: normalizeLibraryPlatform(item.game.platform),
    sourceName: item.game.repositoryName,
    installed: item.installed,
    readyToPlay: item.readyToPlay,
    isDownloading: item.isDownloading,
    isPaused: item.isPaused,
    hasError: item.hasError,
    needsSetup: item.installed && item.missingRequirements.length > 0,
    statusLabel: item.statusLabel,
    statusTone: item.statusTone,
    progressPercent: item.progressPercent,
    searchText: [item.game.title, item.game.platform, libraryPlatformLabel(item.game.platform), item.game.repositoryName,
      item.game.sourceId, item.game.metadata?.developer, item.game.metadata?.publisher, item.game.metadata?.series,
      item.game.metadata?.releaseYear, ...(item.game.metadata?.genres ?? []), ...(item.game.metadata?.tags ?? []),
      item.statusLabel, ...item.missingRequirements].join(' ').toLocaleLowerCase()
  }));
  const fusionItems: UnifiedLibraryItem[] = fusion.map((game) => {
    const gameKey = fusionGameKey(game);
    const operation = operations[gameKey];
    const isDownloading = Boolean(operation && ['queued', 'running', 'cancelling'].includes(operation.status));
    const isPaused = operation?.status === 'interrupted';
    const hasError = operation?.status === 'failed';
    const readyToPlay = game.installed && game.readyToPlay && !isDownloading && !blockedFusionKeys.has(gameKey);
    const needsSetup = game.installed && !readyToPlay && !isDownloading;
    const statusLabel = isDownloading ? (operation?.status === 'cancelling' ? 'Cancelling' : 'Downloading')
      : needsSetup ? 'Missing Requirements'
      : game.installed && game.updateAvailable ? 'Update Available'
      : readyToPlay ? 'Ready to Play'
      : hasError ? 'Download Error'
      : isPaused ? 'Interrupted' : 'Not Installed';
    const transfer = operation?.transfer;
    const progressPercent = transfer && transfer.totalBytes > 0
      ? Math.max(0, Math.min(100, transfer.downloadedBytes / transfer.totalBytes * 100)) : 0;
    return {
      kind: 'fusion_source', game,
      key: `fusion_source:${gameKey}`,
      focusId: `fusion-game:${encodeURIComponent(gameKey)}`,
      title: game.title,
      platform: normalizeLibraryPlatform(game.platform),
      sourceName: game.sourceName,
      installed: game.installed, readyToPlay, isDownloading, isPaused, hasError, needsSetup,
      statusLabel,
      statusTone: isDownloading ? 'active' : needsSetup ? 'missing' : readyToPlay ? 'ready' : hasError ? 'error' : isPaused ? 'paused' : 'idle',
      progressPercent,
      searchText: [game.title, game.gameId, game.sourceName, game.platform, libraryPlatformLabel(game.platform), statusLabel].join(' ').toLocaleLowerCase()
    };
  });
  return [...legacyItems, ...fusionItems];
}

export function filterUnifiedLibraryItems(items: UnifiedLibraryItem[], filter: LibraryFilter): UnifiedLibraryItem[] {
  if (filter === 'ready') return items.filter((item) => item.readyToPlay);
  if (filter === 'installed') return items.filter((item) => item.installed);
  if (filter === 'downloading') return items.filter((item) => item.isDownloading || item.isPaused || item.hasError);
  if (filter === 'missing') return items.filter((item) => item.needsSetup);
  return items;
}

export function selectUnifiedLibraryItems(
  items: UnifiedLibraryItem[], filter: LibraryFilter, query: string, sort: LibrarySort, platform?: string | null
): UnifiedLibraryItem[] {
  const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const selectedPlatform = normalizeLibraryPlatform(platform);
  return filterUnifiedLibraryItems(items, filter)
    .filter((item) => (!selectedPlatform || item.platform === selectedPlatform) && terms.every((term) => item.searchText.includes(term)))
    .sort((left, right) => {
      const compare = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: 'base', numeric: true });
      const rank = (item: UnifiedLibraryItem) => item.readyToPlay ? 0 : item.isDownloading ? 1 : item.isPaused ? 2 : item.hasError ? 3 : item.installed ? 4 : 5;
      const primary = sort === 'platform' ? compare(left.platform ?? '', right.platform ?? '')
        : sort === 'repository' ? compare(left.sourceName, right.sourceName)
        : sort === 'status' ? rank(left) - rank(right) : 0;
      return primary || compare(left.title, right.title) || compare(left.key, right.key);
    });
}

export function countUnifiedLibraryItems(items: UnifiedLibraryItem[]): LibraryCollectionCounts {
  const byPlatform = new Map<string, number>();
  for (const item of items) if (item.platform) byPlatform.set(item.platform, (byPlatform.get(item.platform) ?? 0) + 1);
  return {
    all: items.length,
    ready: filterUnifiedLibraryItems(items, 'ready').length,
    downloading: filterUnifiedLibraryItems(items, 'downloading').length,
    missing: filterUnifiedLibraryItems(items, 'missing').length,
    platforms: Array.from(byPlatform, ([platform, count]) => ({ platform, count })).sort((a, b) => a.platform.localeCompare(b.platform))
  };
}
