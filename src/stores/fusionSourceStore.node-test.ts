import assert from 'node:assert/strict';
import { beforeEach, describe, it } from 'node:test';
import { useFusionSourceStore } from './fusionSourceStore.ts';
import { useLauncherStore } from './launcherStore.ts';
import type { FusionCatalogEntry, FusionGameState, FusionOperationProgress } from '../types/fusionSource.ts';

const event: FusionOperationProgress = { operationId: 'one', sourceId: 'source', gameId: 'game', layerId: 'runtime', phase: 'fetch', status: 'running', sequence: 1, error: null };
describe('Fusion operation identity and catalog isolation', () => {
  beforeEach(() => {
    useFusionSourceStore.setState({ operations: {}, retiredOperationIds: [], readinessBlockers: {}, catalog: [], sources: [] });
    useLauncherStore.setState({ catalog: [], fusionCatalog: [], catalogEntries: [], downloads: [] });
  });
  it('rejects stale sequence events and never revives a completed operation', () => {
    const merge = useFusionSourceStore.getState().mergeOperation;
    merge(event); merge({ ...event, sequence: 3, phase: 'complete', status: 'completed' });
    merge({ ...event, sequence: 2 }); merge({ ...event, sequence: 4 });
    assert.equal(useFusionSourceStore.getState().operations['source::game'].status, 'completed');
    assert.equal(useFusionSourceStore.getState().operations['source::game'].sequence, 3);
  });
  it('ignores late events from a superseded operation while retaining the next run', () => {
    const merge = useFusionSourceStore.getState().mergeOperation;
    merge(event); merge({ ...event, operationId: 'two' }); merge({ ...event, sequence: 900, status: 'failed' });
    assert.equal(useFusionSourceStore.getState().operations['source::game'].operationId, 'two');
    assert.deepEqual(useFusionSourceStore.getState().retiredOperationIds, ['one']);
    assert.deepEqual(useLauncherStore.getState().downloads, []);
  });
  it('uses distinct namespaces for the same game ID from different sources', () => {
    useFusionSourceStore.getState().mergeOperation(event);
    useFusionSourceStore.getState().mergeOperation({ ...event, sourceId: 'other', operationId: 'other' });
    assert.equal(Object.keys(useFusionSourceStore.getState().operations).length, 2);
  });
  it('retains layer transfer counters and ignores them after terminal completion', () => {
    const merge = useFusionSourceStore.getState().mergeOperation;
    const transfer = { downloadedBytes: 1024, totalBytes: 4096, speedBytesPerSecond: 512 };
    merge({ ...event, transfer });
    assert.deepEqual(useFusionSourceStore.getState().operations['source::game'].transfer, transfer);
    merge({ ...event, sequence: 2, status: 'completed', phase: 'completed', transfer: null });
    merge({ ...event, sequence: 3, transfer });
    assert.equal(useFusionSourceStore.getState().operations['source::game'].transfer, null);
    assert.equal(useFusionSourceStore.getState().operations['source::game'].status, 'completed');
    assert.deepEqual(useLauncherStore.getState().downloads, []);
  });
  it('represents a native Fusion title without adding a fabricated platform or download URL', () => {
    const game = { kind: 'fusion_source' as const, sourceId: 'source', sourceName: 'Source', gameId: 'game', title: 'Native Game', version: '1', installed: false, activeVersion: null, updateAvailable: false, platform: null, coverUrl: null, activeSnapshotId: null, readyToPlay: false };
    useLauncherStore.getState().setFusionCatalog([game]);
    const entry = useLauncherStore.getState().catalogEntries[0];
    assert.equal(entry.kind, 'fusion_source');
    assert.equal(entry.game.platform, null);
    assert.equal('downloads' in entry.game, false);
    assert.deepEqual(useLauncherStore.getState().catalog, []);
  });
  it('keeps known integrity failures scoped to the exact installed snapshot', () => {
    const catalog: FusionCatalogEntry = { kind: 'fusion_source', sourceId: 'source', sourceName: 'Source', gameId: 'game', title: 'Game', version: '1', installed: true, activeVersion: '1', updateAvailable: false, platform: 'pc', coverUrl: null, activeSnapshotId: 'snapshot-a', readyToPlay: true };
    const store = useFusionSourceStore.getState();
    store.setSourcesAndCatalog([], [catalog]);
    store.recordIntegrityError(catalog, 'snapshot-a', { code: 'corrupted_critical_binary' });
    assert.deepEqual(useFusionSourceStore.getState().readinessBlockers, { 'source::game': { activeSnapshotId: 'snapshot-a' } });
    store.setSourcesAndCatalog([], [catalog]);
    assert.equal(Object.keys(useFusionSourceStore.getState().readinessBlockers).length, 1);
    store.setSourcesAndCatalog([], [{ ...catalog, activeSnapshotId: 'snapshot-b' }]);
    assert.deepEqual(useFusionSourceStore.getState().readinessBlockers, {});
    store.recordIntegrityError(catalog, null, { code: 'corrupted_critical_binary' });
    store.recordIntegrityError(catalog, 'snapshot-b', { code: 'network' });
    assert.deepEqual(useFusionSourceStore.getState().readinessBlockers, {});
  });
  it('clears an integrity overlay only after a successful full state check or removal', () => {
    const store = useFusionSourceStore.getState();
    const game = { sourceId: 'source', gameId: 'game', installed: true, activeSnapshotId: 'snapshot-a', blockers: [{ code: 'missing_injector_dll', message: 'Missing', details: {} }] } as FusionGameState;
    store.recordGameState(game);
    assert.ok(useFusionSourceStore.getState().readinessBlockers['source::game']);
    store.recordGameState({ ...game, blockers: [] });
    assert.deepEqual(useFusionSourceStore.getState().readinessBlockers, {});
    store.recordGameState(game);
    store.setSourcesAndCatalog([], []);
    assert.deepEqual(useFusionSourceStore.getState().readinessBlockers, {});
  });
});
