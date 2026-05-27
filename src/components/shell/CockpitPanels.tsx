'use client';

import {
  Download,
  FolderHeart,
  Gamepad2,
  type LucideIcon,
  MoreHorizontal,
  Play,
  RotateCw,
  Star
} from 'lucide-react';
import { GameArt, GamePoster } from './GamePoster';
import { PLATFORM_LABELS } from '../../types/platform.ts';
import type { GameLibraryItem } from '../../lib/libraryStatus.ts';
import type { CatalogGame } from '../../types/repository.ts';

interface HeroPanelProps {
  heroItem: GameLibraryItem | null;
  continueItems: GameLibraryItem[];
  recentItems: GameLibraryItem[];
  busyAction: string | null;
  onPrimaryAction: (item: GameLibraryItem) => void;
  onOpenDetails: (game: CatalogGame) => void;
  onFocus: (focusId: string) => void;
}

export function HeroPanel({
  heroItem,
  continueItems,
  recentItems,
  busyAction,
  onPrimaryAction,
  onOpenDetails,
  onFocus
}: HeroPanelProps) {
  if (!heroItem) {
    return (
      <section className="rh-hero-panel grid place-items-center">
        <div className="text-center">
          <Gamepad2 className="mx-auto h-8 w-8 text-hydra-accent" />
          <div className="mt-4 text-2xl font-black uppercase">Library Empty</div>
          <p className="mt-2 text-sm text-white/46">Connect a repository to populate the cockpit.</p>
        </div>
      </section>
    );
  }

  return (
    <section className="rh-hero-panel">
      <div className="rh-hero-bg">
        <GameArt game={heroItem.game} className="h-full w-full" hero />
      </div>
      <div className="rh-hero-scrim" />

      <div className="rh-hero-content">
        <div className="rh-hero-kicker">
          <span className="rounded border border-white/18 px-2 py-1">{heroItem.game.platform}</span>
          <span>{PLATFORM_LABELS[heroItem.game.platform]}</span>
          <span>/</span>
          <span>{heroItem.game.repositoryName}</span>
        </div>
        <h1 className="rh-hero-title">
          {heroItem.game.title}
        </h1>
        {heroItem.game.description && (
          <p className="rh-hero-description">{heroItem.game.description}</p>
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
            {heroItem.primaryAction === 'play' ? <Play className="h-4 w-4" /> : heroItem.primaryAction === 'download' ? <Download className="h-4 w-4" /> : <RotateCw className="h-4 w-4" />}
            {heroItem.primaryActionLabel}
          </button>
          <button
            data-focus-id={`details:${encodeURIComponent(heroItem.game.id)}`}
            data-focus-zone="hero"
            onFocus={() => onFocus(`details:${encodeURIComponent(heroItem.game.id)}`)}
            onClick={() => onOpenDetails(heroItem.game)}
            className="rh-square-action rh-focusable"
            title="Details"
          >
            <MoreHorizontal className="h-4 w-4" />
          </button>
        </div>
        <div className="rh-hero-meta">
          <span>Status: {heroItem.statusLabel}</span>
          <span>Progress: {heroItem.progressPercent.toFixed(0)}%</span>
          <span>Repository: {heroItem.game.repositoryName}</span>
        </div>

        <MiniRail
          title="Continue Playing"
          items={continueItems.length > 0 ? continueItems : recentItems.slice(0, 6)}
          zone="continue"
          onOpenDetails={onOpenDetails}
          onPrimaryAction={onPrimaryAction}
          onFocus={onFocus}
        />
        <MiniRail
          title="Recently Added"
          items={recentItems.slice(0, 8)}
          zone="recent"
          onOpenDetails={onOpenDetails}
          onPrimaryAction={onPrimaryAction}
          onFocus={onFocus}
        />
      </div>
    </section>
  );
}

export function CollectionsPanel({
  items,
  onOpenLibrary,
  onFocus
}: {
  items: GameLibraryItem[];
  onOpenLibrary: () => void;
  onFocus: (focusId: string) => void;
}) {
  const byPlatform = new Map<string, number>();
  items.forEach((item) => byPlatform.set(item.game.platform, (byPlatform.get(item.game.platform) ?? 0) + 1));
  const collectionCards = [
    { label: 'All Games', count: items.length, icon: FolderHeart, active: true },
    { label: 'Favorites', count: items.filter((item) => item.readyToPlay).length, icon: Star },
    ...Array.from(byPlatform.entries()).slice(0, 4).map(([platform, count]) => ({
      label: PLATFORM_LABELS[platform as keyof typeof PLATFORM_LABELS] ?? platform,
      count,
      icon: Gamepad2
    }))
  ];

  return (
    <section className="rh-collections-panel">
      <PanelTitle icon={FolderHeart} title="Collections" />
      <div className="rh-collections-grid">
        {collectionCards.slice(0, 6).map((card, index) => {
          const Icon = card.icon;
          const focusId = `library:open:${index}`;
          return (
            <button
              key={card.label}
              data-focus-id={focusId}
              data-focus-zone="collections"
              onFocus={() => onFocus(focusId)}
              onClick={onOpenLibrary}
              className={`rh-collection-card rh-focusable ${card.active ? 'rh-collection-card-active' : ''}`}
            >
              <Icon className="h-4 w-4" />
              <span className="mt-3 block text-[11px] font-black uppercase">{card.label}</span>
              <span className="mt-1 block text-[10px] text-white/48">{card.count} games</span>
            </button>
          );
        })}
      </div>
    </section>
  );
}

function MiniRail({
  title,
  items,
  zone,
  onOpenDetails,
  onPrimaryAction,
  onFocus
}: {
  title: string;
  items: GameLibraryItem[];
  zone: string;
  onOpenDetails: (game: CatalogGame) => void;
  onPrimaryAction: (item: GameLibraryItem) => void;
  onFocus: (focusId: string) => void;
}) {
  if (items.length === 0) return null;

  return (
    <div className="rh-mini-rail">
      <div className="rh-mini-rail-title">{title}</div>
      <div className="rh-mini-rail-track">
        {items.slice(0, 8).map((item) => (
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
  );
}

function PanelTitle({ icon: Icon, title }: { icon: LucideIcon; title: string }) {
  return (
    <div className="flex items-center gap-2 text-sm font-black uppercase tracking-[0.08em]">
      <Icon className="h-4 w-4 text-white/52" />
      {title}
    </div>
  );
}
