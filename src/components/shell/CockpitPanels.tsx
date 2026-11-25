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
import { GameArt, GamePoster, FusionGamePoster } from './GamePoster';
import { useI18n } from '../I18nProvider';
import { displayProductText } from '../../lib/brandText.ts';
import { PLATFORM_LABELS } from '../../types/platform.ts';
import type { GameLibraryItem, LibraryFilter, LibrarySort } from '../../lib/libraryStatus.ts';
import type { CatalogGame } from '../../types/repository.ts';
import { fusionGameKey, type FusionCatalogEntry } from '../../types/fusionSource.ts';
import {
  buildUnifiedLibraryItems,
  countUnifiedLibraryItems,
  libraryPlatformLabel,
  type LibraryCollectionCounts,
  type UnifiedLibraryItem
} from '../../lib/unifiedLibrary.ts';

export interface CollectionTarget {
  filter: LibraryFilter;
  query: string;
  sort: LibrarySort;
  platform?: string | null;
}

interface HeroPanelProps {
  heroItem: UnifiedLibraryItem | null;
  busyAction: string | null;
  onPrimaryAction: (item: GameLibraryItem) => void;
  onOpenDetails: (game: CatalogGame) => void;
  onOpenFusion: (game: FusionCatalogEntry) => void;
  onLaunchFusion?: (game: FusionCatalogEntry) => void;
  onOpenSettings: () => void;
  onFocus: (focusId: string) => void;
}

export interface HomeRail {
  title: string;
  testId: string;
  zone: string;
  items: UnifiedLibraryItem[];
}

