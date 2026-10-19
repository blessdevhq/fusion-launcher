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

  it('binds the explicit crash send action to the reviewed endpoint', async () => {
    await api.fusionCrashPreview('report');
    assert.deepEqual(calls.map((call) => call.command), ['fusion_crash_preview']);
    await api.fusionCrashSend('report', 'https://reports.example.test/submit');
    assert.deepEqual(calls[1], { command: 'fusion_crash_send', args: { reportId: 'report', expectedEndpoint: 'https://reports.example.test/submit' } });
  });
});

describe('Fusion browser preview', () => {
  beforeEach(() => fusionPreviewApi.reset());
  it('connecting a source preserves existing state and does not install a user-file game', async () => {
    const before = await fusionPreviewApi.state('fusion-preview', 'coop-demo');
    assert.deepEqual(await fusionPreviewApi.catalog(), []);
    await fusionPreviewApi.add(FUSION_PREVIEW_URL);
    const after = await fusionPreviewApi.state('fusion-preview', 'coop-demo');
    assert.equal(after.installed, false);
    assert.equal(after.operation, null);
    assert.deepEqual(after, before);
    assert.equal((await fusionPreviewApi.catalog()).length, 3);
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
  const game = (gameId: string, title: string, installed: boolean): FusionCatalogEntry => ({ kind: 'fusion_source', sourceId: 'source', sourceName: 'Source', gameId, title, installed, version: '1.0.0', activeVersion: installed ? '1.0.0' : null, updateAvailable: false });
  const entries = [game('a', 'Zebra', false), game('b', 'Orbit', true)];
  const operations = { 'source::a': { operationId: 'op', sourceId: 'source', gameId: 'a', layerId: 'base', phase: 'fetch', status: 'running', sequence: 1, error: null } satisfies FusionOperationProgress };
  it('filters Fusion games by search, install state and their own operation stream', () => {
    assert.deepEqual(selectFusionCatalog(entries, 'all', 'orbit', 'title', operations).map((entry) => entry.gameId), ['b']);
    assert.deepEqual(selectFusionCatalog(entries, 'installed', '', 'title', operations).map((entry) => entry.gameId), ['b']);
    assert.deepEqual(selectFusionCatalog(entries, 'downloading', '', 'title', operations).map((entry) => entry.gameId), ['a']);
    assert.deepEqual(selectFusionCatalog(entries, 'missing', '', 'title', operations).map((entry) => entry.gameId), ['a']);
  });
});
