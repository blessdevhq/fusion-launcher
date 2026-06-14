import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';
import { validateRepositorySchema } from './repository.ts';
import type { ImportAssetFileReport, ImportGameFileReport, RequirementItem } from './repository.ts';

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

  it('accepts v3 rich metadata and reserved setup profiles', () => {
    const repo = validateRepositorySchema({
      metadata: { id: 'showcase', name: 'Showcase', version: '1', schemaVersion: 3 },
      system_files: [],
      catalog: [
        {
          id: 'star-orbit',
          platform: 'switch',
          title: 'Star Orbit Prototype',
          description: 'A fictional showcase entry.',
          artwork: {
            cover: 'https://example.com/star-orbit-cover.jpg',
            hero: 'https://example.com/star-orbit-hero.jpg',
            logo: 'https://example.com/star-orbit-logo.png',
            screenshots: ['https://example.com/star-orbit-screen.jpg']
          },
          metadata: {
            releaseYear: 2026,
            developer: 'Fusion Launcher Labs',
            publisher: 'Community Preview',
            genres: ['Adventure'],
            tags: ['user-provided'],
            players: '1',
            series: 'Star Orbit',
            externalIds: { showcase: 'star-orbit' }
          },
          contentMode: 'user_provided',
          setupProfileId: 'switch-manual',
          downloads: [{ kind: 'user_provided', instructions: 'Import your local .xci/.nsp package.' }],
          expectedExtensions: ['.xci', '.nsp']
        }
      ]
    });

    assert.equal(repo.metadata.schemaVersion, 3);
    assert.equal(repo.catalog[0].contentMode, 'user_provided');
    assert.equal(repo.catalog[0].setupProfileId, 'switch-manual');
    assert.equal(repo.catalog[0].artwork?.hero, 'https://example.com/star-orbit-hero.jpg');
  });

  it('inherits expected extensions from a known setup profile', () => {
    const repo = validateRepositorySchema({
      metadata: { id: 'showcase', name: 'Showcase', version: '1', schemaVersion: 3 },
      system_files: [],
      catalog: [
        {
          id: 'star-orbit',
          platform: 'switch',
          title: 'Star Orbit Prototype',
          contentMode: 'user_provided',
          setupProfileId: 'switch-manual',
          downloads: [{ kind: 'user_provided', instructions: 'Import your local package.' }]
        }
      ]
    });

    assert.deepEqual(repo.catalog[0].expectedExtensions, ['.nsp', '.xci', '.nca']);
  });

  it('normalizes platformProfileId to the runtime setup profile', () => {
    const repo = validateRepositorySchema({
      metadata: { id: 'zero-friction', name: 'Zero Friction', version: '1', schemaVersion: 3 },
      system_files: [],
      catalog: [
        {
          id: 'gba-game',
          platform: 'gba',
          title: 'GBA Game',
          platformProfileId: 'gba-mgba',
          downloads: [{ kind: 'magnet', uri: 'magnet:?xt=urn:btih:abcdef' }]
        }
      ]
    });

    assert.equal(repo.catalog[0].setupProfileId, 'gba-mgba');
    assert.deepEqual(repo.catalog[0].expectedExtensions, ['.gba']);
  });

  it('keeps unknown setup profiles visible when expected extensions are explicit', () => {
    const repo = validateRepositorySchema({
      metadata: { id: 'showcase', name: 'Showcase', version: '1', schemaVersion: 3 },
      system_files: [],
      catalog: [
        {
          id: 'unknown-profile-game',
          platform: 'switch',
          title: 'Unknown Profile Game',
          setupProfileId: 'community-profile',
          downloads: [{ kind: 'user_provided', instructions: 'Import your local package.' }],
          expectedExtensions: ['.xci']
        }
      ]
    });

    assert.equal(repo.catalog[0].setupProfileId, 'community-profile');
    assert.deepEqual(repo.catalog[0].expectedExtensions, ['.xci']);
  });

  it('accepts the source library template repository', () => {
    const template = JSON.parse(readFileSync(
      new URL('../../templates/source-library/repository.json', import.meta.url),
      'utf8'
    ));

    const repo = validateRepositorySchema(template);

    assert.equal(repo.metadata.id, 'fusion-launcher-source-template');
    assert.equal(repo.catalog.length, 4);
    assert.equal(repo.catalog[2].contentMode, 'user_provided');
    assert.equal(repo.catalog[3].contentMode, 'metadata_only');
  });

  it('accepts bundled sources for local built-in repository validation', () => {
    const repo = validateRepositorySchema({
      metadata: {
        id: 'fusion-launcher-demo',
        name: 'Fusion Launcher Built-in Demo Repository',
        version: '1.0.0',
        schemaVersion: 2,
        trustLevel: 'official'
      },
      system_files: [],
      catalog: [
        {
          id: 'fusion_launcher_nes_smoke',
          platform: 'nes',
          title: 'Fusion Launcher NES Smoke Demo',
          downloads: [{
            kind: 'bundled',
            path: 'demo-content/fusion-launcher-smoke.nes',
            sha256: hash,
            sizeBytes: 24592
          }],
          expectedExtensions: ['.nes']
        }
      ]
    });

    assert.equal(repo.catalog[0].downloads[0].kind, 'bundled');
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

  it('models import reports and requirement checksums', () => {
    const report: ImportAssetFileReport = {
      status: 'already_installed',
      installedPath: 'C:\\Fusion Launcher\\System\\nes\\bios.bin'
    };
    const requirement = {
      checksum: hash,
      sha256: hash
    } as RequirementItem;
    const gameReport: ImportGameFileReport = {
      status: 'installed',
      gameId: 'repo::game',
      installedPath: 'C:\\Fusion Launcher\\Games\\game.xci',
      sha256: hash
    };

    assert.equal(report.status, 'already_installed');
    assert.equal(gameReport.status, 'installed');
    assert.equal(requirement.checksum, hash);
  });
});
