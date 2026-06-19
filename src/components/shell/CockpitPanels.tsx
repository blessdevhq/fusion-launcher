'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import {
  ChevronLeft,
  ChevronRight,
  Download,
  FolderHeart,
  Gamepad2,
  type LucideIcon,
  MoreHorizontal,
  Play,
  RotateCw
} from 'lucide-react';
import { GameArt, GamePoster } from './GamePoster';
import { useI18n } from '../I18nProvider';
import { displayProductText } from '../../lib/brandText.ts';
import { PLATFORM_LABELS } from '../../types/platform.ts';
import type { GameLibraryItem, LibraryFilter, LibrarySort } from '../../lib/libraryStatus.ts';
import type { CatalogGame } from '../../types/repository.ts';

export interface CollectionTarget {
  filter: LibraryFilter;
  query: string;
  sort: LibrarySort;
}

interface HeroPanelProps {
  heroItem: GameLibraryItem | null;
  busyAction: string | null;
  onPrimaryAction: (item: GameLibraryItem) => void;
  onOpenDetails: (game: CatalogGame) => void;
  onOpenSettings: () => void;
  onFocus: (focusId: string) => void;
}

export interface HomeRail {
  title: string;
  testId: string;
  zone: string;
  items: GameLibraryItem[];
}

export function HeroPanel({
  heroItem,
  busyAction,
  onPrimaryAction,
  onOpenDetails,
  onOpenSettings,
  onFocus
}: HeroPanelProps) {
  const { t } = useI18n();

  if (!heroItem) {
    return (
      <section className="rh-hero-panel rh-hero-empty-panel" data-testid="home-hero">
        <div className="rh-home-empty">
          <div>
            <div className="rh-home-empty-title">{t.shell.hero.emptyTitle}</div>
            <p className="rh-home-empty-copy">{t.shell.hero.emptyCopy}</p>
          </div>
          <button
            data-focus-id="home:settings"
            data-focus-zone="hero"
            onFocus={() => onFocus('home:settings')}
            onClick={onOpenSettings}
          className="rh-primary-action rh-focusable"
        >
            {t.shell.hero.openSettings}
          </button>
        </div>
      </section>
    );
  }

  const progressVisible = heroItem.isDownloading || heroItem.isPaused || heroItem.hasError
    || (heroItem.progressPercent > 0 && heroItem.progressPercent < 100);
  const metaItems = [
    `${t.shell.hero.status}: ${displayStatusLabel(heroItem.statusLabel, t)}`,
    `${t.shell.hero.source}: ${displayProductText(heroItem.game.repositoryName)}`,
    progressVisible ? `${t.shell.hero.progress}: ${heroItem.progressPercent.toFixed(0)}%` : null
  ].filter((item): item is string => Boolean(item));
  const setupHint = heroItem.missingRequirements[0] ?? null;

  return (
    <section className="rh-hero-panel" data-testid="home-hero">
      <div className="rh-hero-bg">
        <GameArt game={heroItem.game} className="h-full w-full" hero />
      </div>
      <div className="rh-hero-scrim" />

      <div className="rh-hero-content">
        <div className="rh-hero-copy-stack">
          <div className="rh-hero-kicker">
            <span className="rh-platform-chip">{heroItem.game.platform}</span>
            <span>{heroItem.game.metadata?.releaseYear ?? t.shell.hero.mvpDemo}</span>
            <span>/</span>
            <span>{heroItem.game.metadata?.genres?.[0] ?? PLATFORM_LABELS[heroItem.game.platform]}</span>
          </div>
          <h1 className="rh-hero-title">
            {displayProductText(heroItem.game.title)}
          </h1>
          {heroItem.game.description && (
            <p className="rh-hero-description">{displayProductText(heroItem.game.description)}</p>
          )}
          {setupHint && (
            <div className="rh-hero-alert">
              <RotateCw className="h-3.5 w-3.5" />
              <span>{setupHint}</span>
            </div>
          )}
          <div className="rh-hero-actions">
            <button
              data-focus-id={`action:${encodeURIComponent(heroItem.game.id)}`}
              data-focus-zone="hero"
              onFocus={() => onFocus(`action:${encodeURIComponent(heroItem.game.id)}`)}
              onClick={() => onPrimaryAction(heroItem)}
              disabled={busyAction !== null}
              className="rh-primary-action rh-focusable"
            >
              {heroItem.primaryAction === 'play' ? <Play className="h-4 w-4" /> : heroItem.primaryAction === 'download' || heroItem.primaryAction === 'import' ? <Download className="h-4 w-4" /> : <RotateCw className="h-4 w-4" />}
              {displayActionLabel(heroItem.primaryActionLabel, t)}
            </button>
            <button
              data-focus-id={`details:${encodeURIComponent(heroItem.game.id)}`}
              data-focus-zone="hero"
              onFocus={() => onFocus(`details:${encodeURIComponent(heroItem.game.id)}`)}
              onClick={() => onOpenDetails(heroItem.game)}
              className="rh-square-action rh-focusable"
              title={t.shell.hero.detailsTitle}
            >
              <MoreHorizontal className="h-4 w-4" />
            </button>
          </div>
          <div className="rh-hero-meta">
            {metaItems.map((item) => <span key={item}>{item}</span>)}
          </div>
        </div>
      </div>
    </section>
  );
}

