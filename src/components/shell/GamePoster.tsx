'use client';

import { useEffect, useState } from 'react';
import { motion } from 'framer-motion';
import { Download, Play, RotateCw, ShieldAlert } from 'lucide-react';
import { useI18n } from '../I18nProvider';
import { displayProductText } from '../../lib/brandText.ts';
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
  const { t } = useI18n();
  const ActionIcon = actionIconFor(item.primaryAction);
  const progressVisible = item.isDownloading || item.isPaused || item.hasError
    || (item.progressPercent > 0 && item.progressPercent < 100);

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
        className="rh-focusable rh-card-button"
      >
        <GameArt game={item.game} className={compact ? 'rh-compact-art' : 'rh-card-art'} />
        <div className={`rh-card-status rh-card-status-${item.statusTone}`}>
          {compact ? shortStatusLabel(item.statusLabel, t) : displayStatusLabel(item.statusLabel, t)}
        </div>
        {progressVisible && (
          <div className="rh-card-progress">
            <div style={{ width: `${item.readyToPlay ? 100 : item.progressPercent}%` }} />
          </div>
        )}
        <div className={compact ? 'rh-compact-card-copy' : 'rh-card-copy'}>
          <div className={`${compact ? 'text-[11px]' : 'text-sm'} line-clamp-2 font-semibold leading-tight text-white`}>{displayProductText(item.game.title)}</div>
          <div className="mt-1 flex items-center justify-between gap-2 text-[11px] text-white/46">
            <span>{item.game.platform}</span>
            {item.missingRequirements.length > 0 && <ShieldAlert className="h-3 w-3 text-amber-200" />}
          </div>
        </div>
      </button>
      {onAction && (
        <button
          onClick={() => onAction(item)}
          aria-label={displayActionLabel(item.primaryActionLabel, t)}
          className="rh-card-action absolute right-2 grid h-7 w-7 place-items-center rounded-md bg-black/72 text-white opacity-0 backdrop-blur transition hover:bg-fusion-accent group-hover:opacity-100"
          title={displayActionLabel(item.primaryActionLabel, t)}
        >
          <ActionIcon className="h-3.5 w-3.5" />
        </button>
      )}
    </motion.article>
  );
}

function actionIconFor(action: PrimaryGameAction) {
  if (action === 'download' || action === 'import') return Download;
  if (action === 'resume' || action === 'retry') return RotateCw;
  if (action === 'details') return ShieldAlert;
  return Play;
}

function shortStatusLabel(label: string, t: ReturnType<typeof useI18n>['t']) {
  return t.shortStatusLabels[label as keyof typeof t.shortStatusLabels] ?? displayStatusLabel(label, t);
}

function displayActionLabel(label: string, t: ReturnType<typeof useI18n>['t']) {
  return t.actions[label as keyof typeof t.actions] ?? label;
}

function displayStatusLabel(label: string, t: ReturnType<typeof useI18n>['t']) {
  return t.statusLabels[label as keyof typeof t.statusLabels] ?? label;
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
  const imageUrl = hero
    ? game.artwork?.hero ?? game.coverImageUrl ?? game.artwork?.cover
    : game.artwork?.cover ?? game.coverImageUrl;
  const [imageFailed, setImageFailed] = useState(false);

  useEffect(() => {
    setImageFailed(false);
  }, [imageUrl]);

  return (
    <div className={`relative overflow-hidden bg-fusion-raised ${className}`}>
      <div className="absolute inset-0" style={hero ? generated.heroStyle : generated.posterStyle}>
        <div className="absolute inset-0 bg-[linear-gradient(115deg,rgba(255,255,255,0.10),transparent_23%),linear-gradient(0deg,rgba(0,0,0,0.34),transparent_44%)]" />
        {hero ? (
          <div className="rh-hero-figure">
            <span>{generated.initials}</span>
            <small>{PLATFORM_LABELS[game.platform]}</small>
          </div>
        ) : (
          <>
            <div className="absolute inset-x-4 top-4 flex items-center justify-between text-[10px] font-semibold text-white/56">
              <span>{game.platform}</span>
              <span>fusion</span>
            </div>
            <div className="absolute inset-0 grid place-items-center">
              <div className="text-center">
                <div className="text-3xl font-black tracking-normal text-white/90">{generated.initials}</div>
                <div className="mt-2 max-w-[120px] text-[10px] font-bold uppercase leading-tight text-white/52">{PLATFORM_LABELS[game.platform]}</div>
              </div>
            </div>
          </>
        )}
      </div>
      {imageUrl && !imageFailed && (
        // eslint-disable-next-line @next/next/no-img-element
        <img src={imageUrl} alt="" loading="lazy" decoding="async" className="absolute inset-0 h-full w-full object-cover" onError={() => setImageFailed(true)} />
      )}
    </div>
  );
}
