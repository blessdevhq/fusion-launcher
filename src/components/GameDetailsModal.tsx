'use client';

import { useEffect, useMemo, useState } from 'react';
import {
  Ban,
  Download as DownloadIcon,
  Loader2,
  Pause,
  Play,
  RotateCw,
  ShieldAlert,
  ShieldCheck,
  X
} from 'lucide-react';
import { LaunchErrorModal } from '@/components/LaunchErrorModal';
import { useDownloadState } from '@/hooks/useDownloadState';
import { api } from '@/lib/api';
import { normalizeLaunchFailure } from '@/lib/launchErrors';
import { defaultSaveDirForGame } from '@/lib/paths';
import type { AppSettings } from '@/lib/settings';
import type {
  CatalogGame,
  LaunchFailure,
  RequirementsReport,
  TorrentDownloadStatus
} from '@/types/repository';

const DOWNLOAD_TITLES: Record<TorrentDownloadStatus, string> = {
  resolving: 'Resolving magnet',
  downloading: 'Downloading',
  paused: 'Paused',
  interrupted: 'Interrupted',
  completed: 'Downloaded',
  cancelling: 'Cancelling',
  cancelled: 'Cancelled',
  error: 'Download error'
};

interface GameDetailsModalProps {
  game: CatalogGame;
  settings: AppSettings;
  onClose: () => void;
  onOpenSettings: () => void;
  onRefresh: () => Promise<void>;
}

