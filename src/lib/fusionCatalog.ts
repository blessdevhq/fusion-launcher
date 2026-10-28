import type { FusionCatalogEntry, FusionOperationProgress } from '../types/fusionSource.ts';
import type { LibraryFilter, LibrarySort } from './libraryStatus.ts';
import { buildUnifiedLibraryItems, selectUnifiedLibraryItems } from './unifiedLibrary.ts';

export function selectFusionCatalog(entries: FusionCatalogEntry[], filter: LibraryFilter, query: string, sort: LibrarySort, operations: Record<string, FusionOperationProgress>) {
  return selectUnifiedLibraryItems(buildUnifiedLibraryItems([], entries, operations), filter, query, sort)
    .flatMap((item) => item.kind === 'fusion_source' ? [item.game] : []);
}
