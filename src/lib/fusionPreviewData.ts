import { fusionGameKey, type FusionCatalogEntry, type FusionCrashReport, type FusionGameOptions, type FusionGameState, type FusionInstallInput, type FusionOperationProgress, type FusionSourcePreview, type FusionSourceSummary } from '../types/fusionSource.ts';

export const FUSION_PREVIEW_URL = 'https://demo.fusion.invalid/index.json';
const source = { sourceId: 'fusion-preview', name: 'Fusion Source Preview', url: FUSION_PREVIEW_URL, gameCount: 3, publicKey: null };
let connected = false;
const states = new Map<string, FusionGameState>();
const listeners = new Set<(event: FusionOperationProgress) => void>();
let attempt = 0;
const completions = new Map<string, (event: FusionOperationProgress) => void>();
const reportIds = new Set(['preview-crash']);

function fixture(gameId: string): FusionGameState {
  const blocked = gameId === 'overlay-demo';
  return {
    sourceId: source.sourceId, gameId,
    title: gameId === 'runtime-demo' ? 'Orbit Runtime Demo' : blocked ? 'Overlay Capability Demo' : 'Cooperative Workspace Demo',
    version: '1.1.0', platform: gameId === 'runtime-demo' ? 'Switch' : null,
    installed: gameId === 'runtime-demo', activeVersion: gameId === 'runtime-demo' ? '1.0.0' : null,
    activeSnapshotId: gameId === 'runtime-demo' ? 'preview-snapshot-1' : null,
    ready: gameId === 'runtime-demo', updateAvailable: gameId === 'runtime-demo', rollbackAvailable: gameId === 'runtime-demo',
    options: { enabledAddons: ['shader-cache'], runtimeChannel: 'pinned' },
    addons: [{ id: 'shader-cache', kind: 'shader_cache', enabled: true }, { id: 'coop', kind: 'addon', enabled: false }],
    runtimeChannels: ['pinned', 'dynamic'],
    userLayers: gameId === 'coop-demo' ? [{ layerId: 'base', instructions: 'Choose your own game files. Preview uses a simulated local directory.', archive: 'raw', rawPathRequired: true }] : [],
    executables: [{ path: 'runtime/engine.exe', blake3: 'a'.repeat(64), role: 'main_executable', trusted: gameId === 'runtime-demo' }],
    blockers: blocked ? [{ code: 'missing_capability', message: 'This game requires vfs.overlay_mount.v2. No compatible fallback is declared.', details: { capability: 'vfs.overlay_mount.v2' } }] : [],
    warnings: gameId === 'runtime-demo' ? ['Shader cache is optional and may be rebuilt by the runtime.'] : [],
    operation: null, crashReportIds: gameId === 'runtime-demo' ? [...reportIds] : []
  };
}