export function HeroPanel({
  heroItem,
  busyAction,
  onPrimaryAction,
  onOpenDetails,
  onOpenFusion,
  onLaunchFusion,
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

  if (heroItem.kind === 'fusion_source') {
    const progressVisible = heroItem.isDownloading || heroItem.isPaused || heroItem.hasError
      || (heroItem.progressPercent > 0 && heroItem.progressPercent < 100);
    const metaItems = [
      `${t.shell.hero.status}: ${displayStatusLabel(heroItem.statusLabel, t)}`,
      `${t.shell.hero.source}: ${displayProductText(heroItem.sourceName)}`,
      progressVisible ? `${t.shell.hero.progress}: ${heroItem.progressPercent.toFixed(0)}%` : null
    ].filter((item): item is string => Boolean(item));
    const gameKey = encodeURIComponent(fusionGameKey(heroItem.game));
    const platformText = libraryPlatformLabel(heroItem.platform) || 'PC';
    const primaryLabel = heroItem.readyToPlay ? t.actions.Play : t.actions.Details;

    return (
      <section className="rh-hero-panel" data-testid="home-hero">
        <div className="rh-hero-bg">
          <GameArt
            game={{
              id: heroItem.key,
              title: heroItem.title,
              platform: heroItem.platform ?? '',
              coverImageUrl: heroItem.game.coverUrl ?? undefined
            }}
            className="h-full w-full"
            hero
          />
        </div>
        <div className="rh-hero-scrim" />

        <div className="rh-hero-content">
          <div className="rh-hero-copy-stack">
            <div className="rh-hero-kicker">
              <span className="rh-platform-chip">{platformText}</span>
              <span>v{heroItem.game.version}</span>
              <span>/</span>
              <span>{displayProductText(heroItem.sourceName)}</span>
            </div>
            <h1 className="rh-hero-title">
              {displayProductText(heroItem.title)}
            </h1>
            <div className="rh-hero-actions">
              <button
                data-focus-id={`fusion-game:hero:${gameKey}`}
                data-focus-zone="hero"
                onFocus={() => onFocus(`fusion-game:hero:${gameKey}`)}
                onClick={() => (heroItem.readyToPlay && onLaunchFusion ? onLaunchFusion(heroItem.game) : onOpenFusion(heroItem.game))}
                disabled={busyAction !== null}
                className="rh-primary-action rh-focusable"
              >
                {heroItem.readyToPlay ? <Play className="h-4 w-4" /> : <MoreHorizontal className="h-4 w-4" />}
                {primaryLabel}
              </button>
              <button
                data-focus-id={`fusion-game:hero-details:${gameKey}`}
                data-focus-zone="hero"
                onFocus={() => onFocus(`fusion-game:hero-details:${gameKey}`)}
                onClick={() => onOpenFusion(heroItem.game)}
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

  const legacyHero = heroItem.item;
  const progressVisible = legacyHero.isDownloading || legacyHero.isPaused || legacyHero.hasError
    || (legacyHero.progressPercent > 0 && legacyHero.progressPercent < 100);
  const metaItems = [
    `${t.shell.hero.status}: ${displayStatusLabel(legacyHero.statusLabel, t)}`,
    `${t.shell.hero.source}: ${displayProductText(legacyHero.game.repositoryName)}`,
    progressVisible ? `${t.shell.hero.progress}: ${legacyHero.progressPercent.toFixed(0)}%` : null
  ].filter((item): item is string => Boolean(item));
  const setupHint = legacyHero.missingRequirements[0] ?? null;

  return (
    <section className="rh-hero-panel" data-testid="home-hero">
      <div className="rh-hero-bg">
        <GameArt game={legacyHero.game} className="h-full w-full" hero />
      </div>
      <div className="rh-hero-scrim" />

      <div className="rh-hero-content">
        <div className="rh-hero-copy-stack">
          <div className="rh-hero-kicker">
            <span className="rh-platform-chip">{legacyHero.game.platform}</span>
            <span>{legacyHero.game.metadata?.releaseYear ?? t.shell.hero.mvpDemo}</span>
            <span>/</span>
            <span>{legacyHero.game.metadata?.genres?.[0] ?? PLATFORM_LABELS[legacyHero.game.platform]}</span>
          </div>
          <h1 className="rh-hero-title">
            {displayProductText(legacyHero.game.title)}
          </h1>
          {legacyHero.game.description && (
            <p className="rh-hero-description">{displayProductText(legacyHero.game.description)}</p>
          )}
          {setupHint && (
            <div className="rh-hero-alert">
              <RotateCw className="h-3.5 w-3.5" />
              <span>{setupHint}</span>
            </div>
          )}
          <div className="rh-hero-actions">
            <button
              data-focus-id={`action:${encodeURIComponent(legacyHero.game.id)}`}
              data-focus-zone="hero"
              onFocus={() => onFocus(`action:${encodeURIComponent(legacyHero.game.id)}`)}
              onClick={() => onPrimaryAction(legacyHero)}
              disabled={busyAction !== null}
              className="rh-primary-action rh-focusable"
            >
              {legacyHero.primaryAction === 'play' ? <Play className="h-4 w-4" /> : legacyHero.primaryAction === 'download' || legacyHero.primaryAction === 'import' ? <Download className="h-4 w-4" /> : <RotateCw className="h-4 w-4" />}
              {displayActionLabel(legacyHero.primaryActionLabel, t)}
            </button>
            <button
              data-focus-id={`details:${encodeURIComponent(legacyHero.game.id)}`}
              data-focus-zone="hero"
              onFocus={() => onFocus(`details:${encodeURIComponent(legacyHero.game.id)}`)}
              onClick={() => onOpenDetails(legacyHero.game)}
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
  onOpenFusion,
  onFocus
}: {
  rails: HomeRail[];
  onPrimaryAction: (item: GameLibraryItem) => void;
  onOpenDetails: (game: CatalogGame) => void;
  onOpenFusion: (game: FusionCatalogEntry) => void;
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
          featured={rail.testId === 'home-rail-library'}
          onOpenDetails={onOpenDetails}
          onOpenFusion={onOpenFusion}
          onPrimaryAction={onPrimaryAction}
          onFocus={onFocus}
        />
      ))}
    </div>
  );
}