export function HomeRailsPanel({
  rails,
  onPrimaryAction,
  onOpenDetails,
  onFocus
}: {
  rails: HomeRail[];
  onPrimaryAction: (item: GameLibraryItem) => void;
  onOpenDetails: (game: CatalogGame) => void;
  onFocus: (focusId: string) => void;
}) {
  const visibleRails = rails.filter((rail) => rail.items.length > 0).slice(0, 4);

  if (visibleRails.length === 0) return null;

  return (
    <div className="rh-home-rails">
      {visibleRails.map((rail) => (
        <MiniRail
          key={rail.testId}
          title={rail.title}
          testId={rail.testId}
          items={rail.items}
          zone={rail.zone}
          onOpenDetails={onOpenDetails}
          onPrimaryAction={onPrimaryAction}
          onFocus={onFocus}
        />
      ))}
    </div>
  );
}

export function CollectionsPanel({
  items,
  activeCollectionId = 'all',
  onOpenCollection,
  onFocus
}: {
  items: GameLibraryItem[];
  activeCollectionId?: string | null;
  onOpenCollection: (target: CollectionTarget) => void;
  onFocus: (focusId: string) => void;
}) {
  const { t } = useI18n();
  const byPlatform = new Map<string, number>();
  items.forEach((item) => byPlatform.set(item.game.platform, (byPlatform.get(item.game.platform) ?? 0) + 1));
  const readyCount = items.filter((item) => item.readyToPlay).length;
  const downloadCount = items.filter((item) => item.isDownloading || item.isPaused || item.hasError).length;
  const missingCount = items.filter((item) => item.missingRequirements.length > 0).length;
  const collectionCards: Array<{
    id: string;
    label: string;
    count: number;
    icon: LucideIcon;
  }> = [
    { id: 'all', label: t.shell.collections.all, count: items.length, icon: FolderHeart },
    { id: 'ready', label: t.shell.collections.ready, count: readyCount, icon: Play },
    { id: 'downloads', label: t.shell.collections.downloads, count: downloadCount, icon: Download },
    { id: 'missing', label: t.shell.collections.missing, count: missingCount, icon: RotateCw },
    ...Array.from(byPlatform.entries()).slice(0, 6).map(([platform, count]) => ({
      id: `platform:${platform}`,
      label: PLATFORM_LABELS[platform as keyof typeof PLATFORM_LABELS] ?? platform,
      count,
      icon: Gamepad2
    }))
  ];

  return (
    <section className="rh-collections-panel" data-testid="collections-panel">
      <PanelTitle icon={FolderHeart} title={t.shell.collections.title} />
      <div className="rh-collections-grid">
        {collectionCards.slice(0, 8).map((card) => {
          const Icon = card.icon;
          const focusId = `collection:${card.id}`;
          const active = activeCollectionId?.toLowerCase() === card.id.toLowerCase();
          return (
            <button
              key={card.label}
              data-testid="collection-card"
              data-focus-id={focusId}
              data-focus-zone="collections"
              onFocus={() => onFocus(focusId)}
              onClick={() => onOpenCollection(collectionTargetForId(card.id))}
              className={`rh-collection-card rh-focusable ${active ? 'rh-collection-card-active' : ''}`}
            >
              <Icon className="h-4 w-4" />
              <span className="mt-3 block text-sm font-semibold">{card.label}</span>
              <span className="mt-1 block text-xs text-white/48">{t.shell.collections.count(card.count)}</span>
            </button>
          );
        })}
      </div>
    </section>
  );
}

export function collectionTargetForId(id: string): CollectionTarget {
  if (id.startsWith('platform:')) {
    return { filter: 'all', query: id.slice('platform:'.length), sort: 'title' };
  }
  if (id === 'ready') return { filter: 'all', query: 'ready to play', sort: 'status' };
  if (id === 'downloads') return { filter: 'downloading', query: '', sort: 'status' };
  if (id === 'missing') return { filter: 'missing', query: '', sort: 'status' };
  return { filter: 'all', query: '', sort: 'title' };
}

export function collectionIdForTarget(target: CollectionTarget): string | null {
  const query = target.query.trim().toLowerCase();
  if (target.filter === 'all' && query === '' && target.sort === 'title') return 'all';
  if (target.filter === 'all' && query === 'ready to play' && target.sort === 'status') return 'ready';
  if (target.filter === 'downloading' && query === '' && target.sort === 'status') return 'downloads';
  if (target.filter === 'missing' && query === '' && target.sort === 'status') return 'missing';
  if (target.filter === 'all' && query && target.sort === 'title') return `platform:${query}`;
  return null;
}

