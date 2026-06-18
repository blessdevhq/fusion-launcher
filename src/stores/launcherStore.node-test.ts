import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import type { DownloadProgressEvent, TorrentDownloadRecord } from '../types/repository.ts';
import { mergeDownloadEventIntoList, mergeDownloadSnapshotIntoList } from './launcherStore.ts';

const event: DownloadProgressEvent = {
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
      downloadRecord({
        gameId: 'repo::game',
        magnetUri: 'magnet:?xt=urn:btih:abc',
        saveDir: 'F:/Games/game',
        status: 'paused',
        progressPercent: 2,
        downloadedBytes: 2,
        torrentId: 7
      })
    ], event);

    assert.equal(downloads[0].status, 'downloading');
    assert.equal(downloads[0].magnetUri, 'magnet:?xt=urn:btih:abc');
    assert.equal(downloads[0].torrentId, 7);
  });

  it('does not move an existing download when a progress event arrives', () => {
    const downloads = mergeDownloadEventIntoList([
      downloadRecord({ gameId: 'repo::first' }),
      downloadRecord({ gameId: 'repo::game' }),
      downloadRecord({ gameId: 'repo::last' })
    ], event);

    assert.deepEqual(downloads.map((download) => download.gameId), [
      'repo::first',
      'repo::game',
      'repo::last'
    ]);
  });

  it('keeps torrent progress from moving backwards on stale events', () => {
    const downloads = mergeDownloadEventIntoList([
      downloadRecord({
        gameId: 'repo::game',
        progressPercent: 64,
        downloadedBytes: 64,
        totalBytes: 100
      })
    ], {
      ...event,
      progressPercent: 12,
      downloadedBytes: 12,
      totalBytes: 100
    });

    assert.equal(downloads[0].progressPercent, 64);
    assert.equal(downloads[0].downloadedBytes, 64);
  });

  it('preserves existing row order when a refreshed snapshot changes statuses', () => {
    const downloads = mergeDownloadSnapshotIntoList([
      downloadRecord({ gameId: 'repo::game', status: 'downloading' }),
      downloadRecord({ gameId: 'repo::error', status: 'error' })
    ], [
      downloadRecord({ gameId: 'repo::error', status: 'error' }),
      downloadRecord({ gameId: 'repo::game', status: 'paused' })
    ]);

    assert.deepEqual(downloads.map((download) => download.gameId), [
      'repo::game',
      'repo::error'
    ]);
    assert.equal(downloads[0].status, 'paused');
  });
});

function downloadRecord(overrides: Partial<TorrentDownloadRecord> = {}): TorrentDownloadRecord {
  return {
    gameId: 'repo::game',
    magnetUri: 'magnet:?xt=urn:btih:abc',
    saveDir: 'F:/Games/game',
    status: 'downloading',
    progressPercent: 42,
    downloadedBytes: 42,
    totalBytes: 100,
    downloadSpeedBytesPerSec: 0,
    uploadSpeedBytesPerSec: 0,
    peersCount: 0,
    torrentId: 7,
    errorMessage: null,
    createdAt: '2026-05-23T00:00:00.000Z',
    updatedAt: '2026-05-23T00:00:00.000Z',
    completedAt: null,
    ...overrides
  };
}
