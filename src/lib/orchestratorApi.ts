'use client';

import { useCallback, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { api } from './api.ts';
import type { InstallGameOptions } from './api.ts';
import { isTauriRuntime } from './runtime.ts';
import type { Platform } from '../types/platform.ts';
import type {
  EmulatorInstallResult,
  EmulatorStatus,
  InstallProgressEvent,
  InstallResult
} from '../types/emulatorProfile.ts';

export const installGame = (gameId: string, options?: InstallGameOptions): Promise<InstallResult> =>
  api.installGame(gameId, options);

export const installEmulator = (platform: Platform): Promise<EmulatorInstallResult> =>
  api.installEmulator(platform);

export const getEmulatorStatus = (platform: Platform): Promise<EmulatorStatus> =>
  api.getEmulatorStatus(platform);

export function listenInstallProgress(
  gameId: string,
  handler: (event: InstallProgressEvent) => void
): Promise<() => void> {
  if (!isTauriRuntime()) return Promise.resolve(() => undefined);

  return listen<InstallProgressEvent>('install:progress', (event) => {
    if (event.payload.gameId === gameId) handler(event.payload);
  });
}

export function useInstallGame(gameId: string) {
  const [progress, setProgress] = useState<InstallProgressEvent | null>(null);
  const [result, setResult] = useState<InstallResult | null>(null);
  const [running, setRunning] = useState(false);

  const install = useCallback(async (options?: InstallGameOptions) => {
    setRunning(true);
    setResult(null);
    setProgress({
      gameId,
      stage: 'emulator',
      message: 'Preparing installation...',
      percent: 1
    });
    const unlisten = await listenInstallProgress(gameId, setProgress);
    try {
      const nextResult = await installGame(gameId, options);
      setResult(nextResult);
      if (nextResult.status === 'ready') {
        setProgress({
          gameId,
          stage: 'done',
          message: 'Ready to play',
          percent: 100
        });
      }
      return nextResult;
    } finally {
      unlisten();
      setRunning(false);
    }
  }, [gameId]);

  return { install, progress, result, running };
}
