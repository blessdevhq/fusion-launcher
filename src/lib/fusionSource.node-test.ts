import assert from 'node:assert/strict';
import { afterEach, beforeEach, describe, it } from 'node:test';
import { api } from './api.ts';
import { fusionPreviewApi, FUSION_PREVIEW_URL } from './fusionPreviewData.ts';
import { selectFusionCatalog } from './fusionCatalog.ts';
import type { FusionCatalogEntry, FusionOperationProgress } from '../types/fusionSource.ts';

describe('Fusion IPC contracts and source routing', () => {
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  let response: (command: string, args: Record<string, unknown>) => unknown;
  beforeEach(() => {
    calls.length = 0;
    response = () => ({});
    Object.defineProperty(globalThis, 'window', { configurable: true, value: {
      __TAURI_INTERNALS__: { invoke: async (command: string, args: Record<string, unknown>) => { calls.push({ command, args }); return response(command, args); } }
    } });
  });
  afterEach(() => { Reflect.deleteProperty(globalThis, 'window'); });

  it('adds a Fusion index without fetching game manifests or starting installation', async () => {
    response = () => ({ sourceId: 'source', name: 'Source', url: 'https://example.test/index.json', gameCount: 50000, publicKey: null });
    const added = await api.addAnySource('https://example.test/index.json');
    assert.equal(added.kind, 'fusion_source');
    assert.deepEqual(calls.map((call) => call.command), ['fusion_source_preview', 'fusion_source_add']);
    assert.deepEqual(calls[0].args, { url: 'https://example.test/index.json' });
  });

  it('passes explicit folder authority and expected identity through dedicated local IPC', async () => {
    await api.fusionSourcePreviewLocal('F:/Sources/release-one');
    await api.fusionSourceAddLocal('F:/Sources/release-two', 'owned', true);
    assert.deepEqual(calls, [
      { command: 'fusion_source_preview_local', args: { directory: 'F:/Sources/release-one' } },
      { command: 'fusion_source_add_local', args: { directory: 'F:/Sources/release-two', expectedSourceId: 'owned', prepareEden: true } }
    ]);
  });

  it('falls back to legacy only on an explicitly unsupported source schema', async () => {
    response = (command) => {
      if (command === 'fusion_source_preview') throw { code: 'unsupported_schema', message: 'Legacy source', details: {} };
      return { name: 'Legacy source' };
    };
    const result = await api.previewAnySource('https://example.test/legacy.json');
    assert.equal(result.kind, 'legacy');
    assert.deepEqual(calls.map((call) => call.command), ['fusion_source_preview', 'preview_source']);
  });

  it('preserves transport and validation failures without trying another parser', async () => {
    for (const code of ['network', 'invalid_digest', 'invalid_uri', 'invalid_json']) {
      calls.length = 0;
      response = () => { throw { code, message: 'Rejected', details: {} }; };
      await assert.rejects(api.previewAnySource('https://example.test/source.json'), (error: unknown) => typeof error === 'object' && error !== null && 'code' in error && error.code === code);
      assert.deepEqual(calls.map((call) => call.command), ['fusion_source_preview']);
    }
  });

  it('keeps legacy pasted JSON supported and explains how to host a Fusion index', async () => {
    await api.previewAnySource('{"manifest_version":"3"}');
    assert.deepEqual(calls.map((call) => call.command), ['preview_source']);
    await assert.rejects(api.previewAnySource('{"schema_version":"1.0","source_info":{}}'), /HTTPS URL/);
    assert.equal(calls.length, 1);
  });

  it('passes qualified game identities and explicit raw destinations independently of legacy APIs', async () => {
    const request = { userFiles: { base: 'D:/Owned/base.bin' }, rawPaths: { base: 'game/base.bin' }, externalWorkspaceRoot: null };
    await api.fusionGameInstall('source', 'game', request);
    await api.fusionGameSetOptions('source', 'game', { enabledAddons: [], runtimeChannel: 'dynamic' });
    await api.fusionGameTrustExecutable('source', 'game', 'runtime/run.exe', 'a'.repeat(64));
    assert.deepEqual(calls[0], { command: 'fusion_game_install', args: { game: { sourceId: 'source', gameId: 'game' }, request } });
    assert.deepEqual(calls[1].args, { game: { sourceId: 'source', gameId: 'game' }, options: { enabledAddons: [], runtimeChannel: 'dynamic' } });
    assert.equal(calls[2].args.blake3, 'a'.repeat(64));
    assert.equal(calls.some((call) => call.command === 'install_game'), false);
  });

  it('requests backup metadata by qualified game identity alone', async () => {
    const inventory = { snapshots: [{ id: 'backup', createdAt: '2026-10-18T10:00:00Z', fileCount: 2, totalBytes: 2048 }], skippedCount: 1, hasMore: false };
    response = () => inventory;
    assert.deepEqual(await api.fusionGameListSaveSnapshots('source', 'game'), inventory);
    assert.deepEqual(calls, [{ command: 'fusion_game_list_save_snapshots', args: { game: { sourceId: 'source', gameId: 'game' } } }]);
  });

  it('creates a local backup by qualified game identity alone and returns terminal progress', async () => {
    const completed = { operationId: 'backup', sourceId: 'source', gameId: 'game', layerId: null, phase: 'complete', sequence: 2, status: 'completed', error: null } satisfies FusionOperationProgress;
    response = () => completed;
    assert.deepEqual(await api.fusionGameCreateSaveSnapshot('source', 'game'), completed);
    assert.deepEqual(calls, [{ command: 'fusion_game_create_save_snapshot', args: { game: { sourceId: 'source', gameId: 'game' } } }]);
  });

  it('binds the explicit crash send action to the reviewed endpoint', async () => {
    await api.fusionCrashPreview('report');
    assert.deepEqual(calls.map((call) => call.command), ['fusion_crash_preview']);
    await api.fusionCrashSend('report', 'https://reports.example.test/submit');
    assert.deepEqual(calls[1], { command: 'fusion_crash_send', args: { reportId: 'report', expectedEndpoint: 'https://reports.example.test/submit' } });
  });
});

