'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { api } from '@/lib/api';
import { isTauriRuntime } from '@/lib/runtime';
import type {
  DownloadProgressEvent,
  TorrentDownloadRecord,
  TorrentDownloadStatus
} from '@/types/repository';

const ACTIVE_STATUSES: TorrentDownloadStatus[] = ['resolving', 'downloading'];
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
  const [record, setRecord] = useState<TorrentDownloadRecord | null>(null);
  const [liveEvent, setLiveEvent] = useState<DownloadProgressEvent | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  const refresh = useCallback(async () => {
    setIsLoading(true);
    try {
      setRecord(await api.getGameDownload(gameId));
      setLiveEvent(null);
    } finally {
      setIsLoading(false);
    }
  }, [gameId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    if (!isTauriRuntime()) return;

    let active = true;
    const unlistenPromise = listen<DownloadProgressEvent>('download:progress', (event) => {
      if (!active || event.payload.gameId !== gameId) {
        return;
      }

      const payload = event.payload;
      setLiveEvent(payload);
      setRecord((previous) => previous ? mergeRecordWithEvent(previous, payload) : previous);
    });

    return () => {
      active = false;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [gameId]);

  const status = liveEvent?.status ?? record?.status ?? null;
  const progressPercent = clampPercent(
    liveEvent?.progressPercent ?? record?.progressPercent ?? (status === 'completed' ? 100 : 0)
  );

  return useMemo(() => ({
    record,
    liveEvent,
    status,
    isLoading,
    isActive: status !== null && ACTIVE_STATUSES.includes(status),
    isResumable: status !== null && RESUMABLE_STATUSES.includes(status),
    isCancelling: status === 'cancelling',
    canPlay: status === 'completed',
    hasError: status === 'error',
    saveDir: liveEvent?.saveDir ?? record?.saveDir ?? null,
    progressPercent,
    downloadedBytes: liveEvent?.downloadedBytes ?? record?.downloadedBytes ?? 0,
    totalBytes: liveEvent?.totalBytes ?? record?.totalBytes ?? 0,
    downloadSpeedBytesPerSec: liveEvent?.downloadSpeedBytesPerSec ?? record?.downloadSpeedBytesPerSec ?? 0,
    uploadSpeedBytesPerSec: liveEvent?.uploadSpeedBytesPerSec ?? record?.uploadSpeedBytesPerSec ?? 0,
    peersCount: liveEvent?.peersCount ?? record?.peersCount ?? 0,
    errorMessage: liveEvent?.error ?? record?.errorMessage ?? null,
    refresh,
    pause: async () => {
      setRecord(await api.pauseDownload(gameId));
      setLiveEvent(null);
    },
    resume: async () => {
      setRecord(await api.resumeDownload(gameId));
      setLiveEvent(null);
    },
    cancel: async () => {
      setRecord(await api.cancelDownload(gameId));
      setLiveEvent(null);
    }
  }), [gameId, isLoading, liveEvent, progressPercent, record, refresh, status]);
}

function mergeRecordWithEvent(
  record: TorrentDownloadRecord,
  event: DownloadProgressEvent
): TorrentDownloadRecord {
  return {
    ...record,
    subjectType: event.subjectType ?? record.subjectType,
    displayName: event.displayName ?? record.displayName,
    status: event.status,
    saveDir: event.saveDir || record.saveDir,
    progressPercent: event.progressPercent,
    downloadedBytes: event.downloadedBytes,
    totalBytes: event.totalBytes,
    downloadSpeedBytesPerSec: event.downloadSpeedBytesPerSec,
    uploadSpeedBytesPerSec: event.uploadSpeedBytesPerSec,
    peersCount: event.peersCount,
    errorMessage: event.error ?? null,
    completedAt: event.finished ? new Date().toISOString() : record.completedAt
  };
}

function clampPercent(value: number) {
  if (!Number.isFinite(value)) {
    return 0;
  }

  return Math.min(Math.max(value, 0), 100);
}
