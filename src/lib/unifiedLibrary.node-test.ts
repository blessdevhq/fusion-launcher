import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { buildGameLibraryItem } from './libraryStatus.ts';
import { DEFAULT_SETTINGS } from './settings.ts';
import { buildUnifiedLibraryItems, countUnifiedLibraryItems, libraryPlatformLabel, selectUnifiedLibraryItems } from './unifiedLibrary.ts';
import { fusionGameKey, type FusionCatalogEntry, type FusionOperationProgress } from '../types/fusionSource.ts';

function fusion(gameId: string, overrides: Partial<FusionCatalogEntry> = {}): FusionCatalogEntry {
  return {
    kind: 'fusion_source', sourceId: 'source', sourceName: 'Fusion Source', gameId,
    title: gameId, version: '1.0.0', installed: false, activeVersion: null,
    updateAvailable: false, platform: 'pc', coverUrl: null, activeSnapshotId: null, readyToPlay: false,
    ...overrides
  };
}

function legacy(id: string, title: string, platform = 'nes', configured = true) {
  return buildGameLibraryItem({
    id, title, platform, repositoryId: 'legacy', repositoryName: 'Legacy Source', sourceId: id,
    downloads: [], expectedExtensions: [], requiredSystemFileIds: []
  }, {
    gameId: id, installed: true, systemRequirementsReady: true, missingRequirements: [], download: null
  }, { ...DEFAULT_SETTINGS, emulators: configured ? { [platform]: 'C:/Emulators/run.exe' } : {}, emulatorConfigs: {} });
}

describe('unified library browsing', () => {
  it('uses the same predicates for mixed collection counts and filtering', () => {
    const installing = fusion('Installing');
    const operations = { [fusionGameKey(installing)]: {
      operationId: 'op', sourceId: 'source', gameId: installing.gameId, layerId: 'runtime',
      phase: 'fetch', status: 'cancelling', sequence: 3, error: null,
      transfer: { downloadedBytes: 25, totalBytes: 100, speedBytesPerSecond: 10 }
    } satisfies FusionOperationProgress };
    const items = buildUnifiedLibraryItems([
      legacy('ready', 'Ready legacy'), legacy('setup', 'Needs emulator', 'switch', false)
    ], [
      fusion('Ready Fusion', { installed: true, activeSnapshotId: 'snapshot', activeVersion: '1.0.0', readyToPlay: true }),
      fusion('Not installed', { platform: 'Switch' }),
      fusion('Needs trust', { installed: true, activeSnapshotId: 'untrusted', activeVersion: '1.0.0' }),
      installing
    ], operations);
    const counts = countUnifiedLibraryItems(items);
    assert.equal(counts.all, 6);
    assert.equal(counts.ready, 2);
    assert.equal(counts.downloading, 1);
    assert.equal(counts.missing, 2);
    for (const [filter, count] of [['all', counts.all], ['ready', counts.ready], ['downloading', counts.downloading], ['missing', counts.missing]] as const) {
      assert.equal(selectUnifiedLibraryItems(items, filter, '', 'title').length, count);
    }
    assert.equal(selectUnifiedLibraryItems(items, 'installed', '', 'title').length, 4);
    assert.deepEqual(selectUnifiedLibraryItems(items, 'all', '', 'status').slice(0, 2).map((item) => item.title), ['Ready Fusion', 'Ready legacy']);
    for (const { platform, count } of counts.platforms) {
      assert.equal(selectUnifiedLibraryItems(items, 'all', '', 'title', platform).length, count);
    }
    assert.equal(items.find((item) => item.title === 'Installing')?.progressPercent, 25);
    assert.equal(selectUnifiedLibraryItems(items, 'missing', 'Not installed', 'title').length, 0);
  });

  it('keeps source-qualified Fusion identity and legacy routing distinct', () => {
    const first = fusion('same', { title: 'A' });
    const second = fusion('same', { sourceId: 'other', title: 'B' });
    const items = buildUnifiedLibraryItems([legacy('source::same', 'C')], [first, second]);
    assert.equal(new Set(items.map((item) => item.key)).size, 3);
    assert.equal(new Set(items.map((item) => item.focusId)).size, 3);
    assert.equal(items[0].focusId, 'game:library:source%3A%3Asame');
    assert.equal(items[1].focusId, 'fusion-game:source%3A%3Asame');
    assert.equal(items[2].focusId, 'fusion-game:other%3A%3Asame');
    assert.equal(items[0].kind, 'legacy');
    assert.equal(items[1].kind, 'fusion_source');
  });

  it('sorts both domains together and searches source and normalized platform metadata', () => {
    const items = buildUnifiedLibraryItems([legacy('legacy', 'Zelda', 'switch')], [
      fusion('a', { title: 'Alpha', platform: 'PC' }),
      fusion('b', { title: 'Beta', sourceName: 'Another source', platform: 'Nintendo Switch' }),
      fusion('unknown', { title: 'Unknown platform', platform: null })
    ]);
    assert.deepEqual(selectUnifiedLibraryItems(items, 'all', '', 'title').map((item) => item.title), ['Alpha', 'Beta', 'Unknown platform', 'Zelda']);
    assert.deepEqual(selectUnifiedLibraryItems(items, 'all', '', 'platform').map((item) => item.title), ['Unknown platform', 'Alpha', 'Beta', 'Zelda']);
    assert.deepEqual(selectUnifiedLibraryItems(items, 'all', '', 'repository').map((item) => item.title), ['Beta', 'Alpha', 'Unknown platform', 'Zelda']);
    assert.deepEqual(selectUnifiedLibraryItems(items, 'all', 'PC fusion', 'title').map((item) => item.title), ['Alpha']);
    assert.deepEqual(selectUnifiedLibraryItems(items, 'all', 'switch', 'title', 'Switch').map((item) => item.title), ['Beta', 'Zelda']);
    assert.equal(countUnifiedLibraryItems(items).platforms.length, 2);
    assert.equal(libraryPlatformLabel('PC'), 'PC');
    assert.equal(libraryPlatformLabel('Switch'), 'Nintendo Switch');
  });

  it('downgrades known blocked snapshots without disabling a ready installation after a failed update', () => {
    const game = fusion('ready', { installed: true, readyToPlay: true, activeSnapshotId: 'snapshot', updateAvailable: true });
    const failed = { [fusionGameKey(game)]: { operationId: 'op', sourceId: 'source', gameId: 'ready', layerId: null, phase: 'fetch', status: 'failed', sequence: 9, error: null } };
    const ready = buildUnifiedLibraryItems([], [game], failed);
    assert.equal(selectUnifiedLibraryItems(ready, 'ready', '', 'status').length, 1);
    assert.equal(ready[0].statusLabel, 'Update Available');
    const blocked = buildUnifiedLibraryItems([], [game], failed, new Set([fusionGameKey(game)]));
    assert.equal(selectUnifiedLibraryItems(blocked, 'ready', '', 'status').length, 0);
    assert.equal(selectUnifiedLibraryItems(blocked, 'missing', '', 'status').length, 1);
  });
});