describe('Fusion browser preview', () => {
  beforeEach(() => fusionPreviewApi.reset());
  it('provides demo backup metadata only for the matching installation', async () => {
    const inventory = await fusionPreviewApi.listSaveSnapshots('fusion-preview', 'runtime-demo');
    assert.deepEqual(inventory.snapshots.map(({ fileCount, totalBytes }) => [fileCount, totalBytes]), [[3, 4096], [2, 2048]]);
    assert.deepEqual(await fusionPreviewApi.listSaveSnapshots('fusion-preview', 'repair-demo'), { snapshots: [], skippedCount: 0, hasMore: false });
    await fusionPreviewApi.removeGame('fusion-preview', 'runtime-demo');
    assert.deepEqual((await fusionPreviewApi.listSaveSnapshots('fusion-preview', 'runtime-demo')).snapshots, []);
  });

  it('publishes only simulated metadata after a qualified preview backup completes', async () => {
    const snapshots = () => fusionPreviewApi.listSaveSnapshots('fusion-preview', 'runtime-demo');
    const before = await snapshots();
    const backup = fusionPreviewApi.createSaveSnapshot('fusion-preview', 'runtime-demo');
    assert.deepEqual(await snapshots(), before);
    await assert.rejects(fusionPreviewApi.createSaveSnapshot('fusion-preview', 'runtime-demo'), { code: 'operation_in_progress' });
    assert.equal((await backup).status, 'completed');
    const after = await snapshots();
    const added = after.snapshots.filter(({ id }) => !before.snapshots.some((snapshot) => snapshot.id === id));
    assert.equal(added.length, 1);
    assert.deepEqual(Object.keys(added[0]).sort(), ['createdAt', 'fileCount', 'id', 'totalBytes']);
    assert.deepEqual((await fusionPreviewApi.listSaveSnapshots('fusion-preview', 'repair-demo')).snapshots, []);
    await assert.rejects(fusionPreviewApi.createSaveSnapshot('other-source', 'runtime-demo'), { code: 'missing_source' });
  });

  it('does not publish preview metadata when a backup is cancelled', async () => {
    const before = await fusionPreviewApi.listSaveSnapshots('fusion-preview', 'runtime-demo');
    const backup = fusionPreviewApi.createSaveSnapshot('fusion-preview', 'runtime-demo');
    await fusionPreviewApi.cancel('fusion-preview', 'runtime-demo');
    assert.equal((await backup).status, 'cancelled');
    assert.deepEqual(await fusionPreviewApi.listSaveSnapshots('fusion-preview', 'runtime-demo'), before);
  });
  it('exposes a disabled optional input before options are saved', async () => {
    const game = await fusionPreviewApi.state('fusion-preview', 'coop-demo');
    const optional = game.userLayers.find((layer) => layer.layerId === 'coop');
    assert.equal(optional?.optional, true);
    assert.equal(optional?.title, 'Cooperative content');
    assert.equal(game.addons.find((addon) => addon.id === 'coop')?.enabled, false);
  });
  it('does not require a disabled optional input during preview installation', async () => {
    const pending = fusionPreviewApi.install('fusion-preview', 'coop-demo', { userFiles: { base: 'preview://owned/' }, rawPaths: {}, externalWorkspaceRoot: null });
    await fusionPreviewApi.cancel('fusion-preview', 'coop-demo');
    assert.equal((await pending).status, 'cancelled');
  });
  it('connecting a source preserves existing state and does not install a user-file game', async () => {
    const before = await fusionPreviewApi.state('fusion-preview', 'coop-demo');
    assert.deepEqual(await fusionPreviewApi.catalog(), []);
    await fusionPreviewApi.add(FUSION_PREVIEW_URL);
    const after = await fusionPreviewApi.state('fusion-preview', 'coop-demo');
    assert.equal(after.installed, false);
    assert.equal(after.operation, null);
    assert.deepEqual(after, before);
    assert.equal((await fusionPreviewApi.catalog()).length, 4);
    assert.equal((await fusionPreviewApi.state('fusion-preview', 'overlay-demo')).blockers[0].code, 'missing_capability');
  });
  it('requires exact hash approval and provides only an already cleaned report to the UI', async () => {
    await assert.rejects(fusionPreviewApi.trust('fusion-preview', 'coop-demo', 'runtime/engine.exe', '0'.repeat(64)));
    const report = await fusionPreviewApi.crash('preview-crash');
    assert.match(report.report.cleanLogTail, /<WORKSPACE_ROOT>/);
    assert.equal(report.canSend, false);
    await fusionPreviewApi.discardCrash('preview-crash');
    assert.deepEqual((await fusionPreviewApi.state('fusion-preview', 'runtime-demo')).crashReportIds, []);
  });
});

