'use client';

import { motion } from 'framer-motion';
import { Download, Play, RotateCw, ShieldAlert } from 'lucide-react';
import { createGameArt } from '../../lib/gameArt.ts';
import { PLATFORM_LABELS } from '../../types/platform.ts';
import type { GameLibraryItem, PrimaryGameAction } from '../../lib/libraryStatus.ts';
import type { CatalogGame } from '../../types/repository.ts';

interface GamePosterProps {
  item: GameLibraryItem;
  focusId: string;
  zone: string;
  compact?: boolean;
  selected?: boolean;
  onOpen: (game: CatalogGame) => void;
  onAction?: (item: GameLibraryItem) => void;
  onFocus?: (focusId: string) => void;
}

export function GamePoster({
  item,
  focusId,
  zone,
  compact,
  selected,
  onOpen,
  onAction,
  onFocus
}: GamePosterProps) {
  const ActionIcon = actionIconFor(item.primaryAction);
  const progressVisible = item.installed || item.isDownloading || item.isPaused || item.hasError || item.progressPercent > 0;

  return (
    <motion.article
      layoutId={`game-card-${focusId}`}
      whileHover={{ scale: compact ? 1.035 : 1.025 }}
      className={`rh-game-card group ${selected ? 'rh-game-card-selected' : ''} ${compact ? 'rh-game-card-compact' : ''}`}
    >
      <button
        data-focus-id={focusId}
        data-focus-zone={zone}
        onFocus={() => onFocus?.(focusId)}
        onClick={() => onOpen(item.game)}
        className="rh-focusable relative block w-full overflow-hidden rounded-md text-left"
      >
        <GameArt game={item.game} className={compact ? 'h-[94px]' : 'aspect-[2/3]'} />
        <div className={`rh-card-status rh-card-status-${item.statusTone}`}>{compact ? shortStatusLabel(item.statusLabel) : item.statusLabel}</div>
        {progressVisible && (
          <div className="rh-card-progress">
            <div style={{ width: `${item.readyToPlay ? 100 : item.progressPercent}%` }} />
          </div>
        )}
        <div className={compact ? 'min-h-[40px] px-2 py-1.5' : 'min-h-[48px] px-2.5 py-2'}>
          <div className={`${compact ? 'text-[10px]' : 'text-[11px]'} line-clamp-2 font-black uppercase leading-tight text-white`}>{item.game.title}</div>
          <div className="mt-1 flex items-center justify-between gap-2 text-[10px] uppercase text-white/46">
            <span>{item.game.platform}</span>
            {item.missingRequirements.length > 0 && <ShieldAlert className="h-3 w-3 text-amber-200" />}
          </div>
        </div>
      </button>
      {onAction && (
        <button
          onClick={() => onAction(item)}
          aria-label={item.primaryActionLabel}
          className={`absolute right-2 grid h-7 w-7 place-items-center rounded-md bg-black/72 text-white opacity-0 backdrop-blur transition hover:bg-hydra-accent group-hover:opacity-100 ${compact ? 'bottom-[44px]' : 'bottom-[54px]'}`}
          title={item.primaryActionLabel}
        >
          <ActionIcon className="h-3.5 w-3.5" />
        </button>
      )}
    </motion.article>
  );
}

function actionIconFor(action: PrimaryGameAction) {
  if (action === 'download') return Download;
  if (action === 'resume' || action === 'retry') return RotateCw;
  if (action === 'details') return ShieldAlert;
  return Play;
}

function shortStatusLabel(label: string) {
  if (label === 'Missing Requirements') return 'Missing';
  if (label === 'Ready to Play') return 'Ready';
  if (label === 'Not Installed') return 'New';
  if (label === 'Resolving Magnet') return 'Resolving';
  return label;
}

export function GameArt({
  game,
  className,
  hero
}: {
  game: CatalogGame;
  className: string;
  hero?: boolean;
}) {
  const generated = createGameArt(game);

  return (
    <div className={`relative overflow-hidden bg-[#111218] ${className}`}>
      {game.coverImageUrl ? (
        // eslint-disable-next-line @next/next/no-img-element
        <img src={game.coverImageUrl} alt="" className="h-full w-full object-cover" />
      ) : (
        <div className="absolute inset-0" style={hero ? generated.heroStyle : generated.posterStyle}>
          <div className="absolute inset-0 bg-[linear-gradient(135deg,rgba(255,255,255,0.08),transparent_28%),radial-gradient(circle_at_25%_80%,rgba(255,255,255,0.08),transparent_24%)]" />
          <div className="absolute inset-x-4 top-4 flex items-center justify-between text-[10px] font-bold uppercase tracking-[0.14em] text-white/56">
            <span>{game.platform}</span>
            <span>RH</span>
          </div>
          <div className="absolute inset-0 grid place-items-center">
            <div className="text-center">
              <div className={`${hero ? 'text-7xl' : 'text-3xl'} font-black tracking-normal text-white/90`}>{generated.initials}</div>
              {!hero && <div className="mt-2 max-w-[120px] text-[10px] font-bold uppercase leading-tight text-white/52">{PLATFORM_LABELS[game.platform]}</div>}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
