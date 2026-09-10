import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { isDirectGameDownload, shouldShowDownloadPeers } from './downloadActions.ts';

const game = {
  id: 'repo::game',
  sourceId: 'game',
  repositoryId: 'repo',
  repositoryName: 'Demo Repo',
  platform: 'nes',
  title: 'Demo Game',
  downloads: [{ kind: 'http', url: 'https://example.com/game.nes', sha256: 'a'.repeat(64) }],
  expectedExtensions: ['.nes'],
  requiredSystemFileIds: []
};

describe('download retry actions', () => {
  it('restarts HTTP and bundled downloads instead of resuming them as torrents', () => {
    assert.equal(isDirectGameDownload(game, null), true);
    assert.equal(
      isDirectGameDownload(
        { ...game, downloads: [{ kind: 'magnet', uri: 'magnet:?xt=urn:btih:abc' }] },
        { magnetUri: 'direct:bundled' }
      ),
      true
    );
  });

  it('keeps torrent retries on the resume path', () => {
    assert.equal(
      isDirectGameDownload(
        { ...game, downloads: [{ kind: 'magnet', uri: 'magnet:?xt=urn:btih:abc' }] },
        { magnetUri: 'magnet:?xt=urn:btih:abc' }
      ),
      false
    );
  });

  it('shows peer counts only for torrent download rows', () => {
    assert.equal(shouldShowDownloadPeers({ magnetUri: 'magnet:?xt=urn:btih:abc' }), true);
    assert.equal(shouldShowDownloadPeers({ magnetUri: 'direct:http' }), false);
    assert.equal(shouldShowDownloadPeers({ magnetUri: 'direct:asset:http' }), false);
  });
});