export function GameDetailsModal({
  game,
  settings,
  onClose,
  onOpenSettings,
  onRefresh
}: GameDetailsModalProps) {
  const [requirements, setRequirements] = useState<RequirementsReport | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [defaultSaveDir, setDefaultSaveDir] = useState<string | null>(null);
  const [launchFailure, setLaunchFailure] = useState<LaunchFailure | null>(null);
  const download = useDownloadState(game.id);
  void settings;

  const downloadableSource = useMemo(
    () => game.downloads.find((source) => source.kind === 'magnet' || source.kind === 'http') ?? null,
    [game.downloads]
  );

  const loadRequirements = async () => {
    try {
      setRequirements(await api.checkRequirements(game.id));
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    }
  };

  useEffect(() => {
    loadRequirements();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [game.id]);

  useEffect(() => {
    let cancelled = false;
    setMessage(null);
    setDefaultSaveDir(null);

    const resolveSaveDir = async () => {
      try {
        const resolvedSaveDir = await defaultSaveDirForGame(game.id);
        if (!cancelled) {
          setDefaultSaveDir(resolvedSaveDir);
        }
      } catch (error) {
        if (!cancelled) {
          setMessage(`Failed to resolve download folder: ${error}`);
        }
      }
    };

    void resolveSaveDir();

    return () => {
      cancelled = true;
    };
  }, [game.id]);

  const run = async (label: string, action: () => Promise<unknown>) => {
    setBusy(label);
    setMessage(null);
    try {
      await action();
      await loadRequirements();
      await onRefresh();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const runDownloadAction = async (label: string, action: () => Promise<unknown>) => {
    setBusy(label);
    setMessage(null);
    try {
      await action();
      await download.refresh();
      await loadRequirements();
      await onRefresh();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const handleDownload = async () => {
    if (!downloadableSource) {
      setMessage('No automatic download source is available for this game.');
      return;
    }

    await runDownloadAction('download', () => api.startGameDownload(game.id));
  };

  const handlePlay = async () => {
    const saveDir = download.saveDir ?? defaultSaveDir;
    if (!saveDir) {
      setMessage('Downloaded game folder is not ready.');
      return;
    }

    setBusy('launch');
    setMessage(null);
    setLaunchFailure(null);
    try {
      await api.launchGame(game.id);
      setMessage('Launch request sent.');
    } catch (error) {
      setLaunchFailure(normalizeLaunchFailure(error, game));
    } finally {
      setBusy(null);
    }
  };

  const saveDir = download.saveDir ?? defaultSaveDir;
  const status = download.status;
  const progressPercent = (download.canPlay ? 100 : download.progressPercent).toFixed(1);
  const statusMessage = message ?? download.errorMessage;
  const showDownloadPanel = download.isLoading || (status !== null && status !== 'cancelled');
  const canDownload = !download.isLoading && (status === null || status === 'cancelled');
  const canPlay = download.canPlay && Boolean(saveDir) && Boolean(requirements?.ready) && busy === null;
  const downloadTitle = download.isLoading
    ? 'Checking download'
    : status
      ? DOWNLOAD_TITLES[status]
      : 'Download';

  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/72 px-5">
      <section className="max-h-[88vh] w-full max-w-3xl overflow-y-auto rounded-lg border border-white/12 bg-[#141417] shadow-2xl">
        <header className="flex items-start gap-4 border-b border-white/10 p-5">
          <div className="min-w-0 flex-1">
            <h2 className="text-2xl font-black">{game.title}</h2>
            <div className="mt-1 text-sm text-white/46">{game.platform} / {game.repositoryName}</div>
          </div>
          <button
            onClick={onClose}
            className="grid h-9 w-9 place-items-center rounded-md border border-white/10 text-white/60 transition hover:text-white"
            title="Close"
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        <div className="grid grid-cols-[210px_1fr] gap-5 p-5">
          <div className="overflow-hidden rounded-lg border border-white/10 bg-[#1a1a20]">
            <div className="aspect-[3/4]">
              {game.coverImageUrl ? (
                // eslint-disable-next-line @next/next/no-img-element
                <img src={game.coverImageUrl} alt="" className="h-full w-full object-cover" />
              ) : (
                <div className="grid h-full place-items-center px-4 text-center text-sm font-bold text-white/32">
                  {game.title}
                </div>
              )}
            </div>
          </div>

          <div className="min-w-0">
            {game.description && (
              <p className="mb-5 text-sm leading-6 text-white/66">{game.description}</p>
            )}

            <div className="mb-5 space-y-2">
              {(requirements?.requirements || []).map((item) => (
                <div key={item.asset.id} className="flex items-center justify-between gap-3 rounded-md bg-black/22 px-3 py-2 text-sm">
                  <div className="min-w-0">
                    <div className="truncate font-semibold">{item.asset.displayName}</div>
                    <div className="mt-1 text-xs text-white/38">
                      {item.asset.assetKind} / {requirementStatusLabel(item.status)}
                    </div>
                  </div>
                  <div className="flex items-center gap-2">
                    {item.status === 'ready' ? (
                      <ShieldCheck className="h-4 w-4 text-hydra-green" />
                    ) : (
                      <ShieldAlert className="h-4 w-4 text-amber-200" />
                    )}
                    <button
                      onClick={() => run(`asset:${item.asset.id}`, () => (
                        item.status === 'corrupt' || item.status === 'error'
                          ? api.redownloadAsset(item.asset.id)
                          : api.downloadAsset(item.asset.id)
                      ))}
                      disabled={busy !== null || item.status === 'ready'}
                      className="h-8 rounded-md border border-white/10 px-3 text-xs font-semibold text-white/72 transition hover:bg-white/10 disabled:opacity-40"
                    >
                      {busy === `asset:${item.asset.id}` ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : item.status === 'corrupt' || item.status === 'error' ? 'Retry' : 'Download'}
                    </button>
                    {item.asset.executable && item.downloaded && !item.trusted && (
                      <button
                        onClick={() => run(`trust:${item.asset.id}`, () => api.trustExecutable(item.asset.id))}
                        disabled={busy !== null}
                        className="h-8 rounded-md bg-hydra-accent px-3 text-xs font-bold text-white transition hover:bg-violet-500 disabled:opacity-40"
                      >
                        Trust
                      </button>
                    )}
                  </div>
                </div>
              ))}
            </div>

            {statusMessage && (
              <div className="mb-4 rounded-md border border-amber-300/24 bg-amber-300/10 px-3 py-2 text-sm text-amber-100">
                {statusMessage}
              </div>
            )}

            <div className="space-y-3">
              {showDownloadPanel ? (
                <div className="rounded-md border border-white/10 bg-white/[0.06] px-4 py-3">
                  <div className="flex items-center justify-between gap-3 text-sm">
                    <span className="font-semibold text-white/80">{downloadTitle}</span>
                    <span className="text-xs text-white/50">{progressPercent}%</span>
                  </div>
                  <div className="mt-3 h-2 overflow-hidden rounded bg-black/35">
                    <div
                      className={`h-full rounded transition-[width] duration-500 ${download.hasError ? 'bg-red-400' : 'bg-hydra-green'}`}
                      style={{ width: `${progressPercent}%` }}
                    />
                  </div>
                  <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-white/45">
                    <span>{formatBytes(download.downloadedBytes)} / {formatBytes(download.totalBytes)}</span>
                    <span>{formatSpeed(download.downloadSpeedBytesPerSec)}</span>
                    <span>{download.peersCount} peers</span>
                  </div>
                  <div className="mt-3 flex flex-wrap gap-2">
                    {status === 'downloading' && (
                      <button
                        onClick={() => runDownloadAction('pause', download.pause)}
                        disabled={busy !== null}
                        className="inline-flex h-8 items-center gap-2 rounded-md border border-white/10 px-3 text-xs font-semibold text-white/72 transition hover:bg-white/10 disabled:opacity-40"
                      >
                        {busy === 'pause' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Pause className="h-3.5 w-3.5" />}
                        Pause
                      </button>
                    )}
                    {(status === 'paused' || status === 'interrupted' || status === 'error') && (
                      <button
                        onClick={() => runDownloadAction('resume', download.resume)}
                        disabled={busy !== null}
                        className="inline-flex h-8 items-center gap-2 rounded-md border border-white/10 px-3 text-xs font-semibold text-white/72 transition hover:bg-white/10 disabled:opacity-40"
                      >
                        {busy === 'resume' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RotateCw className="h-3.5 w-3.5" />}
                        {status === 'error' ? 'Retry' : 'Resume'}
                      </button>
                    )}
                    {(status === 'resolving' || status === 'downloading' || status === 'paused' || status === 'interrupted' || status === 'error') && (
                      <button
                        onClick={() => runDownloadAction('cancel', download.cancel)}
                        disabled={busy !== null}
                        className="inline-flex h-8 items-center gap-2 rounded-md border border-red-300/20 px-3 text-xs font-semibold text-red-100/80 transition hover:bg-red-300/10 disabled:opacity-40"
                      >
                        {busy === 'cancel' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Ban className="h-3.5 w-3.5" />}
                        Cancel
                      </button>
                    )}
                  </div>
                </div>
              ) : (
                <button
                  onClick={handleDownload}
                  disabled={!canDownload || !downloadableSource || busy !== null}
                  className="inline-flex h-10 items-center gap-2 rounded-md border border-white/10 bg-white/[0.06] px-4 text-sm font-semibold text-white/76 transition hover:bg-white/12 disabled:opacity-40"
                >
                  {busy === 'download' ? <Loader2 className="h-4 w-4 animate-spin" /> : <DownloadIcon className="h-4 w-4" />}
                  {downloadableSource ? 'Download' : 'Manual Source'}
                </button>
              )}

              <button
                onClick={handlePlay}
                disabled={!canPlay}
                className="inline-flex h-10 items-center gap-2 rounded-md bg-hydra-accent px-4 text-sm font-bold text-white shadow-glow transition hover:bg-violet-500 disabled:opacity-40"
              >
                {busy === 'launch' ? <Loader2 className="h-4 w-4 animate-spin" /> : <Play className="h-4 w-4" />}
                Play
              </button>
              {(download.canPlay || status === 'completed') && (
                <button
                  onClick={() => runDownloadAction('open-folder', () => api.openGameFolder(game.id))}
                  disabled={busy !== null}
                  className="ml-2 inline-flex h-10 items-center gap-2 rounded-md border border-white/10 px-4 text-sm font-semibold text-white/72 transition hover:bg-white/10 disabled:opacity-40"
                >
                  Open game folder
                </button>
              )}
              {(download.canPlay || status === 'completed' || status === 'cancelled') && (
                <button
                  onClick={() => {
                    if (window.confirm(`Remove downloaded files for ${game.title}?`)) {
                      void runDownloadAction('remove', () => api.removeGame(game.id, true));
                    }
                  }}
                  disabled={busy !== null}
                  className="ml-2 inline-flex h-10 items-center gap-2 rounded-md border border-red-300/20 px-4 text-sm font-semibold text-red-100/80 transition hover:bg-red-300/10 disabled:opacity-40"
                >
                  Remove files
                </button>
              )}
            </div>
          </div>
        </div>
      </section>

      {launchFailure && (
        <LaunchErrorModal
          failure={launchFailure}
          onClose={() => setLaunchFailure(null)}
          onOpenSettings={() => {
            setLaunchFailure(null);
            onOpenSettings();
          }}
          onOpenDetails={() => setLaunchFailure(null)}
          onRetryDownload={() => {
            setLaunchFailure(null);
            void handleDownload();
          }}
        />
      )}
    </div>
  );
}

function requirementStatusLabel(status: RequirementsReport['requirements'][number]['status']) {
  switch (status) {
    case 'ready':
      return 'Ready';
    case 'corrupt':
      return 'Corrupt';
    case 'blocked':
      return 'Blocked';
    case 'error':
      return 'Error';
    default:
      return 'Missing';
  }
}

function formatBytes(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return '0 MB';
  }

  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function formatSpeed(bytesPerSecond: number) {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) {
    return '0.00 MB/s';
  }

  return `${(bytesPerSecond / 1024 / 1024).toFixed(2)} MB/s`;
}
