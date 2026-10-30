import type { CatalogGame, RepositoryPreview, RepositorySummary } from './repository.ts';

/** IPC models stay separate from the snake_case publisher manifest. */
export interface FusionError {
  code: string;
  message: string;
  details: Record<string, unknown>;
}

export interface FusionSourcePreview {
  /** Host-owned location, supplied only by the local-directory IPC flow. */
  localDirectory?: string | null;
  sourceId: string;
  name: string;
  url: string;
  gameCount: number;
  publicKey: string | null;
  signatureVerified: boolean;
}

export interface FusionSourceSummary extends Omit<FusionSourcePreview, 'signatureVerified'> {
  prepareEden?: boolean;
  refreshedAt: string;
}

export interface FusionCatalogEntry {
  sourceId: string;
  sourceName: string;
  gameId: string;
  title: string;
  version: string;
  kind: 'fusion_source';
  installed: boolean;
  activeVersion: string | null;
  updateAvailable: boolean;
  platform: string | null;
  coverUrl: string | null;
  activeSnapshotId: string | null;
  /** Configuration/trust readiness; launching still performs the full integrity guard. */
  readyToPlay: boolean;
}

export type CatalogEntry =
  | { kind: 'legacy'; game: CatalogGame }
  | { kind: 'fusion_source'; game: FusionCatalogEntry };

export type SourcePreview =
  | { kind: 'legacy'; source: RepositoryPreview }
  | { kind: 'fusion_source'; source: FusionSourcePreview };
export type SourceSummary =
  | { kind: 'legacy'; source: RepositorySummary }
  | { kind: 'fusion_source'; source: FusionSourceSummary };

export type FusionRuntimeChannel = 'pinned' | 'dynamic';
export interface FusionGameOptions {
  /** Wave-zero contract only: non-null selections are explicitly unavailable. */
  buildProfileId?: string | null;
  controlsProfileId?: string | null;
  enabledAddons: string[] | null;
  runtimeChannel: FusionRuntimeChannel | null;
}

export interface FusionAddon {
  id: string;
  title: string | null;
  description: string | null;
  kind: string;
  enabled: boolean;
}

export interface FusionUserLayer {
  layerId: string;
  instructions: string | null;
  archive: string;
  rawPathRequired: boolean;
}

export interface FusionExecutableTrust {
  path: string;
  blake3: string;
  role: 'main_executable' | 'injector_dll' | 'core_library';
  trusted: boolean;
}

export interface FusionGameState {
  sourceId: string;
  gameId: string;
  title: string;
  version: string;
  platform: string | null;
  installed: boolean;
  activeVersion: string | null;
  ready: boolean;
  updateAvailable: boolean;
  activeSnapshotId: string | null;
  rollbackAvailable: boolean;
  options: FusionGameOptions;
  appliedOptions: FusionGameOptions | null;
  defaultRuntimeChannel: FusionRuntimeChannel | null;
  externalWorkspaceRoot: string | null;
  addons: FusionAddon[];
  runtimeChannels: FusionRuntimeChannel[];
  userLayers: FusionUserLayer[];
  executables: FusionExecutableTrust[];
  blockers: FusionError[];
  warnings: string[];
  operation: FusionOperationProgress | null;
  crashReportIds: string[];
  crashReports: FusionCrashSummary[];
}

export interface FusionCrashSummary {
  reportId: string;
  createdAt: string;
  exitCode: number | null;
  crashSignatureHash: string | null;
}

export interface FusionOperationProgress {
  operationId: string;
  sourceId: string;
  gameId: string;
  layerId: string | null;
  phase: string;
  sequence: number;
  status: string;
  error: FusionError | null;
  transfer?: {
    downloadedBytes: number;
    totalBytes: number;
    speedBytesPerSecond: number;
  } | null;
}

export interface FusionInstallInput {
  userFiles: Record<string, string>;
  rawPaths: Record<string, string>;
  externalWorkspaceRoot: string | null;
}

export interface FusionOperationReceipt {
  operationId: string;
  sourceId: string;
  gameId: string;
}

export interface FusionCrashReport {
  reportId: string;
  report: FusionSanitizedCrashReport;
  status: string;
  endpoint: string | null;
  canSend: boolean;
}

export interface FusionSanitizedCrashReport {
  schemaVersion: string;
  game: { sourceId: string; gameId: string };
  layerHashes: Record<string, string>;
  runtime: { channel: FusionRuntimeChannel; version: string; executableBlake3: string } | null;
  enabledAddons: string[];
  exitCode: number;
  cleanLogTail: string;
  crashSignatureHash: string;
}

export function fusionGameKey(game: Pick<FusionCatalogEntry, 'sourceId' | 'gameId'>) {
  return `${game.sourceId}::${game.gameId}`;
}

export function fusionErrorMessage(error: unknown): string {
  if (typeof error === 'object' && error !== null && 'message' in error && typeof error.message === 'string') {
    return error.message;
  }
  return String(error);
}

export function combineCatalog(legacy: CatalogGame[], fusion: FusionCatalogEntry[]): CatalogEntry[] {
  return [
    ...legacy.map((game): CatalogEntry => ({ kind: 'legacy', game })),
    ...fusion.map((game): CatalogEntry => ({ kind: 'fusion_source', game }))
  ];
}
