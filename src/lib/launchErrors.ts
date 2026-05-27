import type { CatalogGame, LaunchFailure } from '@/types/repository';

export interface LaunchFailureView {
  title: string;
  message: string;
  actionLabel: string;
  actionKind: 'settings' | 'details' | 'retry-download' | 'close';
}

const FALLBACK_FAILURE: LaunchFailure = {
  kind: 'SpawnFailed',
  assets: [],
  message: 'Failed to launch the emulator.'
};

export function normalizeLaunchFailure(error: unknown, game?: CatalogGame): LaunchFailure {
  if (isLaunchFailure(error)) {
    return error;
  }

  if (typeof error === 'string') {
    return {
      ...FALLBACK_FAILURE,
      gameId: game?.id,
      message: error
    };
  }

  if (error instanceof Error) {
    return {
      ...FALLBACK_FAILURE,
      gameId: game?.id,
      message: error.message
    };
  }

  return {
    ...FALLBACK_FAILURE,
    gameId: game?.id
  };
}

export function launchFailureView(failure: LaunchFailure): LaunchFailureView {
  switch (failure.kind) {
    case 'EmulatorNotConfigured':
      return {
        title: 'Emulator not set up',
        message: 'Point RetroHydra to the emulator executable before launching this platform.',
        actionLabel: 'Open Settings',
        actionKind: 'settings'
      };
    case 'EmulatorFileMissing':
      return {
        title: 'Emulator file not found',
        message: failure.path
          ? `The configured emulator executable is missing: ${failure.path}`
          : 'The configured emulator executable is missing.',
        actionLabel: 'Re-select executable',
        actionKind: 'settings'
      };
    case 'GameFileMissing':
      return {
        title: 'Game file missing',
        message: failure.message ?? 'The downloaded game file was moved or deleted.',
        actionLabel: 'Re-download',
        actionKind: 'retry-download'
      };
    case 'SystemFilesMissing':
      return {
        title: 'System files required',
        message: missingAssetsMessage('Missing', failure.assets),
        actionLabel: 'Open Details',
        actionKind: 'details'
      };
    case 'SystemFileCorrupt':
      return {
        title: 'System file verification failed',
        message: missingAssetsMessage('Corrupt', failure.assets),
        actionLabel: 'Open Details',
        actionKind: 'details'
      };
    case 'AlreadyRunning':
      return {
        title: 'Game already running',
        message: 'RetroHydra already has a running emulator process for this game.',
        actionLabel: 'Close',
        actionKind: 'close'
      };
    default:
      return {
        title: 'Launch failed',
        message: failure.message ?? 'The emulator could not be started.',
        actionLabel: 'Close',
        actionKind: 'close'
      };
  }
}

function missingAssetsMessage(prefix: string, assets: string[]) {
  if (assets.length === 0) {
    return `${prefix} system files are required before launch.`;
  }

  return `${prefix}: ${assets.join(', ')}`;
}

function isLaunchFailure(value: unknown): value is LaunchFailure {
  if (!value || typeof value !== 'object') return false;
  const record = value as Partial<LaunchFailure>;
  return typeof record.kind === 'string' && Array.isArray(record.assets);
}
