import { z } from 'zod';
import { PLATFORMS, type Platform } from './platform.ts';

const sha256Pattern = /^[a-f0-9]{64}$/i;
const extensionPattern = /^\.[a-z0-9]+$/i;
const profileIdPattern = /^[a-z0-9][a-z0-9-]*$/i;

const BUILT_IN_PROFILE_EXTENSIONS: Record<string, { platform: Platform; expectedExtensions: string[] }> = {
  'nes-mesen': { platform: 'nes', expectedExtensions: ['.nes'] },
  'snes-mesen': { platform: 'snes', expectedExtensions: ['.sfc', '.smc'] },
  'n64-rmg': { platform: 'n64', expectedExtensions: ['.z64', '.n64', '.v64'] },
  'gba-mgba': { platform: 'gba', expectedExtensions: ['.gba'] },
  'ps2-pcsx2': { platform: 'ps2', expectedExtensions: ['.iso', '.bin', '.img', '.chd'] },
  'psp-ppsspp': { platform: 'psp', expectedExtensions: ['.iso', '.cso', '.pbp'] },
  'ps1-manual': { platform: 'ps1', expectedExtensions: ['.cue', '.bin', '.iso', '.img', '.pbp', '.chd'] },
  'switch-manual': { platform: 'switch', expectedExtensions: ['.nsp', '.xci', '.nca'] },
  'snes-manual': { platform: 'snes', expectedExtensions: ['.sfc', '.smc'] },
  'ps2-manual': { platform: 'ps2', expectedExtensions: ['.iso', '.bin', '.img', '.chd'] },
  'psp-manual': { platform: 'psp', expectedExtensions: ['.iso', '.cso', '.pbp'] }
};

