import type { ReactNode } from 'react';
import { motion } from 'framer-motion';
import { Ban, Loader2, Pause, Play, RotateCw } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import { GameArt } from '@/components/shell/GamePoster';
import type { GameLibraryItem } from '@/lib/libraryStatus';
import type { CatalogGame, TorrentDownloadRecord } from '@/types/repository';
import { ACTIVE_DOWNLOAD_STATUSES, RESUMABLE_DOWNLOAD_STATUSES } from './constants';
import { ScreenHeader } from './shared';
import type { BusyAction } from './types';
import { downloadStatusHint, formatBytes, formatSpeed, summarizeDownloads } from './utils';

export function DownloadsScreen({
  downloads,
  itemsByGameId,
  busyAction,
  onOpenDetails,
  onPause,
  onResume,
  onCancel,
  onPlay,
  onFocus
}: {
  downloads: TorrentDownloadRecord[];
  itemsByGameId: Map<string, GameLibraryItem>;
  busyAction: BusyAction;
  onOpenDetails: (game: CatalogGame) => void;
  onPause: (gameId: string) => Promise<void>;
  onResume: (gameId: string) => Promise<void>;
  onCancel: (gameId: string) => Promise<void>;
  onPlay: (item: GameLibraryItem) => void;
  onFocus: (focusId: string) => void;
}) {
  const { t } = useI18n();
  const summary = summarizeDownloads(downloads);
  const description = summary.active > 0
    ? t.dashboard.downloads.activeDescription(downloads.length)
    : t.dashboard.downloads.idleDescription(downloads.length);

  return (
    <motion.section initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} className="rh-screen rh-panel">
      <div className="rh-downloads-center" data-testid="downloads-center">
        <ScreenHeader eyebrow={t.dashboard.downloads.eyebrow} title={t.dashboard.downloads.title} description={description} />
        <div className="rh-download-summary">
          <DownloadMetric label={t.dashboard.downloads.active} value={String(summary.active)} tone="active" />
          <DownloadMetric label={t.dashboard.downloads.paused} value={String(summary.paused)} tone="paused" />
          <DownloadMetric label={t.dashboard.downloads.errors} value={String(summary.errors)} tone="error" />
          <DownloadMetric label={t.dashboard.downloads.downloaded} value={formatBytes(summary.downloadedBytes)} tone="ready" />
        </div>
        <div className="rh-download-list">
          {downloads.length === 0 ? (
            <div className="rh-empty-compact">{t.dashboard.downloads.empty}</div>
          ) : downloads.map((download) => {
            const item = itemsByGameId.get(download.gameId) ?? null;
            return (
              <DownloadRow
                key={download.gameId}
                download={download}
                item={item}
                busyAction={busyAction}
                onOpenDetails={onOpenDetails}
                onPause={onPause}
                onResume={onResume}
                onCancel={onCancel}
                onPlay={onPlay}
                onFocus={onFocus}
              />
            );
          })}
        </div>
      </div>
    </motion.section>
  );
}

