import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { buildGameLibraryItems, buildGameLibraryItem, searchAndSortLibraryItems } from './libraryStatus.ts';

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

const secondGame = {
  ...game,
  id: 'repo::second',
  sourceId: 'second',
  title: 'Alpha Quest',
  platform: 'gba'
};

const configuredSettings = {
  emulators: {
    nes: 'C:/Emulators/nes.exe'
  },
  emulatorConfigs: {},
  language: 'en'
};

const emptySettings = {
  emulators: {},
  emulatorConfigs: {},
  language: 'en'
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

  it('marks a completed direct download as ready to play', () => {
    const item = buildGameLibraryItem(
      {
        ...game,
        downloads: [{
          kind: 'bundled',
          path: 'demo-content/fusion-launcher-smoke.nes',
          sha256: 'a'.repeat(64),
          sizeBytes: 24592
        }]
      },
      status({
        download: download({
          magnetUri: 'direct:bundled',
          saveDir: 'F:/Games/fusion-launcher-smoke.nes',
          status: 'completed',
          progressPercent: 100,
          downloadedBytes: 24592,
          totalBytes: 24592,
          peersCount: 0,
          torrentId: null,
          completedAt: '2026-05-23T00:05:00Z'
        })
      }),
      configuredSettings
    );

    assert.equal(item.installed, true);
    assert.equal(item.readyToPlay, true);
    assert.equal(item.primaryAction, 'play');
  });

  it('marks an installed game without an emulator as missing requirements', () => {
    const item = buildGameLibraryItem(game, status({ installed: true }), emptySettings);

    assert.equal(item.readyToPlay, false);
    assert.equal(item.primaryAction, 'details');
    assert.equal(item.statusLabel, 'Missing Requirements');
    assert.deepEqual(item.missingRequirements, ['Set up NES / Famicom emulator']);
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

  it('makes corrupt downloaded game files re-downloadable instead of playable', () => {
    const item = buildGameLibraryItem(
      game,
      status({
        installed: true,
        systemRequirementsReady: false,
        missingRequirements: ['Game file: NES game file does not contain a valid iNES header'],
        download: download({ status: 'completed', progressPercent: 100, completedAt: '2026-05-23T00:05:00Z' })
      }),
      configuredSettings
    );

    assert.equal(item.readyToPlay, false);
    assert.equal(item.primaryAction, 'download');
    assert.equal(item.primaryActionLabel, 'Re-download');
    assert.equal(item.statusLabel, 'Game File Issue');
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

  it('searches library by title, platform, repository, source id, and status text', () => {
    const items = buildGameLibraryItems(
      [game, secondGame],
      [
        status({ installed: true }),
        {
          ...status({
            gameId: 'repo::second',
            download: download({ gameId: 'repo::second', status: 'paused' })
          })
        }
      ],
      configuredSettings
    );

    assert.deepEqual(
      searchAndSortLibraryItems(items, 'all', 'alpha', 'title').map((item) => item.game.id),
      ['repo::second']
    );
    assert.deepEqual(
      searchAndSortLibraryItems(items, 'all', 'gba', 'title').map((item) => item.game.id),
      ['repo::second']
    );
    assert.deepEqual(
      searchAndSortLibraryItems(items, 'all', 'paused', 'title').map((item) => item.game.id),
      ['repo::second']
    );
    assert.deepEqual(
      searchAndSortLibraryItems(items, 'all', 'demo repo', 'title').map((item) => item.game.id),
      ['repo::second', 'repo::game']
    );
  });

  it('applies library filters before search and sorts status by actionability', () => {
    const items = buildGameLibraryItems(
      [game, secondGame, { ...game, id: 'repo::third', sourceId: 'third', title: 'Beta Run' }],
      [
        status({
          installed: true,
          download: download({ status: 'completed', progressPercent: 100, completedAt: '2026-05-23T00:05:00Z' })
        }),
        {
          ...status({
            gameId: 'repo::second',
            download: download({ gameId: 'repo::second', status: 'downloading' })
          })
        },
        {
          ...status({
            gameId: 'repo::third',
            download: download({ gameId: 'repo::third', status: 'error', errorMessage: 'No peers' })
          })
        }
      ],
      configuredSettings
    );

    assert.deepEqual(
      searchAndSortLibraryItems(items, 'downloading', '', 'status').map((item) => item.game.id),
      ['repo::second', 'repo::third']
    );
    assert.deepEqual(
      searchAndSortLibraryItems(items, 'installed', 'alpha', 'title').map((item) => item.game.id),
      []
    );
  });
});
