import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { effectiveFusionOptions, formatFusionBytes, fusionHasVersionUpdate, fusionIsIntegrityError, fusionLayerTitle, fusionOperationVisible, fusionProgressRatio, fusionSettingsDiffer, fusionStructuredError, selectableFusionAddons } from './fusionGamePresentation.ts';
import { fusionText } from './fusionText.ts';
import type { FusionGameState, FusionOperationProgress } from '../types/fusionSource.ts';

function fixture(): FusionGameState {
  return {
    sourceId: 'source', gameId: 'game', title: 'Game', version: '1.0.0', platform: 'pc',
    installed: true, activeVersion: '1.0.0', ready: true, updateAvailable: false,
    activeSnapshotId: 'snapshot', rollbackAvailable: false,
    options: { enabledAddons: null, runtimeChannel: null },
    appliedOptions: { enabledAddons: ['coop', '60fps'], runtimeChannel: 'dynamic' },
    defaultRuntimeChannel: 'dynamic', externalWorkspaceRoot: null,
    addons: [
      { id: '60fps', title: null, description: null, kind: 'addon', enabled: true },
      { id: 'coop', title: 'Cooperative mode', description: null, kind: 'mod', enabled: true },
      { id: 'patch', title: null, description: null, kind: 'patch', enabled: true },
    ],
    runtimeChannels: ['pinned', 'dynamic'], userLayers: [], executables: [], blockers: [], warnings: [],
    operation: null, crashReportIds: [], crashReports: [],
  };
}

describe('Fusion applied settings', () => {
  it('resolves null selections against effective defaults instead of treating them as changes', () => {
    const state = fixture();
    assert.deepEqual(effectiveFusionOptions(state), { enabledAddons: ['60fps', 'coop'], runtimeChannel: 'dynamic' });
    assert.equal(fusionSettingsDiffer(state, state.options, null), false);
    assert.equal(fusionSettingsDiffer(state, { enabledAddons: ['coop', '60fps', 'coop'], runtimeChannel: 'dynamic' }, ''), false);
    assert.deepEqual(selectableFusionAddons(state.addons).map((addon) => addon.id), ['60fps', 'coop']);
  });

  it('stays dirty when desired options were saved but a cancelled or failed rebuild retained the old snapshot', () => {
    const state = fixture();
    state.options = { enabledAddons: ['coop'], runtimeChannel: 'pinned' };
    assert.equal(fusionSettingsDiffer(state, state.options, null), true);
    state.appliedOptions = { ...state.options };
    assert.equal(fusionSettingsDiffer(state, state.options, null), false);
    assert.equal(fusionSettingsDiffer(state, state.options, 'D:/Workspace'), true);
  });

  it('compares uninstalled drafts with saved preferences and supports games without a runtime', () => {
    const state = fixture();
    state.installed = false; state.appliedOptions = null;
    state.options = { enabledAddons: [], runtimeChannel: null };
    state.defaultRuntimeChannel = null; state.runtimeChannels = [];
    assert.equal(fusionSettingsDiffer(state, state.options, null), false);
    assert.equal(fusionSettingsDiffer(state, { enabledAddons: ['coop'], runtimeChannel: null }, null), true);
  });

  it('shows a source update only for a changed installed version, independently from rebuild-needed metadata', () => {
    const state = fixture();
    state.updateAvailable = true;
    assert.equal(fusionHasVersionUpdate(state), false);
    state.version = '1.0.1';
    assert.equal(fusionHasVersionUpdate(state), true);
    state.installed = false;
    assert.equal(fusionHasVersionUpdate(state), false);
  });
});

describe('Fusion operation and integrity presentation', () => {
  const operation = (status: string): FusionOperationProgress => ({ operationId: 'op', sourceId: 'source', gameId: 'game', layerId: null, phase: 'fetch', status, sequence: 1, error: null });

  it('shows active and failed operations while hiding successful and cancelled history', () => {
    for (const status of ['running', 'cancelling', 'failed']) assert.equal(fusionOperationVisible(operation(status)), true);
    for (const status of ['completed', 'cancelled']) assert.equal(fusionOperationVisible(operation(status)), false);
    assert.equal(fusionOperationVisible(null), false);
    assert.equal(fusionText('ru').operationPhase('resolveRuntime'), 'Выбор версии runtime');
    assert.equal(fusionText('ru').operationPhase('cancelling'), 'Отмена операции');
    assert.equal(fusionText('en').operationPhase('future_internal_phase'), 'Preparing operation');
  });

  it('uses bounded transfer progress and handles unknown totals or invalid samples', () => {
    assert.equal(fusionProgressRatio({ downloadedBytes: 150, totalBytes: 100, speedBytesPerSecond: 1 }), 1);
    assert.equal(fusionProgressRatio({ downloadedBytes: -5, totalBytes: 100, speedBytesPerSecond: 1 }), 0);
    assert.equal(fusionProgressRatio({ downloadedBytes: 5, totalBytes: 0, speedBytesPerSecond: 1 }), null);
    assert.equal(fusionProgressRatio({ downloadedBytes: 5, totalBytes: Number.NaN, speedBytesPerSecond: 1 }), null);
    assert.equal(formatFusionBytes(Number.POSITIVE_INFINITY, 'en'), '0 B');
    assert.equal(formatFusionBytes(1024, 'ru'), '1 КБ');
    assert.equal(formatFusionBytes(0.5, 'en'), '1 B');
  });

  it('retains structured guard errors and recognizes repair separately from exact-hash trust', () => {
    const error = { code: 'corrupted_critical_binary', message: 'Changed', details: { path: 'runtime/core.dll' } };
    assert.deepEqual(fusionStructuredError(error), error);
    assert.equal(fusionIsIntegrityError(error), true);
    assert.equal(fusionIsIntegrityError({ code: 'missing_injector_dll' }), true);
    assert.equal(fusionIsIntegrityError({ code: 'untrusted_executable' }), false);
    assert.equal(fusionStructuredError('Internal error'), null);
  });

  it('prefers publisher titles and formats legacy layer IDs without raw addon labels', () => {
    assert.equal(fusionLayerTitle({ id: 'addon-60fps' }), '60 FPS');
    assert.equal(fusionLayerTitle({ id: 'addon-crash_test' }), 'Crash Test');
    assert.equal(fusionLayerTitle({ id: 'addon-60fps', title: 'Плавность 60 FPS' }), 'Плавность 60 FPS');
    assert.equal(fusionText('ru').verifiedFiles(2), 'Проверено и одобрено: 2 исполняемых файла');
    assert.equal(fusionText('ru').verifiedFiles(11), 'Проверено и одобрено: 11 исполняемых файлов');
  });
});
