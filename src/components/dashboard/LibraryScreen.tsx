import { useEffect, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import { Search, X } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import { GamePoster } from '@/components/shell/GamePoster';
import type { GameLibraryItem, LibraryFilter, LibrarySort } from '@/lib/libraryStatus';
import type { CatalogGame } from '@/types/repository';
import { FILTERS, SORTS } from './constants';
import { ScreenHeader } from './shared';
import type { BusyAction } from './types';
import { filterCountLabel } from './utils';

const LIBRARY_PAGE_SIZE = 60;

export function LibraryScreen({
  items,
  allItems,
  totalCount,
  filter,
  query,
  sort,
  busyAction,
  onFilterChange,
  onQueryChange,
  onSortChange,
  onPrimaryAction,
  onOpenDetails,
  onFocus
}: {
  items: GameLibraryItem[];
  allItems: GameLibraryItem[];
  totalCount: number;
  filter: LibraryFilter;
  query: string;
  sort: LibrarySort;
  busyAction: BusyAction;
  onFilterChange: (filter: LibraryFilter) => void;
  onQueryChange: (query: string) => void;
  onSortChange: (sort: LibrarySort) => void;
  onPrimaryAction: (item: GameLibraryItem) => void;
  onOpenDetails: (game: CatalogGame) => void;
  onFocus: (focusId: string) => void;
}) {
  const { t } = useI18n();
  const gridRef = useRef<HTMLDivElement | null>(null);
  const sentinelRef = useRef<HTMLDivElement | null>(null);
  const [visibleCount, setVisibleCount] = useState(LIBRARY_PAGE_SIZE);

  // Reset the render window whenever the filtered/sorted/searched set changes.
  useEffect(() => {
    setVisibleCount(LIBRARY_PAGE_SIZE);
  }, [items]);

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

  return (
    <motion.section initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} className="rh-screen rh-panel" data-testid="library-screen">
      <ScreenHeader eyebrow={t.dashboard.library.eyebrow} title={t.dashboard.library.title} description={t.dashboard.library.description(items.length, totalCount)} />
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
      <div className="mb-4 flex flex-wrap gap-2" data-testid="library-filters">
        {FILTERS.map((item) => {
          const count = filterCountLabel(item, totalCount, allItems);
          return (
            <button
              key={item}
              data-focus-id={`filter:${item}`}
              data-focus-zone="library-filters"
              onFocus={() => onFocus(`filter:${item}`)}
              onClick={() => onFilterChange(item)}
              className={`rh-filter-chip rh-focusable ${filter === item ? 'rh-filter-chip-active' : ''}`}
            >
              {t.dashboard.filters[item]}
              <span>{count}</span>
            </button>
          );
        })}
      </div>
      <div ref={gridRef} className="rh-library-grid" data-testid="library-grid">
        {items.length === 0 ? (
          <div className="rh-empty-compact" data-testid="library-empty">{t.dashboard.library.empty}</div>
        ) : (
          <>
            {renderedItems.map((item) => (
              <GamePoster
                key={item.game.id}
                item={item}
                focusId={`game:library:${encodeURIComponent(item.game.id)}`}
                zone="library"
                selected={busyAction?.endsWith(item.game.id)}
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
