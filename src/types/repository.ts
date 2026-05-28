import { z } from 'zod';
import { PLATFORMS, type Platform } from './platform.ts';

const sha256Pattern = /^[a-f0-9]{64}$/i;
const extensionPattern = /^\.[a-z0-9]+$/i;

export interface RepositorySchema {
  metadata: {
    id: string;
    name: string;
    version: string;
    schemaVersion: 2;
    maintainer?: string;
    homepageUrl?: string;
    license?: string;
    trustLevel?: RepositoryTrustLevel;
    contentHash?: string;
    updatedAt?: string;
  };
  system_files: RepositoryAsset[];
  catalog: RepositoryGame[];
}

export type SourceUri =
  | { kind: 'http'; url: string; sha256: string; sizeBytes?: number }
  | { kind: 'bundled'; path: string; sha256: string; sizeBytes?: number }
  | { kind: 'magnet'; uri: string; infoHash?: string; sizeBytes?: number }
  | { kind: 'user_provided'; instructions?: string; sha256?: string; sizeBytes?: number };

export type RepositoryTrustLevel = 'official' | 'community' | 'unknown';

export interface RepositoryAsset {
  id: string;
  platform: Platform;
  assetKind: 'emulator' | 'bios' | 'firmware' | 'keys' | 'patch' | 'runtime';
  displayName: string;
  sources: SourceUri[];
  installHint?: {
    target: 'app_system' | 'emulator_dir' | 'user_selected';
    relativePath?: string;
  };
  executable?: boolean;
}

export interface RepositoryGame {
  id: string;
  platform: Platform;
  title: string;
  description?: string;
  coverImageUrl?: string;
  trailerUrl?: string;
  downloads: SourceUri[];
  expectedExtensions: string[];
  requiredSystemFileIds?: string[];
}

export interface RepositorySummary {
  id: string;
  name: string;
  version: string;
  url: string;
  connectedAt: string;
  catalogCount: number;
  systemFileCount: number;
  maintainer?: string;
  homepageUrl?: string;
  license?: string;
  trustLevel: RepositoryTrustLevel;
  contentHash?: string;
  lastRefreshedAt?: string;
  hasExecutableAssets: boolean;
}

export interface RepositoryPreview {
  url: string;
  id: string;
  name: string;
  version: string;
  maintainer?: string;
  homepageUrl?: string;
  license?: string;
  trustLevel: RepositoryTrustLevel;
  catalogCount: number;
  systemFileCount: number;
  hasExecutableAssets: boolean;
  contentHash: string;
}

export interface CatalogGame {
  id: string;
  sourceId: string;
  repositoryId: string;
  repositoryName: string;
  platform: Platform;
  title: string;
  description?: string;
  coverImageUrl?: string;
  trailerUrl?: string;
  downloads: SourceUri[];
  expectedExtensions: string[];
  requiredSystemFileIds: string[];
}

export interface AssetView {
  id: string;
  sourceId: string;
  repositoryId: string;
  platform: Platform;
  assetKind: RepositoryAsset['assetKind'];
  displayName: string;
  sources: SourceUri[];
  installHint?: RepositoryAsset['installHint'];
  executable: boolean;
}

export type AssetInstallationStatus = 'ready' | 'missing' | 'corrupt' | 'blocked' | 'error';

export interface RequirementItem {
  asset: AssetView;
  status: AssetInstallationStatus;
  downloaded: boolean;
  trusted: boolean;
  localPath?: string;
  targetPath?: string;
  sha256?: string;
  message?: string;
}

export interface RequirementsReport {
  gameId: string;
  ready: boolean;
  gameDownloaded: boolean;
  requirements: RequirementItem[];
}

export interface LibraryGameStatus {
  gameId: string;
  installed: boolean;
  systemRequirementsReady: boolean;
  missingRequirements: string[];
  download: TorrentDownloadRecord | null;
}

export interface DownloadRecord {
  subjectId: string;
  subjectType: 'asset' | 'game';
  status: 'ready' | 'error';
  localPath?: string;
  sha256?: string;
  message?: string;
  updatedAt: string;
}