function DownloadRow({
  download,
  item,
  busyAction,
  onOpenDetails,
  onPause,
  onResume,
  onCancel,
  onPlay,
  onFocus
}: {
  download: TorrentDownloadRecord;
  item: GameLibraryItem | null;
  busyAction: BusyAction;
  onOpenDetails: (game: CatalogGame) => void;
  onPause: (gameId: string) => Promise<void>;
  onResume: (gameId: string) => Promise<void>;
  onCancel: (gameId: string) => Promise<void>;
  onPlay: (item: GameLibraryItem) => void;
  onFocus: (focusId: string) => void;
}) {
  const { t } = useI18n();
  const active = ACTIVE_DOWNLOAD_STATUSES.includes(download.status);
  const resumable = RESUMABLE_DOWNLOAD_STATUSES.includes(download.status);
  const cancellable = !['completed', 'cancelled', 'cancelling'].includes(download.status);
  const statusHint = downloadStatusHint(download, t);

  return (
    <article className="rh-download-row" data-testid="download-row">
      <button
        data-focus-id={`details:${encodeURIComponent(download.gameId)}`}
        data-focus-zone="downloads"
        onFocus={() => onFocus(`details:${encodeURIComponent(download.gameId)}`)}
        onClick={() => item && onOpenDetails(item.game)}
        className="rh-download-art rh-focusable"
      >
        {item ? <GameArt game={item.game} className="h-full w-full" /> : null}
      </button>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <div className="truncate text-sm font-black">{item?.game.title ?? download.gameId}</div>
          <span className="rounded border border-white/10 px-2 py-1 text-[10px] uppercase text-white/54">{t.gameDetails.downloadTitles[download.status]}</span>
        </div>
        <div className="mt-2 h-1.5 overflow-hidden rounded bg-black/42">
          <div className="h-full rounded bg-fusion-accent" style={{ width: `${download.status === 'completed' ? 100 : download.progressPercent}%` }} />
        </div>
        <div className="mt-2 flex flex-wrap gap-4 text-xs text-white/42">
          <span>{formatBytes(download.downloadedBytes)} / {formatBytes(download.totalBytes)}</span>
          <span>{formatSpeed(download.downloadSpeedBytesPerSec)}</span>
          <span>{download.peersCount} {t.common.peers}</span>
        </div>
        {download.saveDir && <div className="mt-2 truncate text-xs text-white/32">{download.saveDir}</div>}
        {statusHint && <div className="mt-2 text-xs text-white/42">{statusHint}</div>}
        {download.errorMessage && <div className="mt-2 text-xs text-red-100">{download.errorMessage}</div>}
      </div>
      <div className="flex flex-wrap justify-end gap-2">
        {active && download.status !== 'cancelling' && (
          <IconAction
            focusId={`download-action:pause:${encodeURIComponent(download.gameId)}`}
            onFocus={onFocus}
            busy={busyAction === `pause:${download.gameId}`}
            label={t.dashboard.downloads.pause}
            icon={<Pause className="h-3.5 w-3.5" />}
            onClick={() => onPause(download.gameId)}
          />
        )}
        {resumable && (
          <IconAction
            focusId={`download-action:${download.status === 'error' ? 'retry' : 'resume'}:${encodeURIComponent(download.gameId)}`}
            onFocus={onFocus}
            busy={busyAction === `resume:${download.gameId}`}
            label={download.status === 'error' ? t.dashboard.downloads.retry : t.dashboard.downloads.resume}
            icon={<RotateCw className="h-3.5 w-3.5" />}
            onClick={() => onResume(download.gameId)}
          />
        )}
        {item?.readyToPlay && download.status === 'completed' && (
          <IconAction
            focusId={`download-action:play:${encodeURIComponent(download.gameId)}`}
            onFocus={onFocus}
            busy={busyAction === `play:${download.gameId}`}
            label={t.dashboard.downloads.play}
            icon={<Play className="h-3.5 w-3.5" />}
            onClick={() => onPlay(item)}
          />
        )}
        {cancellable && (
          <IconAction
            focusId={`download-action:cancel:${encodeURIComponent(download.gameId)}`}
            onFocus={onFocus}
            busy={busyAction === `cancel:${download.gameId}`}
            label={t.dashboard.downloads.cancel}
            icon={<Ban className="h-3.5 w-3.5" />}
            onClick={() => onCancel(download.gameId)}
            danger
          />
        )}
      </div>
    </article>
  );
}

function IconAction({
  label,
  icon,
  busy,
  danger,
  focusId,
  onFocus,
  onClick
}: {
  label: string;
  icon: ReactNode;
  busy: boolean;
  danger?: boolean;
  focusId: string;
  onFocus: (focusId: string) => void;
  onClick: () => void;
}) {
  return (
    <button
      data-focus-id={focusId}
      data-focus-zone="download-actions"
      onFocus={() => onFocus(focusId)}
      onClick={onClick}
      className={`rh-mini-action rh-focusable ${danger ? 'rh-mini-action-danger' : ''}`}
    >
      {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : icon}
      {label}
    </button>
  );
}

function DownloadMetric({
  label,
  value,
  tone
}: {
  label: string;
  value: string;
  tone: 'active' | 'paused' | 'error' | 'ready';
}) {
  return (
    <div className={`rh-download-metric rh-download-metric-${tone}`}>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}
