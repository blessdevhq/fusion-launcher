import type { FusionCatalogEntry, FusionOperationProgress } from '../types/fusionSource.ts';
import { fusionGameKey } from '../types/fusionSource.ts';
import type { LibraryFilter, LibrarySort } from './libraryStatus.ts';

export function selectFusionCatalog(entries: FusionCatalogEntry[], filter: LibraryFilter, query: string, sort: LibrarySort, operations: Record<string, FusionOperationProgress>) {
  const needle = query.trim().toLocaleLowerCase();
  return entries.filter((game) => {
    const active = operations[fusionGameKey(game)]?.status === 'running';
    if (filter === 'installed' && !game.installed) return false;
    if (filter === 'missing' && game.installed) return false;
    if (filter === 'downloading' && !active) return false;
    return !needle || `${game.title} ${game.sourceName} ${game.gameId}`.toLocaleLowerCase().includes(needle);
  }).sort((a, b) => {
    if (sort === 'repository') return a.sourceName.localeCompare(b.sourceName) || a.title.localeCompare(b.title);
    if (sort === 'status') return Number(b.installed) - Number(a.installed) || a.title.localeCompare(b.title);
    return a.title.localeCompare(b.title);
  });
}