function state(sourceId: string, gameId: string) {
  if (sourceId !== source.sourceId) throw { code: 'missing_source', message: 'Preview source was not found.', details: {} };
  if (!['runtime-demo', 'overlay-demo', 'coop-demo'].includes(gameId)) throw { code: 'invalid_id', message: 'Preview game was not found.', details: {} };
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
  async preview(url: string): Promise<FusionSourcePreview> {
    if (url !== FUSION_PREVIEW_URL) throw { code: 'unsupported_schema', message: 'This browser preview URL uses a legacy source.', details: {} };
    return { ...source, signatureVerified: false };
  },
  async add(url: string): Promise<FusionSourceSummary> {
    await this.preview(url);
    connected = true;
    return { ...source, refreshedAt: new Date().toISOString() };
  },
  async list(): Promise<FusionSourceSummary[]> {
    return connected ? [{ ...source, refreshedAt: '2026-10-17T00:00:00Z' }] : [];
  },
  async catalog(): Promise<FusionCatalogEntry[]> {
    if (!connected) return [];
    return ['runtime-demo', 'overlay-demo', 'coop-demo'].map((gameId) => {
      const game = state(source.sourceId, gameId);
      return { kind: 'fusion_source', sourceId: source.sourceId, sourceName: source.name, gameId, title: game.title, version: game.version, installed: game.installed, activeVersion: game.activeVersion, updateAvailable: game.updateAvailable };
    });
  },
  async refresh(sourceId: string) {
    if (sourceId !== source.sourceId) throw new Error('Preview source was not found.');
    return { ...source, refreshedAt: new Date().toISOString() };
  },
  async remove(sourceId: string) {
    if (sourceId !== source.sourceId) return;
    if ([...states.values()].some((game) => game.operation?.status === 'running')) throw { code: 'operation_in_progress', message: 'The source has active operations.', details: {} };
    if (['runtime-demo', 'overlay-demo', 'coop-demo'].some((id) => state(sourceId, id).installed)) throw { code: 'snapshot_conflict', message: "Remove the source's installed games before disconnecting it.", details: {} };
    connected = false;
  },
  async state(sourceId: string, gameId: string) { return structuredClone(state(sourceId, gameId)); },
  async setOptions(sourceId: string, gameId: string, options: FusionGameOptions) {
    const game = state(sourceId, gameId);
    game.options = structuredClone(options);
    game.addons.forEach((addon) => { addon.enabled = options.enabledAddons?.includes(addon.id) ?? addon.enabled; });
    return structuredClone(game);
  },
  async install(sourceId: string, gameId: string, input: FusionInstallInput) {
    const game = state(sourceId, gameId);
    if (game.blockers.length) throw game.blockers[0];
    if (game.userLayers.some((layer) => !input.userFiles[layer.layerId])) throw { code: 'missing_layer', message: 'Select the required user files before installing.', details: {} };
    const operationId = `preview-operation-${++attempt}`;
    const event = { operationId, sourceId, gameId, layerId: null, phase: 'fetch', sequence: 1, status: 'running', error: null } satisfies FusionOperationProgress;
    emit(event);
    const completion = new Promise<FusionOperationProgress>((resolve) => completions.set(operationId, resolve));
    setTimeout(() => {
      if (game.operation?.operationId !== operationId || game.operation.status !== 'running') return;
      emit({ ...event, phase: 'transform', layerId: 'runtime', sequence: 2 });
    }, 350);
    setTimeout(() => {
      if (game.operation?.operationId !== operationId || game.operation.status !== 'running') return;
      game.installed = true; game.activeVersion = game.version; game.updateAvailable = false;
      game.rollbackAvailable = true; game.activeSnapshotId = `snapshot-${attempt}`;
      game.ready = game.executables.every((exe) => exe.trusted);
      finish({ ...event, phase: 'complete', sequence: 3, status: 'completed' });
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
    game.installed = false; game.ready = false; game.activeVersion = null; game.activeSnapshotId = null; game.rollbackAvailable = false;
    const event = { operationId: `preview-remove-${++attempt}`, sourceId, gameId, layerId: null, phase: 'completed', sequence: 1, status: 'completed', error: null } satisfies FusionOperationProgress;
    emit(event); return event;
  },
  async trust(sourceId: string, gameId: string, path: string, blake3: string) {
    const game = state(sourceId, gameId);
    const executable = game.executables.find((exe) => exe.path === path && exe.blake3 === blake3);
    if (!executable) throw { code: 'integrity_violation', message: 'The executable hash changed. Reload the game details.', details: {} };
    executable.trusted = true; game.ready = game.installed && !game.blockers.length && game.executables.every((exe) => exe.trusted);
  },
  async crash(reportId: string): Promise<FusionCrashReport> {
    if (!reportIds.has(reportId)) throw new Error('Report was discarded.');
    return { reportId, report: { schemaVersion: '1.0', game: { sourceId: source.sourceId, gameId: 'runtime-demo' }, layerHashes: { runtime: 'a'.repeat(64) }, runtime: { channel: 'pinned', version: '1.0.0', executableBlake3: 'a'.repeat(64) }, enabledAddons: ['shader-cache'], exitCode: 1, cleanLogTail: 'Runtime exited with code 1.\nWorkspace: <WORKSPACE_ROOT>\nContact: <EMAIL>\nToken: <TOKEN>', crashSignatureHash: 'b'.repeat(64) }, status: 'pending', endpoint: null, canSend: false };
  },
  async sendCrash(_reportId: string, _expectedEndpoint: string): Promise<void> { throw new Error('No telemetry endpoint is configured for this preview source.'); },
  async discardCrash(reportId: string) { reportIds.delete(reportId); states.forEach((game) => { game.crashReportIds = game.crashReportIds.filter((id) => id !== reportId); }); },
  subscribe(listener: (event: FusionOperationProgress) => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
  reset() { connected = false; states.clear(); completions.clear(); reportIds.add('preview-crash'); }
};

export { fusionGameKey };