export interface GameDownloadStartReport {
  gameId: string;
  sourceKind: 'http' | 'bundled' | 'magnet';
  saveDir: string;
  record?: DownloadRecord | null;
  torrent?: TorrentDownloadRecord | null;
}

export type TorrentDownloadStatus =
  | 'resolving'
  | 'downloading'
  | 'paused'
  | 'interrupted'
  | 'completed'
  | 'cancelling'
  | 'cancelled'
  | 'error';

export interface TorrentDownloadRecord {
  gameId: string;
  magnetUri: string;
  saveDir: string;
  status: TorrentDownloadStatus;
  progressPercent: number;
  downloadedBytes: number;
  totalBytes: number;
  downloadSpeedBytesPerSec: number;
  uploadSpeedBytesPerSec: number;
  peersCount: number;
  torrentId?: number | null;
  errorMessage?: string | null;
  createdAt: string;
  updatedAt: string;
  completedAt?: string | null;
}

export interface TrustedExecutable {
  assetId: string;
  localPath: string;
  sha256: string;
  trustedAt: string;
}

export type EmulatorConfigStatus = 'valid' | 'missing' | 'invalid';

export interface EmulatorConfig {
  platform: Platform;
  exePath?: string;
  status: EmulatorConfigStatus;
  lastValidatedAt?: string;
  version?: string;
  launchArgsTemplate?: string;
}

export type RecommendedEmulatorInstallStatus = 'available' | 'installed' | 'manual';

export interface RecommendedEmulator {
  platform: Platform;
  platformLabel: string;
  emulatorName: string;
  version?: string | null;
  downloadUrl?: string | null;
  sha256?: string | null;
  executableName: string;
  status: RecommendedEmulatorInstallStatus;
  installedPath?: string | null;
  message?: string | null;
}

export interface TorrentStartReport {
  gameId: string;
  state: string;
  saveDir: string;
}

export interface TorrentStatus {
  gameId: string;
  state: TorrentDownloadStatus;
  progress: number;
  downloadedBytes: number;
  totalBytes: number;
  downloadSpeedBytesPerSec: number;
  uploadSpeedBytesPerSec: number;
  peersCount: number;
  finished: boolean;
  saveDir: string;
  error?: string | null;
}

export interface DownloadProgressEvent {
  gameId: string;
  status: TorrentDownloadStatus;
  progress: number;
  progressPercent: number;
  downloadedBytes: number;
  totalBytes: number;
  downloadSpeedBytesPerSec: number;
  uploadSpeedBytesPerSec: number;
  peersCount: number;
  finished: boolean;
  saveDir: string;
  error?: string | null;
}

export interface LaunchReport {
  pid: number;
  executable: string;
  gamePath: string;
  resolvedGamePath: string;
  args: string[];
}

export type LaunchFailureKind =
  | 'EmulatorNotConfigured'
  | 'EmulatorFileMissing'
  | 'GameFileMissing'
  | 'SystemFilesMissing'
  | 'SystemFileCorrupt'
  | 'AlreadyRunning'
  | 'SpawnFailed';

export interface LaunchFailure {
  kind: LaunchFailureKind;
  platform?: Platform;
  gameId?: string;
  path?: string;
  assets: string[];
  message?: string;
}

export interface OnboardingState {
  step: 'addRepository' | 'configureEmulator' | 'complete';
  repositoriesConfigured: boolean;
  emulatorsConfigured: boolean;
  catalogCount: number;
  validEmulatorCount: number;
}

export type HealthStatus = 'ready' | 'missing' | 'corrupt' | 'blocked' | 'error' | string;

export interface HealthCheckItem {
  id: string;
  label: string;
  status: HealthStatus;
  message?: string;
  action?: string;
  path?: string;
}

export interface HealthReport {
  generatedAt: string;
  emulators: HealthCheckItem[];
  systemFiles: HealthCheckItem[];
  gameFiles: HealthCheckItem[];
  repositories: HealthCheckItem[];
  downloader: HealthCheckItem;
}

