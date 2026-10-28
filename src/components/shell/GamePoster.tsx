'use client';

import { useEffect, useState, type ReactNode } from 'react';
import { motion } from 'framer-motion';
import { Download, Play, RotateCw, ShieldAlert } from 'lucide-react';
import { useI18n } from '../I18nProvider';
import { displayProductText } from '../../lib/brandText.ts';
import { createGameArt, selectGameArtImageUrl, resolveArtworkUrl } from '../../lib/gameArt.ts';
import type { GameLibraryItem, PrimaryGameAction } from '../../lib/libraryStatus.ts';
import type { CatalogGame } from '../../types/repository.ts';
import { libraryPlatformLabel, type UnifiedLibraryItem } from '../../lib/unifiedLibrary.ts';
import type { StatusTone } from '../../lib/libraryStatus.ts';
import type { FusionCatalogEntry } from '../../types/fusionSource.ts';

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

  return <PosterCard
    game={item.game}
    focusId={focusId}
    zone={zone}
    compact={compact}
    selected={selected}
    statusLabel={compact ? shortStatusLabel(item.statusLabel, t) : displayStatusLabel(item.statusLabel, t)}
    statusTone={item.statusTone}
    progressVisible={progressVisible}
    progressPercent={item.readyToPlay ? 100 : item.progressPercent}
    needsSetup={item.missingRequirements.length > 0}
    onOpen={() => onOpen(item.game)}
    onFocus={onFocus}
    action={onAction ? <button
      onClick={() => onAction(item)}
      aria-label={displayActionLabel(item.primaryActionLabel, t)}
      className="rh-card-action absolute right-2 grid h-7 w-7 place-items-center rounded-md bg-black/72 text-white opacity-0 backdrop-blur transition hover:bg-fusion-accent group-hover:opacity-100"
      title={displayActionLabel(item.primaryActionLabel, t)}
    ><ActionIcon className="h-3.5 w-3.5" /></button> : undefined}
  />;
}

export function FusionGamePoster({ item, onOpen, onFocus }: {
  item: Extract<UnifiedLibraryItem, { kind: 'fusion_source' }>;
  onOpen: (game: FusionCatalogEntry) => void;
  onFocus: (focusId: string) => void;
}) {
  const { t } = useI18n();
  return <PosterCard
    game={{ id: item.key, title: item.title, platform: item.platform ?? '', coverImageUrl: item.game.coverUrl ?? undefined }}
    focusId={item.focusId}
    zone="library"
    statusLabel={displayStatusLabel(item.statusLabel, t)}
    statusTone={item.statusTone}
    progressVisible={item.isDownloading}
    progressPercent={item.progressPercent}
    needsSetup={item.needsSetup}
    onOpen={() => onOpen(item.game)}
    onFocus={onFocus}
    testId={`fusion-catalog-${item.game.gameId}`}
  />;
}

type GameArtInput = Pick<CatalogGame, 'id' | 'title' | 'artwork' | 'coverImageUrl'> & { platform: string };

function PosterCard({ game, focusId, zone, compact, selected, statusLabel, statusTone, progressVisible, progressPercent,
  needsSetup, onOpen, onFocus, action, testId }: {
  game: GameArtInput;
  focusId: string;
  zone: string;
  compact?: boolean;
  selected?: boolean;
  statusLabel: string;
  statusTone: StatusTone;
  progressVisible: boolean;
  progressPercent: number;
  needsSetup: boolean;
  onOpen: () => void;
  onFocus?: (focusId: string) => void;
  action?: ReactNode;
  testId?: string;
}) {
  const platformLabel = libraryPlatformLabel(game.platform);
  return (
    <motion.article
      whileHover={{ scale: compact ? 1.035 : 1.025 }}
      className={`rh-game-card group ${selected ? 'rh-game-card-selected' : ''} ${compact ? 'rh-game-card-compact' : ''}`}
    >
      <button
        data-focus-id={focusId}
        data-focus-zone={zone}
        onFocus={() => onFocus?.(focusId)}
        onClick={onOpen}
        className="rh-focusable rh-card-button"
        data-testid={testId}
      >
        <GameArt game={game} className={compact ? 'rh-compact-art' : 'rh-card-art'} />
        <div className={`rh-card-status rh-card-status-${statusTone}`} title={statusLabel}>
          {statusLabel}
        </div>
        {progressVisible && (
          <div className="rh-card-progress">
            <div style={{ width: `${progressPercent}%` }} />
          </div>
        )}
        <div className={compact ? 'rh-compact-card-copy' : 'rh-card-copy'}>
          <div className={`${compact ? 'text-[11px]' : 'text-sm'} line-clamp-2 font-semibold leading-tight text-white`}>{displayProductText(game.title)}</div>
          <div className="mt-1 flex items-center justify-between gap-2 text-[11px] text-white/46">
            {platformLabel && <span className={compact ? undefined : 'rounded border border-fusion-line px-1.5 text-[10px] leading-[14px]'}>{platformLabel}</span>}
            {needsSetup && <ShieldAlert className="h-3 w-3 text-amber-200" />}
          </div>
        </div>
      </button>
      {action}
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
  game: GameArtInput;
  className: string;
  hero?: boolean;
}) {
  const generated = createGameArt(game);
  const imageUrl = selectGameArtImageUrl(game, hero ? 'hero' : 'poster');
  const [resolvedUrl, setResolvedUrl] = useState<string | null>(imageUrl);
  const [imageFailed, setImageFailed] = useState(false);

  useEffect(() => {
    setImageFailed(false);
    let active = true;
    setResolvedUrl(imageUrl);
    if (imageUrl) {
      resolveArtworkUrl(imageUrl).then((cached) => {
        if (active && cached) {
          setResolvedUrl(cached);
        }
      });
    }
    return () => {
      active = false;
    };
  }, [imageUrl]);

  return (
    <div className={`relative overflow-hidden bg-fusion-raised ${className}`}>
      <div className="absolute inset-0" style={hero ? generated.heroStyle : generated.posterStyle}>
        <div className="absolute inset-0 bg-[linear-gradient(115deg,rgba(255,255,255,0.10),transparent_23%),linear-gradient(0deg,rgba(0,0,0,0.34),transparent_44%)]" />
        {hero ? (
          <div className="rh-hero-figure">
            <span>{generated.initials}</span>
            <small>{libraryPlatformLabel(game.platform)}</small>
          </div>
        ) : (
          <>
            <div className="absolute inset-0 grid place-items-center">
              <div className="text-center">
                <div className="text-3xl font-black tracking-normal text-white/90">{generated.initials}</div>
                <div className="mt-2 max-w-[120px] text-[10px] font-bold uppercase leading-tight text-white/52">{libraryPlatformLabel(game.platform)}</div>
              </div>
            </div>
          </>
        )}
      </div>
      {resolvedUrl && !imageFailed && (
        // eslint-disable-next-line @next/next/no-img-element
        <img src={resolvedUrl} alt="" loading="lazy" decoding="async" className="absolute inset-0 h-full w-full object-cover" onError={() => setImageFailed(true)} />
      )}
    </div>
  );
}
