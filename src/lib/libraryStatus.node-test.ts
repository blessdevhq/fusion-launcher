import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { buildGameLibraryItem } from './libraryStatus.ts';

const game = {
  id: 'repo::game',
  sourceId: 'game',
  repositoryId: 'repo',
  repositoryName: 'Demo Repo',
  platform: 'nes',
  title: 'Demo Game',
  downloads: [{ kind: 'magnet', uri: 'magnet:?xt=urn:btih:abc' }],
  expectedExtensions: ['.nes'],
  requiredSystemFileIds: []
};

const configuredSettings = {
  emulators: {
    nes: 'C:/Emulators/nes.exe'
  },
  emulatorConfigs: {}
};

const emptySettings = {
  emulators: {},
  emulatorConfigs: {}
};

function status(overrides = {}) {
  return {
    gameId: 'repo::game',
    installed: false,
    systemRequirementsReady: true,
    missingRequirements: [],
    download: null,
    ...overrides
  };
}

function download(overrides = {}) {
  return {
    gameId: 'repo::game',
    magnetUri: 'magnet:?xt=urn:btih:abc',
    saveDir: 'F:/Games/game',
    status: 'downloading',
    progressPercent: 25,
    downloadedBytes: 25,
    totalBytes: 100,
    downloadSpeedBytesPerSec: 10,
    uploadSpeedBytesPerSec: 0,
    peersCount: 2,
    torrentId: 1,
    errorMessage: null,
    createdAt: '2026-05-23T00:00:00Z',
    updatedAt: '2026-05-23T00:00:00Z',
    completedAt: null,
    ...overrides
  };
}

describe('library status derivation', () => {
  it('marks a fully configured installed game as ready to play', () => {
    const item = buildGameLibraryItem(
      game,
      status({
        installed: true,
        download: download({ status: 'completed', progressPercent: 100, completedAt: '2026-05-23T00:05:00Z' })
      }),
      configuredSettings
    );

    assert.equal(item.readyToPlay, true);
    assert.equal(item.primaryAction, 'play');
    assert.equal(item.statusLabel, 'Ready to Play');
  });

  it('marks an installed game without an emulator as missing requirements', () => {
    const item = buildGameLibraryItem(game, status({ installed: true }), emptySettings);

    assert.equal(item.readyToPlay, false);
    assert.equal(item.primaryAction, 'details');
    assert.equal(item.statusLabel, 'Missing Requirements');
    assert.deepEqual(item.missingRequirements, ['Configure NES / Famicom emulator']);
  });

  it('preserves missing system file requirements', () => {
    const item = buildGameLibraryItem(
      game,
      status({
        installed: true,
        systemRequirementsReady: false,
        missingRequirements: ['Demo BIOS is not installed']
      }),
      configuredSettings
    );

    assert.equal(item.readyToPlay, false);
    assert.equal(item.primaryAction, 'details');
    assert.deepEqual(item.missingRequirements, ['Demo BIOS is not installed']);
  });

  it('marks active downloads as downloading', () => {
    const item = buildGameLibraryItem(
      game,
      status({ download: download({ status: 'downloading', progressPercent: 42 }) }),
      configuredSettings
    );

    assert.equal(item.isDownloading, true);
    assert.equal(item.progressPercent, 42);
    assert.equal(item.statusLabel, 'Downloading');
  });

  it('makes errored downloads retryable', () => {
    const item = buildGameLibraryItem(
      game,
      status({
        download: download({
          status: 'error',
          progressPercent: 18,
          errorMessage: 'No peers found'
        })
      }),
      configuredSettings
    );

    assert.equal(item.hasError, true);
    assert.equal(item.primaryAction, 'retry');
    assert.equal(item.statusLabel, 'Download Error');
  });
});