export interface DiagnosticsBundle {
  generatedAt: string;
  appVersion: string;
  os: string;
  dataDir: string;
  logPath: string;
  health: HealthReport;
  downloads: TorrentDownloadRecord[];
  logs: string[];
}

export type UpdateCheckErrorKind = 'endpointUnreachable' | 'parseError' | 'signatureInvalid';

export interface UpdateCheckError {
  kind: UpdateCheckErrorKind;
  message?: string;
}

export interface UpdateCheckReport {
  available: boolean;
  currentVersion: string;
  version?: string | null;
  date?: string | null;
  body?: string | null;
}

export const sourceUriSchema = z.discriminatedUnion('kind', [
  z.object({
    kind: z.literal('http'),
    url: z.string().url(),
    sha256: z.string().regex(sha256Pattern, 'HTTP sources must include a lowercase or uppercase SHA-256 hash'),
    sizeBytes: z.number().int().positive().optional()
  }),
  z.object({
    kind: z.literal('bundled'),
    path: z.string()
      .trim()
      .min(1)
      .regex(/^(?!\.?\/)(?!.*\/\/)(?!.*\\)(?!.*(?:^|\/)\.\.?(?:\/|$)).+$/, 'Bundled paths must be normalized relative paths'),
    sha256: z.string().regex(sha256Pattern, 'Bundled sources must include a lowercase or uppercase SHA-256 hash'),
    sizeBytes: z.number().int().positive().optional()
  }),
  z.object({
    kind: z.literal('magnet'),
    uri: z.string().startsWith('magnet:'),
    infoHash: z.string().min(32).optional(),
    sizeBytes: z.number().int().positive().optional()
  }),
  z.object({
    kind: z.literal('user_provided'),
    instructions: z.string().optional(),
    sha256: z.string().regex(sha256Pattern).optional(),
    sizeBytes: z.number().int().positive().optional()
  })
]);

export const extensionSchema = z.string()
  .trim()
  .regex(extensionPattern, 'Extensions must start with "." and contain only letters or numbers')
  .transform((extension) => extension.toLowerCase());

export const platformSchema = z.enum(PLATFORMS);

export const repositoryAssetSchema = z.object({
  id: z.string().min(1),
  platform: platformSchema,
  assetKind: z.enum(['emulator', 'bios', 'firmware', 'keys', 'patch', 'runtime']),
  displayName: z.string().min(1),
  sources: z.array(sourceUriSchema).min(1),
  installHint: z.object({
    target: z.enum(['app_system', 'emulator_dir', 'user_selected']),
    relativePath: z.string().min(1).optional()
  }).optional(),
  executable: z.boolean().optional()
});

export const repositoryGameSchema = z.object({
  id: z.string().min(1),
  platform: platformSchema,
  title: z.string().min(1),
  description: z.string().optional(),
  coverImageUrl: z.string().url().optional(),
  trailerUrl: z.string().url().optional(),
  downloads: z.array(sourceUriSchema).min(1),
  expectedExtensions: z.array(extensionSchema).min(1),
  requiredSystemFileIds: z.array(z.string().min(1)).optional()
});

export const repositorySchema = z.object({
  metadata: z.object({
    id: z.string().min(1),
    name: z.string().min(1),
    version: z.string().min(1),
    schemaVersion: z.literal(2),
    maintainer: z.string().optional(),
    homepageUrl: z.string().url().optional(),
    license: z.string().optional(),
    trustLevel: z.enum(['official', 'community', 'unknown']).optional(),
    contentHash: z.string().regex(sha256Pattern).optional(),
    updatedAt: z.string().optional()
  }),
  system_files: z.array(repositoryAssetSchema),
  catalog: z.array(repositoryGameSchema)
}) satisfies z.ZodType<RepositorySchema>;

export function validateRepositorySchema(input: unknown): RepositorySchema {
  return repositorySchema.parse(input);
}
