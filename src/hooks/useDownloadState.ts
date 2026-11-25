'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import { api } from '@/lib/api';
import { useLauncherStore } from '@/stores/launcherStore';
import type {
  DownloadProgressEvent,
  TorrentDownloadRecord,
  TorrentDownloadStatus
} from '@/types/repository';

const ACTIVE_STATUSES: TorrentDownloadStatus[] = ['queued', 'resolving', 'downloading'];
const RESUMABLE_STATUSES: TorrentDownloadStatus[] = ['paused', 'interrupted', 'error'];

export interface DownloadState {
  record: TorrentDownloadRecord | null;
  liveEvent: DownloadProgressEvent | null;
  status: TorrentDownloadStatus | null;
  isLoading: boolean;
  isActive: boolean;
  isResumable: boolean;
  isCancelling: boolean;
  canPlay: boolean;
  hasError: boolean;
  saveDir: string | null;
  progressPercent: number;
  downloadedBytes: number;
  totalBytes: number;
  downloadSpeedBytesPerSec: number;
  uploadSpeedBytesPerSec: number;
  peersCount: number;
  errorMessage: string | null;
  refresh: () => Promise<void>;
  pause: () => Promise<void>;
  resume: () => Promise<void>;
  cancel: () => Promise<void>;
}

export function useDownloadState(gameId: string): DownloadState {
  const record = useLauncherStore((state) => state.downloads.find((item) => item.gameId === gameId) ?? null);
  const [isLoading, setIsLoading] = useState(true);

  const refresh = useCallback(async () => {
    setIsLoading(true);
    try {
      applySnapshot(await api.getGameDownload(gameId));
    } finally {
      setIsLoading(false);
    }
  }, [gameId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const status = record?.status ?? null;
  const progressPercent = clampPercent(
    record?.progressPercent ?? (status === 'completed' ? 100 : 0)
  );

  return useMemo(() => ({
    record,
    liveEvent: null,
    status,
    isLoading,
    isActive: status !== null && ACTIVE_STATUSES.includes(status),
    isResumable: status !== null && RESUMABLE_STATUSES.includes(status),
    isCancelling: status === 'cancelling',
    canPlay: status === 'completed',
    hasError: status === 'error',
    saveDir: record?.saveDir ?? null,
    progressPercent,
    downloadedBytes: record?.downloadedBytes ?? 0,
    totalBytes: record?.totalBytes ?? 0,
    downloadSpeedBytesPerSec: record?.downloadSpeedBytesPerSec ?? 0,
    uploadSpeedBytesPerSec: record?.uploadSpeedBytesPerSec ?? 0,
    peersCount: record?.peersCount ?? 0,
    errorMessage: record?.errorMessage ?? null,
    refresh,
    pause: async () => {
      applySnapshot(await api.pauseDownload(gameId));
    },
    resume: async () => {
      applySnapshot(await api.resumeDownload(gameId));
    },
    cancel: async () => {
      applySnapshot(await api.cancelDownload(gameId));
    }
  }), [gameId, isLoading, progressPercent, record, refresh, status]);
}

function applySnapshot(record: TorrentDownloadRecord | null) {
  if (!record) return;
  const state = useLauncherStore.getState();
  state.setDownloads([...state.downloads.filter((item) => item.gameId !== record.gameId), record]);
}

function clampPercent(value: number) {
  if (!Number.isFinite(value)) {
    return 0;
  }

  return Math.min(Math.max(value, 0), 100);
}