export function CollectionsPanel({
  items = [],
  counts,
  activeCollectionId = 'all',
  onOpenCollection,
  onFocus
}: {
  items?: GameLibraryItem[];
  counts?: LibraryCollectionCounts;
  activeCollectionId?: string | null;
  onOpenCollection: (target: CollectionTarget) => void;
  onFocus: (focusId: string) => void;
}) {
  const { t } = useI18n();
  const summary = counts ?? countUnifiedLibraryItems(buildUnifiedLibraryItems(items, []));
  const collectionCards: Array<{
    id: string;
    label: string;
    count: number;
    icon: LucideIcon;
  }> = [
    { id: 'all', label: t.shell.collections.all, count: summary.all, icon: FolderHeart },
    { id: 'ready', label: t.shell.collections.ready, count: summary.ready, icon: Play },
    { id: 'downloads', label: t.shell.collections.downloads, count: summary.downloading, icon: Download },
    { id: 'missing', label: t.shell.collections.missing, count: summary.missing, icon: RotateCw },
    ...summary.platforms.map(({ platform, count }) => ({
      id: `platform:${platform}`,
      label: libraryPlatformLabel(platform),
      count,
      icon: Gamepad2
    }))
  ];

  return (
    <section className="rh-collections-panel" data-testid="collections-panel">
      <PanelTitle icon={FolderHeart} title={t.shell.collections.title} />
      <div className="rh-collections-grid">
        {collectionCards.map((card) => {
          const Icon = card.icon;
          const focusId = `collection:${card.id}`;
          const active = activeCollectionId?.toLowerCase() === card.id.toLowerCase();
          return (
            <button
              key={card.label}
              data-testid="collection-card"
              data-collection-id={card.id}
              data-focus-id={focusId}
              data-focus-zone="collections"
              onFocus={() => onFocus(focusId)}
              onClick={() => onOpenCollection(collectionTargetForId(card.id))}
              aria-pressed={active}
              className={`rh-collection-card rh-focusable ${active ? 'rh-collection-card-active' : ''}`}
            >
              <Icon className="h-4 w-4" />
              <span>{card.label}</span>
              <span className="rh-collection-count">{t.shell.collections.count(card.count)}</span>
            </button>
          );
        })}
      </div>
    </section>
  );
}

export function collectionTargetForId(id: string): CollectionTarget {
  if (id.startsWith('platform:')) {
    return { filter: 'all', query: '', sort: 'title', platform: id.slice('platform:'.length) };
  }
  if (id === 'ready') return { filter: 'ready', query: '', sort: 'status' };
  if (id === 'downloads') return { filter: 'downloading', query: '', sort: 'status' };
  if (id === 'missing') return { filter: 'missing', query: '', sort: 'status' };
  return { filter: 'all', query: '', sort: 'title' };
}

export function collectionIdForTarget(target: CollectionTarget): string | null {
  if (target.platform) return `platform:${target.platform}`;
  if (target.filter === 'all') return 'all';
  if (target.filter === 'ready') return 'ready';
  if (target.filter === 'downloading') return 'downloads';
  if (target.filter === 'missing') return 'missing';
  return null;
}

function MiniRail({
  title,
  testId,
  items,
  zone,
  featured,
  onOpenDetails,
  onOpenFusion,
  onPrimaryAction,
  onFocus
}: {
  title: string;
  testId: string;
  items: UnifiedLibraryItem[];
  zone: string;
  featured: boolean;
  onOpenDetails: (game: CatalogGame) => void;
  onOpenFusion: (game: FusionCatalogEntry) => void;
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
    <div className={`rh-mini-rail ${featured ? 'rh-mini-rail-featured' : ''}`} data-testid={testId}>
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
          {items.map((entry) => entry.kind === 'fusion_source' ? (
            <FusionGamePoster
              key={`${zone}:${entry.key}`}
              item={entry}
              compact
              focusId={`fusion-game:${zone}:${encodeURIComponent(fusionGameKey(entry.game))}`}
              zone={zone}
              onOpen={onOpenFusion}
              onFocus={onFocus}
            />
          ) : (
            <GamePoster
              key={`${zone}:${entry.key}`}
              item={entry.item}
              compact
              focusId={`game:${zone}:${encodeURIComponent(entry.item.game.id)}`}
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