describe('Fusion compact catalog selection', () => {
  const game = (gameId: string, title: string, installed: boolean): FusionCatalogEntry => ({ kind: 'fusion_source', sourceId: 'source', sourceName: 'Source', gameId, title, installed, version: '1.0.0', activeVersion: installed ? '1.0.0' : null, updateAvailable: false, platform: 'pc', coverUrl: null, activeSnapshotId: installed ? 'snapshot' : null, readyToPlay: installed });
  const entries = [game('a', 'Zebra', false), game('b', 'Orbit', true)];
  const operations = { 'source::a': { operationId: 'op', sourceId: 'source', gameId: 'a', layerId: 'base', phase: 'fetch', status: 'running', sequence: 1, error: null } satisfies FusionOperationProgress };
  it('filters Fusion games by search, install state and their own operation stream', () => {
    assert.deepEqual(selectFusionCatalog(entries, 'all', 'orbit', 'title', operations).map((entry) => entry.gameId), ['b']);
    assert.deepEqual(selectFusionCatalog(entries, 'installed', '', 'title', operations).map((entry) => entry.gameId), ['b']);
    assert.deepEqual(selectFusionCatalog(entries, 'downloading', '', 'title', operations).map((entry) => entry.gameId), ['a']);
    assert.deepEqual(selectFusionCatalog(entries, 'missing', '', 'title', operations).map((entry) => entry.gameId), []);
  });
});