export interface RepositorySchema {
  metadata: {
    id: string;
    name: string;
    version: string;
    schemaVersion: 2 | 3;
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
export type GameContentMode = 'downloadable' | 'user_provided' | 'metadata_only';

export interface GameArtwork {
  cover?: string;
  hero?: string;
  logo?: string;
  screenshots?: string[];
}

export interface GameMetadata {
  releaseYear?: number;
  developer?: string;
  publisher?: string;
  genres?: string[];
  tags?: string[];
  players?: string;
  series?: string;
  externalIds?: Record<string, string>;
}

export interface ManualGameMetadataInput {
  title?: string;
  description?: string;
  cover?: string;
  hero?: string;
  logo?: string;
  screenshots?: string[];
  metadata?: GameMetadata;
}

export type ScrapeStatus = 'pending' | 'hashing' | 'fetching' | 'ready' | 'ambiguous' | 'failed' | 'skipped';
export type ScrapeMatchKind = 'hash' | 'name' | 'override' | string;

export interface ScrapeCandidate {
  provider: string;
  providerGameId: string;
  title: string;
  platform?: string | null;
  releaseYear?: number | null;
  developer?: string | null;
  cover?: string | null;
  matchKind: ScrapeMatchKind;
}

export interface ScrapeState {
  gameId: string;
  status: ScrapeStatus;
  matchKind?: ScrapeMatchKind | null;
  candidates: ScrapeCandidate[];
  message?: string | null;
  updatedAt: string;
}

export interface ScreenScraperStatus {
  configured: boolean;
  ssid?: string | null;
  region: 'auto' | 'eu' | 'us' | 'jp' | string;
  dailyRequests: number;
  dailyLimit: number;
}

export interface SteamGridDbStatus {
  configured: boolean;
  keySource: 'user' | 'built-in' | 'none' | string;
  dailyRequests: number;
  dailyLimit: number;
  pendingBatch: number;
  batchRunning: boolean;
}

export interface LibraryScrapeStatus {
  running: boolean;
  pending: number;
}

export interface LibraryScrapeProgressEvent {
  done: number;
  total: number;
  currentGameId?: string | null;
}

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
  artwork?: GameArtwork;
  metadata?: GameMetadata;
  contentMode?: GameContentMode;
  /** Reserved for the future Platform Setup Profiles step; no runtime behavior in this slice. */
  setupProfileId?: string;
  /** Alias used by zero-friction source libraries. Normalized to setupProfileId. */
  platformProfileId?: string;
  downloads: SourceUri[];
  expectedExtensions?: string[];
  requiredSystemFileIds?: string[];
  launch?: GameLaunchConfig;
}

export interface GameLaunchConfig {
  argsTemplate?: string;
  preferredFile?: string;
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
  artwork?: GameArtwork;
  metadata?: GameMetadata;
  contentMode?: GameContentMode;
  /** Reserved for the future Platform Setup Profiles step; no runtime behavior in this slice. */
  setupProfileId?: string;
  downloads: SourceUri[];
  expectedExtensions: string[];
  requiredSystemFileIds: string[];
  launch?: GameLaunchConfig;
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
  checksum?: string | null;
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
  status: 'ready' | 'completed' | 'error';
  localPath?: string;
  sha256?: string;
  message?: string;
  source: string;
  magnetUri: string;
  updatedAt: string;
}

export type ImportAssetFileStatus = 'installed' | 'already_installed' | 'error';
export type ImportAssetFileErrorCode =
  | 'unknown_asset'
  | 'unsupported_target'
  | 'source_missing'
  | 'source_not_file'
  | 'wrong_extension'
  | 'checksum_mismatch'
  | 'copy_failed'
  | 'store_failed';

export interface ImportAssetFileReport {
  status: ImportAssetFileStatus;
  installedPath: string;
  errorCode?: ImportAssetFileErrorCode;
}

export type ImportGameFileStatus = 'installed' | 'already_installed' | 'error';
export type ImportGameFileErrorCode =
  | 'unknown_game'
  | 'unsupported_target'
  | 'source_missing'
  | 'source_not_file'
  | 'wrong_extension'
  | 'checksum_mismatch'
  | 'copy_failed'
  | 'store_failed';

export interface ImportGameFileReport {
  status: ImportGameFileStatus;
  gameId: string;
  installedPath: string;
  sha256?: string;
  errorCode?: ImportGameFileErrorCode;
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
  subjectType?: 'asset' | 'game' | null;
  displayName?: string | null;
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
  subjectType?: 'asset' | 'game' | null;
  displayName?: string | null;
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
  | 'GameFileCorrupt'
  | 'SystemFilesMissing'
  | 'SystemFileCorrupt'
  | 'AlreadyRunning'
  | 'SpawnFailed';

export interface RepairLibraryReport {
  repaired: boolean;
  repositoryId?: string | null;
  removedPaths: string[];
}

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
  platformSetup: HealthCheckItem[];
  systemFiles: HealthCheckItem[];
  gameFiles: HealthCheckItem[];
  repositories: HealthCheckItem[];
  downloader: HealthCheckItem;
}

export interface PlatformSetupProfile {
  id: string;
  platform: Platform;
  displayName: string;
  emulator: PlatformSetupEmulator;
  gameFiles: PlatformSetupGameFiles;
  systemFiles: ProfileSystemFileRequirement[];
  launch: PlatformSetupLaunch;
}

export interface PlatformSetupEmulator {
  installMode: 'bundled' | 'downloadable' | 'manual';
  emulatorName: string;
  executableName?: string | null;
  executableCandidates: string[];
  download?: ProfileEmulatorDownload | null;
}

export interface ProfileEmulatorDownload {
  url: string;
  sha256: string;
  version: string;
}

export interface PlatformSetupGameFiles {
  expectedExtensions: string[];
  allowDirectory: boolean;
  preferredFilePatterns: string[];
  validators: string[];
}

export interface ProfileSystemFileRequirement {
  id: string;
  label: string;
  assetKind: 'bios' | 'firmware' | 'keys' | 'runtime' | string;
  required: boolean;
  extensions: string[];
  targetName?: string | null;
  checksum?: string | null;
  sourceMode: 'user_provided';
  notes?: string | null;
}

export interface PlatformSetupLaunch {
  argsTemplate: string;
  workingDirectory?: string | null;
  preferredFile?: string | null;
}

export interface ProfileEmulatorConfig {
  profileId: string;
  platform: Platform;
  exePath?: string | null;
  status: 'valid' | 'missing' | 'invalid' | string;
  lastValidatedAt?: string | null;
  version?: string | null;
  launchArgsTemplate?: string | null;
}

export interface ProfileEmulatorRemovalReport {
  profileId: string;
  platform: Platform;
  removedConfig: boolean;
  deletedFiles: boolean;
  removedPath?: string | null;
  message?: string | null;
}

export interface GameSetupState {
  gameId: string;
  profileId?: string | null;
  profileDisplayName?: string | null;
  unsupportedProfileId?: string | null;
  emulator: GameSetupEmulatorState;
  systemFiles: GameSetupSystemFileState[];
  repositoryRequirements: RequirementItem[];
  gameFile: GameSetupGameFileState;
  launch: GameSetupLaunchState;
  primaryAction: 'play' | 'import_game' | 'setup' | 'download' | 'details' | string;
}

export interface GameSetupEmulatorState {
  status: 'missing' | 'ready' | 'manual_required' | string;
  profileId?: string | null;
  platform: Platform;
  emulatorName: string;
  installMode: 'bundled' | 'downloadable' | 'manual' | string;
  executablePath?: string | null;
  message?: string | null;
}

export interface GameSetupSystemFileState {
  id: string;
  label: string;
  assetKind: string;
  required: boolean;
  status: 'missing' | 'ready' | 'invalid' | 'corrupt' | 'error' | string;
  installedPath?: string | null;
  expectedExtensions: string[];
  checksum?: string | null;
  message?: string | null;
}

export interface GameSetupGameFileState {
  status: 'missing' | 'ready' | 'invalid' | string;
  installedPath?: string | null;
  expectedExtensions: string[];
  allowDirectory: boolean;
  message?: string | null;
}

export interface GameSetupLaunchState {
  status: 'blocked' | 'ready' | string;
  blockers: string[];
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

export interface DiagnosticsPaths {
  dataDir: string;
  logPath: string;
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
export const gameContentModeSchema = z.enum(['downloadable', 'user_provided', 'metadata_only']);

export const gameArtworkSchema = z.object({
  cover: z.string().url().optional(),
  hero: z.string().url().optional(),
  logo: z.string().url().optional(),
  screenshots: z.array(z.string().url()).optional()
});

export const gameMetadataSchema = z.object({
  releaseYear: z.number().int().min(1950).max(2100).optional(),
  developer: z.string().min(1).optional(),
  publisher: z.string().min(1).optional(),
  genres: z.array(z.string().min(1)).optional(),
  tags: z.array(z.string().min(1)).optional(),
  players: z.string().min(1).optional(),
  series: z.string().min(1).optional(),
  externalIds: z.record(z.string().min(1), z.string().min(1)).optional()
});

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
  artwork: gameArtworkSchema.optional(),
  metadata: gameMetadataSchema.optional(),
  contentMode: gameContentModeSchema.optional(),
  setupProfileId: z.string().regex(profileIdPattern, 'setupProfileId must be a profile id like switch-manual').optional(),
  platformProfileId: z.string().regex(profileIdPattern, 'platformProfileId must be a profile id like nes-mesen').optional(),
  downloads: z.array(sourceUriSchema).min(1),
  expectedExtensions: z.array(extensionSchema).optional(),
  requiredSystemFileIds: z.array(z.string().min(1)).optional(),
  launch: z.object({
    argsTemplate: z.string().trim().min(1).optional(),
    preferredFile: z.string()
      .trim()
      .min(1)
      .regex(/^(?!\.?\/)(?!.*\\)(?!.*(?:^|\/)\.\.?(?:\/|$)).+$/, 'preferredFile must be a normalized relative path')
      .optional()
  }).optional()
}).superRefine((game, ctx) => {
  const profileId = game.platformProfileId ?? game.setupProfileId;
  const profile = profileId ? BUILT_IN_PROFILE_EXTENSIONS[profileId] : undefined;
  if (profile && profile.platform !== game.platform) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      path: [game.platformProfileId ? 'platformProfileId' : 'setupProfileId'],
      message: `profile ${profileId} is for ${profile.platform}, not ${game.platform}`
    });
  }
  if (!game.expectedExtensions?.length && !profile) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      path: ['expectedExtensions'],
      message: 'expectedExtensions is required unless setupProfileId names a built-in profile'
    });
  }
}).transform((game) => ({
  ...game,
  setupProfileId: game.platformProfileId ?? game.setupProfileId,
  expectedExtensions: game.expectedExtensions
    ?? BUILT_IN_PROFILE_EXTENSIONS[game.platformProfileId ?? game.setupProfileId ?? '']?.expectedExtensions
    ?? []
}));

export const repositorySchema = z.object({
  metadata: z.object({
    id: z.string().min(1),
    name: z.string().min(1),
    version: z.string().min(1),
    schemaVersion: z.union([z.literal(2), z.literal(3)]),
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
