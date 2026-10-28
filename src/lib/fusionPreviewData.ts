import { fusionGameKey, type FusionCatalogEntry, type FusionCrashReport, type FusionGameOptions, type FusionGameState, type FusionInstallInput, type FusionOperationProgress, type FusionSourcePreview, type FusionSourceSummary } from '../types/fusionSource.ts';

export const FUSION_PREVIEW_URL = 'https://demo.fusion.invalid/index.json';
const gameIds = ['runtime-demo', 'overlay-demo', 'coop-demo', 'repair-demo'];
const source = { sourceId: 'fusion-preview', name: 'Fusion Source Preview', url: FUSION_PREVIEW_URL, gameCount: gameIds.length, publicKey: null };
let connected = false;
let localDirectory: string | null = null;
let prepareEden = false;
const states = new Map<string, FusionGameState>();
const listeners = new Set<(event: FusionOperationProgress) => void>();
let attempt = 0;
const completions = new Map<string, (event: FusionOperationProgress) => void>();
const reportIds = new Set(['preview-crash']);
const failedRepairs = new Set<string>();

function fixture(gameId: string): FusionGameState {
  const blocked = gameId === 'overlay-demo';
  const repair = gameId === 'repair-demo';
  const installed = gameId === 'runtime-demo' || repair;
  const appliedOptions: FusionGameOptions = { enabledAddons: ['shader-cache'], runtimeChannel: 'pinned' };
  return {
    sourceId: source.sourceId, gameId,
    title: gameId === 'runtime-demo' ? 'Orbit Runtime Demo' : blocked ? 'Overlay Capability Demo' : repair ? 'Integrity Recovery Demo' : 'Cooperative Workspace Demo',
    version: '1.1.0', platform: gameId === 'runtime-demo' ? 'switch' : blocked ? null : 'pc',
    installed, activeVersion: gameId === 'runtime-demo' ? '1.0.0' : repair ? '1.1.0' : null,
    activeSnapshotId: installed ? `preview-${gameId}-snapshot-1` : null,
    ready: gameId === 'runtime-demo', updateAvailable: gameId === 'runtime-demo', rollbackAvailable: installed,
    options: structuredClone(appliedOptions), appliedOptions: installed ? structuredClone(appliedOptions) : null,
    defaultRuntimeChannel: 'pinned', externalWorkspaceRoot: repair ? 'D:/Fusion/Workspaces/recovery' : null,
    addons: [{ id: 'shader-cache', title: 'Shader cache', description: 'Precompiled shaders reduce loading stutter.', kind: 'shader_cache', enabled: true }, { id: 'coop', title: null, description: null, kind: 'addon', enabled: false }],
    runtimeChannels: ['pinned', 'dynamic'],
    userLayers: gameId === 'coop-demo' ? [{ layerId: 'base', instructions: 'Choose your own game files. Preview uses a simulated local directory.', archive: 'raw', rawPathRequired: true }] : [],
    executables: installed ? [{ path: 'runtime/engine.exe', blake3: 'a'.repeat(64), role: 'main_executable', trusted: true }, { path: 'runtime/core.dll', blake3: 'c'.repeat(64), role: 'core_library', trusted: true }] : [],
    blockers: blocked ? [{ code: 'missing_capability', message: 'This game requires vfs.overlay_mount.v2. No compatible fallback is declared.', details: { capability: 'vfs.overlay_mount.v2' } }] : repair ? [{ code: 'corrupted_critical_binary', message: 'The runtime binary differs from the approved snapshot.', details: { path: 'runtime/core.dll' } }] : [],
    warnings: gameId === 'runtime-demo' ? ['Shader cache is optional and may be rebuilt by the runtime.'] : [],
    operation: null, crashReportIds: gameId === 'runtime-demo' ? [...reportIds] : [],
    crashReports: gameId === 'runtime-demo' ? [...reportIds].map((reportId) => ({ reportId, createdAt: '2026-10-19T06:10:00Z', exitCode: 1, crashSignatureHash: 'b'.repeat(64) })) : []
  };
}

