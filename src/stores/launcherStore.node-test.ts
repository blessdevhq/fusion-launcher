import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { mergeDownloadEventIntoList } from './launcherStore.ts';

const event = {
  gameId: 'repo::game',
  status: 'downloading',
  progress: 0.42,
  progressPercent: 42,
  downloadedBytes: 42,
  totalBytes: 100,
  downloadSpeedBytesPerSec: 2048,
  uploadSpeedBytesPerSec: 64,
  peersCount: 3,
  finished: false,
  saveDir: 'F:/Games/game',
  error: null
};

describe('launcher store download merging', () => {
  it('creates a download record when an event arrives first', () => {
    const downloads = mergeDownloadEventIntoList([], event);

    assert.equal(downloads.length, 1);
    assert.equal(downloads[0].gameId, 'repo::game');
    assert.equal(downloads[0].progressPercent, 42);
  });

  it('updates an existing download record', () => {
    const downloads = mergeDownloadEventIntoList([
      {
        gameId: 'repo::game',
        magnetUri: 'magnet:?xt=urn:btih:abc',
        saveDir: 'F:/Games/game',
        status: 'paused',
        progressPercent: 2,
        downloadedBytes: 2,
        totalBytes: 100,
        downloadSpeedBytesPerSec: 0,
        uploadSpeedBytesPerSec: 0,
        peersCount: 0,
        torrentId: 7,
        errorMessage: null,
        createdAt: '2026-05-23T00:00:00.000Z',
        updatedAt: '2026-05-23T00:00:00.000Z',
        completedAt: null
      }
    ], event);

    assert.equal(downloads[0].status, 'downloading');
    assert.equal(downloads[0].magnetUri, 'magnet:?xt=urn:btih:abc');
    assert.equal(downloads[0].torrentId, 7);
  });
});
