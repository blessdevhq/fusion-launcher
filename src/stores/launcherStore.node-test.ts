import assert from 'node:assert/strict';
import { beforeEach, describe, it } from 'node:test';
import type { DownloadProgressEvent, TorrentDownloadRecord } from '../types/repository.ts';
import { mergeDownloadEventIntoList, mergeDownloadSnapshotIntoList, useLauncherStore } from './launcherStore.ts';

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

describe('launcher store download generations', () => {
  beforeEach(() => {
    useLauncherStore.setState({ downloads: [], libraryStatuses: [], activityEvents: [], removedDownloads: [] });
  });

  it('does not resurrect removed A through late completed events or snapshots', () => {
    const store = useLauncherStore.getState();
    store.setDownloads([downloadRecord({ attemptId: 'A', runId: 1 })]);
    store.setLibraryStatuses([{ gameId: event.gameId, installed: false, systemRequirementsReady: true, missingRequirements: [], download: null }]);
    store.removeDownload({ gameId: event.gameId, attemptId: 'A' });
    store.mergeDownloadEvent({ ...event, attemptId: 'A', runId: 1, status: 'completed', finished: true });
    store.setDownloads([downloadRecord({ attemptId: 'A', runId: 1, status: 'completed' })]);
    store.setLibraryStatuses([{ gameId: event.gameId, installed: true, systemRequirementsReady: true, missingRequirements: [], download: downloadRecord({ attemptId: 'A' }) }]);
    const state = useLauncherStore.getState();
    assert.equal(state.downloads.length, 0);
    assert.equal(state.activityEvents.length, 0);
    assert.equal(state.libraryStatuses[0].installed, false);
    assert.equal(state.libraryStatuses[0].download, null);
  });

  it('accepts B after removing A and ignores late A progress and removal', () => {
    const store = useLauncherStore.getState();
    store.mergeDownloadEvent({ ...event, attemptId: 'A', runId: 1 });
    store.removeDownload({ gameId: event.gameId, attemptId: 'A' });
    store.mergeDownloadEvent({ ...event, attemptId: 'B', runId: 1, progressPercent: 3 });
    store.mergeDownloadEvent({ ...event, attemptId: 'A', runId: 10, progressPercent: 98 });
    store.removeDownload({ gameId: event.gameId, attemptId: 'A' });
    assert.equal(useLauncherStore.getState().downloads[0].attemptId, 'B');
    assert.equal(useLauncherStore.getState().downloads[0].progressPercent, 3);
  });

  it('ignores an older runtime after resume, including completed callbacks', () => {
    const store = useLauncherStore.getState();
    store.mergeDownloadEvent({ ...event, attemptId: 'A', runId: 1, status: 'paused' });
    store.mergeDownloadEvent({ ...event, attemptId: 'A', runId: 2, progressPercent: 50 });
    store.mergeDownloadEvent({ ...event, attemptId: 'A', runId: 1, status: 'completed', finished: true, progressPercent: 100 });
    const state = useLauncherStore.getState();
    assert.equal(state.downloads[0].runId, 2);
    assert.equal(state.downloads[0].status, 'downloading');
    assert.equal(state.downloads[0].progressPercent, 50);
    assert.equal(state.activityEvents.length, 0);
  });

  it('does not let progress or completion from the paused runtime undo pause', () => {
    const store = useLauncherStore.getState();
    store.setDownloads([downloadRecord({ attemptId: 'A', runId: 1, status: 'paused' })]);
    store.mergeDownloadEvent({ ...event, attemptId: 'A', runId: 1 });
    store.mergeDownloadEvent({ ...event, attemptId: 'A', runId: 1, status: 'completed', finished: true });
    assert.equal(useLauncherStore.getState().downloads[0].status, 'paused');
    assert.equal(useLauncherStore.getState().activityEvents.length, 0);
    store.mergeDownloadEvent({ ...event, attemptId: 'A', runId: 2 });
    assert.equal(useLauncherStore.getState().downloads[0].status, 'downloading');
  });

  it('retains event identity across legacy snapshots and then rejects older runtime', () => {
    const store = useLauncherStore.getState();
    store.mergeDownloadEvent({ ...event, attemptId: 'A', runId: 2 });
    store.setDownloads([downloadRecord({ status: 'paused' })]);
    assert.equal(useLauncherStore.getState().downloads[0].attemptId, 'A');
    assert.equal(useLauncherStore.getState().downloads[0].runId, 2);
    store.mergeDownloadEvent({ ...event, attemptId: 'A', runId: 1 });
    assert.equal(useLauncherStore.getState().downloads[0].status, 'paused');
  });

  it('retires replaced attempts and resets progress for the new attempt', () => {
    const store = useLauncherStore.getState();
    store.setDownloads([downloadRecord({ attemptId: 'A', runId: 4, progressPercent: 90 })]);
    store.setDownloads([downloadRecord({ attemptId: 'B', runId: 1, progressPercent: 2 })]);
    store.mergeDownloadEvent({ ...event, attemptId: 'A', runId: 5, progressPercent: 99 });
    assert.equal(useLauncherStore.getState().downloads[0].attemptId, 'B');
    assert.equal(useLauncherStore.getState().downloads[0].progressPercent, 2);
  });

  it('preserves B when a stale A snapshot arrives', () => {
    const store = useLauncherStore.getState();
    store.mergeDownloadEvent({ ...event, attemptId: 'A', runId: 1 });
    store.removeDownload({ gameId: event.gameId, attemptId: 'A' });
    store.mergeDownloadEvent({ ...event, attemptId: 'B', runId: 1 });
    store.setDownloads([downloadRecord({ attemptId: 'A', runId: 1, status: 'completed' })]);
    assert.equal(useLauncherStore.getState().downloads[0].attemptId, 'B');
    assert.equal(useLauncherStore.getState().downloads[0].status, 'downloading');
  });

  it('rejects identity-free events once a generation is known or subject removed', () => {
    const store = useLauncherStore.getState();
    store.mergeDownloadEvent({ ...event, attemptId: 'A', runId: 1, status: 'paused' });
    store.mergeDownloadEvent({ ...event, status: 'completed', finished: true });
    assert.equal(useLauncherStore.getState().downloads[0].status, 'paused');
    store.removeDownload({ gameId: event.gameId, attemptId: 'A' });
    store.mergeDownloadEvent(event);
    assert.equal(useLauncherStore.getState().downloads.length, 0);
  });

  it('bounds tombstones while retaining the newest removals', () => {
    const store = useLauncherStore.getState();
    for (let index = 0; index < 300; index += 1) {
      store.removeDownload({ gameId: `game-${index}`, attemptId: `attempt-${index}` });
    }
    assert.equal(useLauncherStore.getState().removedDownloads.length, 256);
    assert.equal(useLauncherStore.getState().removedDownloads[0].attemptId, 'attempt-299');
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