function MiniRail({
  title,
  testId,
  items,
  zone,
  onOpenDetails,
  onPrimaryAction,
  onFocus
}: {
  title: string;
  testId: string;
  items: GameLibraryItem[];
  zone: string;
  onOpenDetails: (game: CatalogGame) => void;
  onPrimaryAction: (item: GameLibraryItem) => void;
  onFocus: (focusId: string) => void;
}) {
  const trackRef = useRef<HTMLDivElement | null>(null);
  const [canScrollLeft, setCanScrollLeft] = useState(false);
  const [canScrollRight, setCanScrollRight] = useState(false);

  const updateScrollState = useCallback(() => {
    const track = trackRef.current;
    if (!track) return;

    const maxScrollLeft = Math.max(0, track.scrollWidth - track.clientWidth);
    setCanScrollLeft(track.scrollLeft > 1);
    setCanScrollRight(track.scrollLeft < maxScrollLeft - 1);
  }, []);

  const handleWheel = useCallback((event: globalThis.WheelEvent) => {
    const track = trackRef.current;
    if (!track || track.scrollWidth <= track.clientWidth) return;

    const delta = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY;
    if (delta === 0) return;

    const maxScrollLeft = track.scrollWidth - track.clientWidth;
    const atStart = track.scrollLeft <= 1;
    const atEnd = track.scrollLeft >= maxScrollLeft - 1;
    if ((delta < 0 && atStart) || (delta > 0 && atEnd)) return;

    event.preventDefault();
    track.scrollLeft += delta;
    updateScrollState();
  }, [updateScrollState]);

  useEffect(() => {
    updateScrollState();

    const track = trackRef.current;
    if (!track) return;

    let resizeObserver: ResizeObserver | null = null;
    if (typeof ResizeObserver !== 'undefined') {
      resizeObserver = new ResizeObserver(updateScrollState);
      resizeObserver.observe(track);
    }

    track.addEventListener('wheel', handleWheel, { passive: false });
    window.addEventListener('resize', updateScrollState);
    return () => {
      track.removeEventListener('wheel', handleWheel);
      resizeObserver?.disconnect();
      window.removeEventListener('resize', updateScrollState);
    };
  }, [handleWheel, items.length, updateScrollState]);

  const scrollRail = useCallback((direction: -1 | 1) => {
    const track = trackRef.current;
    if (!track) return;

    const distance = Math.max(track.clientWidth * 0.82, 180);
    track.scrollBy({ left: direction * distance, behavior: 'smooth' });
    window.setTimeout(updateScrollState, 180);
  }, [updateScrollState]);

  if (items.length === 0) return null;

  return (
    <div className="rh-mini-rail" data-testid={testId}>
      <div className="rh-mini-rail-header">
        <div className="rh-mini-rail-title">{title}</div>
        <div className="rh-mini-rail-actions">
          <button
            type="button"
            data-focus-id={`rail:${zone}:left`}
            data-focus-zone={zone}
            onFocus={() => onFocus(`rail:${zone}:left`)}
            onClick={() => scrollRail(-1)}
            disabled={!canScrollLeft}
            className="rh-rail-arrow rh-focusable"
            aria-label={`${title}: scroll left`}
            title={`${title}: scroll left`}
          >
            <ChevronLeft className="h-4 w-4" />
          </button>
          <button
            type="button"
            data-focus-id={`rail:${zone}:right`}
            data-focus-zone={zone}
            onFocus={() => onFocus(`rail:${zone}:right`)}
            onClick={() => scrollRail(1)}
            disabled={!canScrollRight}
            className="rh-rail-arrow rh-focusable"
            aria-label={`${title}: scroll right`}
            title={`${title}: scroll right`}
          >
            <ChevronRight className="h-4 w-4" />
          </button>
        </div>
      </div>
      <div className="rh-mini-rail-viewport">
        <div ref={trackRef} className="rh-mini-rail-track" onScroll={updateScrollState}>
          {items.map((item) => (
            <GamePoster
              key={`${zone}:${item.game.id}`}
              item={item}
              compact
              focusId={`game:${zone}:${encodeURIComponent(item.game.id)}`}
              zone={zone}
              onOpen={onOpenDetails}
              onAction={onPrimaryAction}
              onFocus={onFocus}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

export function mergeRailItems(primaryItems: GameLibraryItem[], fallbackItems: GameLibraryItem[], limit: number) {
  const seen = new Set<string>();
  const result: GameLibraryItem[] = [];

  for (const item of [...primaryItems, ...fallbackItems]) {
    if (seen.has(item.game.id)) continue;
    seen.add(item.game.id);
    result.push(item);
    if (result.length >= limit) break;
  }

  return result;
}

function PanelTitle({ icon: Icon, title }: { icon: LucideIcon; title: string }) {
  return (
    <div className="flex items-center gap-2 text-base font-bold">
      <Icon className="h-4 w-4 text-white/52" />
      {title}
    </div>
  );
}

function displayActionLabel(label: string, t: ReturnType<typeof useI18n>['t']) {
  return t.actions[label as keyof typeof t.actions] ?? label;
}

function displayStatusLabel(label: string, t: ReturnType<typeof useI18n>['t']) {
  return t.statusLabels[label as keyof typeof t.statusLabels] ?? label;
}
