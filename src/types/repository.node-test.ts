import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { validateRepositorySchema } from './repository.ts';

const hash = 'a'.repeat(64);

describe('repository schema', () => {
  it('accepts a strict BYOR repository', () => {
    const repo = validateRepositorySchema({
      metadata: {
        id: 'community-index',
        name: 'Community Index',
        version: '1.0.0',
        schemaVersion: 2,
        maintainer: 'Community',
        homepageUrl: 'https://example.com',
        license: 'MIT',
        trustLevel: 'community',
        contentHash: hash
      },
      system_files: [
        {
          id: 'emu-1',
          platform: 'nes',
          assetKind: 'emulator',
          displayName: 'User Emulator',
          executable: true,
          sources: [{ kind: 'http', url: 'https://example.com/emulator.zip', sha256: hash }]
        }
      ],
      catalog: [
        {
          id: 'game-1',
          platform: 'nes',
          title: 'Homebrew Game',
          downloads: [{ kind: 'magnet', uri: 'magnet:?xt=urn:btih:abcdef' }],
          expectedExtensions: ['.nes'],
          requiredSystemFileIds: ['emu-1']
        }
      ]
    });

    assert.equal(repo.metadata.id, 'community-index');
    assert.equal(repo.metadata.trustLevel, 'community');
  });

  it('accepts user-provided system files with optional hashes', () => {
    const repo = validateRepositorySchema({
      metadata: { id: 'repo', name: 'Repo', version: '1', schemaVersion: 2 },
      system_files: [
        {
          id: 'ps1-bios',
          platform: 'ps1',
          assetKind: 'bios',
          displayName: 'PS1 BIOS',
          sources: [{ kind: 'user_provided', sha256: hash, instructions: 'Dump your own BIOS.' }],
          installHint: { target: 'app_system', relativePath: 'bios/scph1001.bin' }
        }
      ],
      catalog: [
        {
          id: 'game',
          platform: 'ps1',
          title: 'Game',
          downloads: [{ kind: 'magnet', uri: 'magnet:?xt=urn:btih:abcdef' }],
          expectedExtensions: ['.bin'],
          requiredSystemFileIds: ['ps1-bios']
        }
      ]
    });

    assert.equal(repo.system_files[0].sources[0].kind, 'user_provided');
  });

  it('rejects HTTP assets without sha256', () => {
    assert.throws(() => validateRepositorySchema({
      metadata: { id: 'bad', name: 'Bad', version: '1', schemaVersion: 2 },
      system_files: [
        {
          id: 'asset',
          platform: 'switch',
          assetKind: 'keys',
          displayName: 'Keys',
          sources: [{ kind: 'http', url: 'https://example.com/keys.zip' }]
        }
      ],
      catalog: []
    }));
  });

  it('rejects unsupported URL-shaped protocols', () => {
    assert.throws(() => validateRepositorySchema({
      metadata: { id: 'bad', name: 'Bad', version: '1', schemaVersion: 2 },
      system_files: [],
      catalog: [
        {
          id: 'game',
          platform: 'nes',
          title: 'Game',
          downloads: [{ kind: 'magnet', uri: 'file:///game.rom' }],
          expectedExtensions: ['.nes']
        }
      ]
    }));
  });

  it('rejects unknown platforms', () => {
    assert.throws(() => validateRepositorySchema({
      metadata: { id: 'bad', name: 'Bad', version: '1', schemaVersion: 2 },
      system_files: [],
      catalog: [
        {
          id: 'game',
          platform: 'unknown-platform',
          title: 'Game',
          downloads: [{ kind: 'magnet', uri: 'magnet:?xt=urn:btih:abcdef' }],
          expectedExtensions: ['.rom']
        }
      ]
    }));
  });

  it('rejects missing expected extensions', () => {
    assert.throws(() => validateRepositorySchema({
      metadata: { id: 'bad', name: 'Bad', version: '1', schemaVersion: 2 },
      system_files: [],
      catalog: [
        {
          id: 'game',
          platform: 'nes',
          title: 'Game',
          downloads: [{ kind: 'magnet', uri: 'magnet:?xt=urn:btih:abcdef' }]
        }
      ]
    }));
  });

  it('normalizes expected extensions to lowercase', () => {
    const repo = validateRepositorySchema({
      metadata: { id: 'repo', name: 'Repo', version: '1', schemaVersion: 2 },
      system_files: [],
      catalog: [
        {
          id: 'game',
          platform: 'switch',
          title: 'Game',
          downloads: [{ kind: 'magnet', uri: 'magnet:?xt=urn:btih:abcdef' }],
          expectedExtensions: ['.NSP']
        }
      ]
    });

    assert.deepEqual(repo.catalog[0].expectedExtensions, ['.nsp']);
  });
});