function state(sourceId: string, gameId: string) {
  if (sourceId !== source.sourceId) throw { code: 'missing_source', message: 'Preview source was not found.', details: {} };
  if (!gameIds.includes(gameId)) throw { code: 'invalid_id', message: 'Preview game was not found.', details: {} };
  if (!states.has(gameId)) states.set(gameId, fixture(gameId));
  return states.get(gameId)!;
}

function emit(event: FusionOperationProgress) {
  state(event.sourceId, event.gameId).operation = event;
  listeners.forEach((listener) => listener(structuredClone(event)));
}

function finish(event: FusionOperationProgress) {
  emit(event);
  completions.get(event.operationId)?.(event);
  completions.delete(event.operationId);
}

export const fusionPreviewApi = {
  async previewLocal(directory: string): Promise<FusionSourcePreview> {
    if (!['preview://Sources/local-release', 'preview://Sources/local-release-v2'].includes(directory)) throw { code: 'invalid_path', message: 'Choose a preview source folder.', details: {} };
    return { ...source, url: 'https://offline.fusion.invalid/fusion-preview/source-index.json', localDirectory: directory, signatureVerified: false };
  },
  async addLocal(directory: string, expectedSourceId: string, eden: boolean): Promise<FusionSourceSummary> {
    const preview = await this.previewLocal(directory);
    if (expectedSourceId !== preview.sourceId) throw { code: 'snapshot_conflict', message: 'The folder belongs to a different source.', details: {} };
    if ([...states.values()].some((game) => isActive(game.operation))) throw { code: 'operation_in_progress', message: 'Wait for the active source operation.', details: {} };
    connected = true; localDirectory = directory; prepareEden = eden;
    if (directory.endsWith('-v2')) gameIds.forEach((id) => { const game = state(source.sourceId, id); game.version = '1.2.0'; game.updateAvailable = game.installed && game.activeVersion !== game.version; });
    return { ...preview, prepareEden, refreshedAt: new Date().toISOString() };
  },
  async preview(url: string): Promise<FusionSourcePreview> {
    if (url !== FUSION_PREVIEW_URL) throw { code: 'unsupported_schema', message: 'This browser preview URL uses a legacy source.', details: {} };
    return { ...source, signatureVerified: false };
  },
  async add(url: string): Promise<FusionSourceSummary> {
    await this.preview(url);
    connected = true;
    localDirectory = null; prepareEden = false;
    return { ...source, refreshedAt: new Date().toISOString() };
  },
  async list(): Promise<FusionSourceSummary[]> {
    return connected ? [{ ...source, ...(localDirectory ? { url: 'https://offline.fusion.invalid/fusion-preview/source-index.json', localDirectory, prepareEden } : {}), refreshedAt: '2026-10-17T00:00:00Z' }] : [];
  },
  async catalog(): Promise<FusionCatalogEntry[]> {
    if (!connected) return [];
    return gameIds.map((gameId) => {
      const game = state(source.sourceId, gameId);
      return { kind: 'fusion_source', sourceId: source.sourceId, sourceName: source.name, gameId, title: game.title, version: game.version, installed: game.installed, activeVersion: game.activeVersion, updateAvailable: game.updateAvailable, platform: game.platform, coverUrl: gameId === 'runtime-demo' ? '/demo-content/cover-ps1.svg' : gameId === 'overlay-demo' ? '/demo-content/missing-cover.svg' : null, activeSnapshotId: game.activeSnapshotId, readyToPlay: game.ready && !isActive(game.operation) };
    });
  },
  async refresh(sourceId: string) {
    if (sourceId !== source.sourceId) throw new Error('Preview source was not found.');
    return { ...source, ...(localDirectory ? { url: 'https://offline.fusion.invalid/fusion-preview/source-index.json', localDirectory, prepareEden } : {}), refreshedAt: new Date().toISOString() };
  },
  async remove(sourceId: string) {
    if (sourceId !== source.sourceId) return;
    if ([...states.values()].some((game) => game.operation?.status === 'running')) throw { code: 'operation_in_progress', message: 'The source has active operations.', details: {} };
    if (gameIds.some((id) => state(sourceId, id).installed)) throw { code: 'snapshot_conflict', message: "Remove the source's installed games before disconnecting it.", details: {} };
    connected = false;
  },
  async state(sourceId: string, gameId: string) {
    const game = structuredClone(state(sourceId, gameId));
    if (!isActive(game.operation)) game.operation = null;
    game.ready = game.ready && !game.operation;
    return game;
  },
  async setOptions(sourceId: string, gameId: string, options: FusionGameOptions) {
    const game = state(sourceId, gameId);
    game.options = structuredClone(options);
    game.addons.forEach((addon) => { addon.enabled = options.enabledAddons?.includes(addon.id) ?? addon.enabled; });
    game.updateAvailable = game.installed && (game.activeVersion !== game.version || JSON.stringify(game.options) !== JSON.stringify(game.appliedOptions));
    return structuredClone(game);
  },
  async install(sourceId: string, gameId: string, input: FusionInstallInput) {
    const game = state(sourceId, gameId);
    const blocked = game.blockers.find((blocker) => !['untrusted_executable', 'missing_injector_dll', 'corrupted_critical_binary', 'local_base_modified'].includes(blocker.code));
    if (blocked) throw blocked;
    if (!game.installed && game.userLayers.some((layer) => !input.userFiles[layer.layerId])) throw { code: 'missing_layer', message: 'Select the required user files before installing.', details: { layerId: 'base' } };
    if (game.installed && input.externalWorkspaceRoot && input.externalWorkspaceRoot !== game.externalWorkspaceRoot) throw { code: 'snapshot_conflict', message: 'An installed workspace cannot be moved.', details: {} };
    const operationId = `preview-operation-${++attempt}`;
    const event = { operationId, sourceId, gameId, layerId: 'runtime', phase: 'fetch', sequence: 1, status: 'running', error: null, transfer: { downloadedBytes: 1048576, totalBytes: 8388608, speedBytesPerSecond: 2097152 } } satisfies FusionOperationProgress;
    emit(event);
    const completion = new Promise<FusionOperationProgress>((resolve) => completions.set(operationId, resolve));
    setTimeout(() => {
      if (game.operation?.operationId !== operationId || game.operation.status !== 'running') return;
      emit({ ...event, phase: 'transform', layerId: 'runtime', sequence: 2, transfer: null });
    }, 350);
    setTimeout(() => {
      if (game.operation?.operationId !== operationId || game.operation.status !== 'running') return;
      if (gameId === 'repair-demo' && !failedRepairs.has(gameId)) {
        failedRepairs.add(gameId);
        finish({ ...event, phase: 'failed', sequence: 3, status: 'failed', transfer: null, error: { code: 'network', message: 'The preview download was interrupted. Retry to restore the files.', details: {} } });
        return;
      }
      game.installed = true; game.activeVersion = game.version; game.updateAvailable = false;
      game.rollbackAvailable = true; game.activeSnapshotId = `snapshot-${attempt}`;
      game.appliedOptions = structuredClone(game.options);
      game.externalWorkspaceRoot ??= input.externalWorkspaceRoot;
      game.blockers = [];
      if (!game.executables.length) game.executables = [{ path: 'runtime/engine.exe', blake3: 'a'.repeat(64), role: 'main_executable', trusted: false }];
      game.ready = game.executables.every((exe) => exe.trusted);
      if (!game.ready) game.blockers = game.executables.filter((exe) => !exe.trusted).map((exe) => ({ code: 'untrusted_executable', message: 'Approve this executable hash before launching.', details: { path: exe.path, blake3: exe.blake3 } }));
      finish({ ...event, phase: 'complete', sequence: 3, status: 'completed', transfer: null });
    }, 950);
    return completion;
  },
  async cancel(sourceId: string, gameId: string) {
    const game = state(sourceId, gameId);
    if (game.operation?.status === 'running') finish({ ...game.operation, sequence: game.operation.sequence + 1, phase: 'cancelled', status: 'cancelled' });
  },
  async rollback(sourceId: string, gameId: string) {
    const game = state(sourceId, gameId);
    game.activeVersion = '1.0.0'; game.updateAvailable = true;
    game.activeSnapshotId = `preview-rollback-${attempt + 1}`;
    const event = { operationId: `preview-rollback-${++attempt}`, sourceId, gameId, layerId: null, phase: 'completed', sequence: 1, status: 'completed', error: null } satisfies FusionOperationProgress;
    emit(event); return event;
  },
  async launch(sourceId: string, gameId: string) {
    const game = state(sourceId, gameId);
    if (!game.ready) throw { code: 'untrusted_executable', message: 'Approve the executable hash before launching.', details: {} };
    const operationId = `preview-launch-${++attempt}`;
    const completion = new Promise<FusionOperationProgress>((resolve) => completions.set(operationId, resolve));
    emit({ operationId, sourceId, gameId, layerId: null, phase: 'exec', sequence: 1, status: 'running', error: null });
    setTimeout(() => { if (game.operation?.operationId === operationId && game.operation.status === 'running') finish({ operationId, sourceId, gameId, layerId: null, phase: 'complete', sequence: 2, status: 'completed', error: null }); }, 900);
    return completion;
  },
  async removeGame(sourceId: string, gameId: string) {
    const game = state(sourceId, gameId);
    game.installed = false; game.ready = false; game.activeVersion = null; game.activeSnapshotId = null; game.rollbackAvailable = false; game.appliedOptions = null; game.externalWorkspaceRoot = null;
    game.blockers = [];
    const event = { operationId: `preview-remove-${++attempt}`, sourceId, gameId, layerId: null, phase: 'completed', sequence: 1, status: 'completed', error: null } satisfies FusionOperationProgress;
    emit(event); return event;
  },
  async trust(sourceId: string, gameId: string, path: string, blake3: string) {
    const game = state(sourceId, gameId);
    const executable = game.executables.find((exe) => exe.path === path && exe.blake3 === blake3);
    if (!executable) throw { code: 'integrity_violation', message: 'The executable hash changed. Reload the game details.', details: {} };
    executable.trusted = true;
    game.blockers = game.blockers.filter((blocker) => blocker.code !== 'untrusted_executable' || blocker.details.path !== path || blocker.details.blake3 !== blake3);
    game.ready = game.installed && !game.blockers.length && game.executables.every((exe) => exe.trusted);
  },
  async crash(reportId: string): Promise<FusionCrashReport> {
    if (!reportIds.has(reportId)) throw new Error('Report was discarded.');
    return { reportId, report: { schemaVersion: '1.0', game: { sourceId: source.sourceId, gameId: 'runtime-demo' }, layerHashes: { runtime: 'a'.repeat(64) }, runtime: { channel: 'pinned', version: '1.0.0', executableBlake3: 'a'.repeat(64) }, enabledAddons: ['shader-cache'], exitCode: 1, cleanLogTail: 'Runtime exited with code 1.\nWorkspace: <WORKSPACE_ROOT>\nContact: <EMAIL>\nToken: <TOKEN>', crashSignatureHash: 'b'.repeat(64) }, status: 'pending', endpoint: null, canSend: false };
  },
  async sendCrash(_reportId: string, _expectedEndpoint: string): Promise<void> { throw new Error('No telemetry endpoint is configured for this preview source.'); },
  async discardCrash(reportId: string) { reportIds.delete(reportId); states.forEach((game) => { game.crashReportIds = game.crashReportIds.filter((id) => id !== reportId); game.crashReports = game.crashReports.filter((report) => report.reportId !== reportId); }); },
  subscribe(listener: (event: FusionOperationProgress) => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
  reset() { connected = false; localDirectory = null; prepareEden = false; states.clear(); completions.clear(); failedRepairs.clear(); reportIds.clear(); reportIds.add('preview-crash'); }
};

function isActive(operation: FusionOperationProgress | null) {
  return operation !== null && ['running', 'cancelling'].includes(operation.status);
}

export { fusionGameKey };
