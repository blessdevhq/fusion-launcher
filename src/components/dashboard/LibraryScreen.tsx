import { useEffect, useMemo, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import { Search, X } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import { FusionGamePoster, GamePoster } from '@/components/shell/GamePoster';
import { CollectionsPanel, collectionIdForTarget, type CollectionTarget } from '@/components/shell/CockpitPanels';
import type { GameLibraryItem, LibraryFilter, LibrarySort } from '@/lib/libraryStatus';
import type { CatalogGame } from '@/types/repository';
import { SORTS } from './constants';
import { ScreenHeader } from './shared';
import type { BusyAction } from './types';
import type { FusionCatalogEntry } from '@/types/fusionSource';
import { countUnifiedLibraryItems, type UnifiedLibraryItem } from '@/lib/unifiedLibrary';

const LIBRARY_PAGE_SIZE = 60;

export function LibraryScreen({
  items,
  allItems,
  filter,
  platform,
  query,
  sort,
  busyAction,
  onQueryChange,
  onSortChange,
  onOpenCollection,
  onPrimaryAction,
  onOpenDetails,
  onOpenFusion,
  onFocus
}: {
  items: UnifiedLibraryItem[];
  allItems: UnifiedLibraryItem[];
  filter: LibraryFilter;
  platform: string | null;
  query: string;
  sort: LibrarySort;
  busyAction: BusyAction;
  onQueryChange: (query: string) => void;
  onSortChange: (sort: LibrarySort) => void;
  onOpenCollection: (target: CollectionTarget) => void;
  onPrimaryAction: (item: GameLibraryItem) => void;
  onOpenDetails: (game: CatalogGame) => void;
  onOpenFusion: (game: FusionCatalogEntry) => void;
  onFocus: (focusId: string) => void;
}) {
  const { t } = useI18n();
  const gridRef = useRef<HTMLDivElement | null>(null);
  const sentinelRef = useRef<HTMLDivElement | null>(null);
  const [visibleCount, setVisibleCount] = useState(LIBRARY_PAGE_SIZE);

  // Reset the render window whenever the filtered/sorted/searched set changes.
  useEffect(() => {
    setVisibleCount(LIBRARY_PAGE_SIZE);
  }, [filter, query, sort, platform, allItems.length]);

  // Grow the window as the bottom sentinel scrolls into view, so large catalogs
  // never mount thousands of cards at once (which froze the initial render).
  useEffect(() => {
    const sentinel = sentinelRef.current;
    if (!sentinel || visibleCount >= items.length) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setVisibleCount((count) => Math.min(count + LIBRARY_PAGE_SIZE, items.length));
        }
      },
      { root: gridRef.current, rootMargin: '600px' }
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [items.length, visibleCount]);

  const renderedItems = items.slice(0, visibleCount);
  const activeCollectionId = collectionIdForTarget({ filter, query, sort, platform });
  const counts = useMemo(() => countUnifiedLibraryItems(allItems), [allItems]);

  return (
    <motion.section initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} className="rh-screen rh-panel" data-testid="library-screen">
      <ScreenHeader eyebrow={t.dashboard.library.eyebrow} title={t.dashboard.library.title} description={t.dashboard.library.description(items.length, allItems.length)} />
      <div className="rh-library-collections">
        <CollectionsPanel
          counts={counts}
          activeCollectionId={activeCollectionId}
          onOpenCollection={onOpenCollection}
          onFocus={onFocus}
        />
      </div>
      <div className="rh-library-toolbar">
        <div className="rh-library-search">
          <Search className="h-4 w-4 text-white/42" />
          <input
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
            placeholder={t.dashboard.library.searchPlaceholder}
            data-testid="library-search"
          />
          {query && (
            <button onClick={() => onQueryChange('')} className="rh-search-clear" title={t.dashboard.library.clearSearch}>
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
        <select
          value={sort}
          onChange={(event) => onSortChange(event.target.value as LibrarySort)}
          className="rh-library-sort"
          aria-label={t.dashboard.library.sortAria}
          data-testid="library-sort"
        >
          {SORTS.map((item) => (
            <option key={item} value={item}>{t.dashboard.sorts[item]}</option>
          ))}
        </select>
      </div>
      <div ref={gridRef} className="rh-library-grid" data-testid="library-grid">
        {items.length === 0 ? (
          <div className="rh-empty-compact" data-testid="library-empty">{t.dashboard.library.empty}</div>
        ) : (
          <>
            {renderedItems.map((entry) => entry.kind === 'fusion_source' ? (
              <FusionGamePoster key={entry.key} item={entry} onOpen={onOpenFusion} onFocus={onFocus} />
            ) : (
              <GamePoster
                key={entry.key}
                item={entry.item}
                focusId={entry.focusId}
                zone="library"
                selected={busyAction?.endsWith(entry.item.game.id)}
                onOpen={onOpenDetails}
                onAction={onPrimaryAction}
                onFocus={onFocus}
              />
            ))}
            {visibleCount < items.length && (
              <div ref={sentinelRef} className="rh-library-sentinel" aria-hidden="true" data-testid="library-sentinel" />
            )}
          </>
        )}
      </div>
    </motion.section>
  );
}
