import type { Platform } from './platform.ts';

export interface EmulatorProfile {
  id: string;
  platform: Platform;
  displayName: string;
  versionStrategy: VersionStrategy;
  exeRelativePath: string;
  launchArgs: string[];
  requiresSystemFiles: boolean;
  license: string;
  portable: boolean;
}

export type VersionStrategy =
  | { kind: 'GithubLatest'; repo: string; assetPattern: string }
  | { kind: 'Fixed'; url: string; sha256: string };

export interface EmulatorInstallResult {
  profileId: string;
  exePath: string;
  version: string;
  fromCache: boolean;
}

export interface EmulatorStatus {
  platform: Platform;
  installed: boolean;
  exePath: string | null;
  profileId: string | null;
}

export type InstallStage = 'emulator' | 'system_files' | 'game' | 'verify' | 'done';

export interface InstallProgressEvent {
  gameId: string;
  stage: InstallStage;
  message: string;
  percent: number;
}

export interface InstallResult {
  gameId: string;
  status: 'ready' | 'needs_system_files' | 'needs_game_file' | 'error';
  errorCode: string | null;
  message: string | null;
}
