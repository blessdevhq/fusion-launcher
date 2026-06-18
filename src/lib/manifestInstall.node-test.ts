import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  extractErrorMessage,
  manifestGameHasEmulator,
  manifestInstallOutcome
} from './manifestInstall.ts';
import type { InstallResult } from '../types/emulatorProfile.ts';
import type { ManifestGame } from '../types/manifest.ts';

function gameWith(coreBundleUrl?: string | null): ManifestGame {
  return {
    title_id: '0100F2C0115B6000',
    title: 'The Legend of Zelda: Tears of the Kingdom',
    platform: 'switch',
    game_version: '1.2.1',
    visuals: { cover_url: 'https://example.com/cover.png', background_url: 'https://example.com/bg.jpg' },
    assets: {
      heavy_rom_magnet: 'magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567',
      core_bundle_p2p_hash: 'hash',
      shader_cache_url: 'https://example.com/shaders.zip',
      core_bundle_url: coreBundleUrl
    },
    launch_config: { engine: 'eden', executable: 'eden.exe', args: ['-f', '-g', '{rom_path}'], inject_mods: [] }
  };
}

function resultWith(status: InstallResult['status']): InstallResult {
  return { gameId: 'g', status, errorCode: null, message: null };
}

describe('manifestGameHasEmulator', () => {
  it('is true when a non-empty core_bundle_url is present', () => {
    assert.equal(manifestGameHasEmulator(gameWith('https://example.com/eden.zip')), true);
  });

  it('is false when core_bundle_url is missing, null, or blank', () => {
    assert.equal(manifestGameHasEmulator(gameWith(undefined)), false);
    assert.equal(manifestGameHasEmulator(gameWith(null)), false);
    assert.equal(manifestGameHasEmulator(gameWith('   ')), false);
  });
});

describe('manifestInstallOutcome', () => {
  it('maps a ready result to "ready"', () => {
    assert.equal(manifestInstallOutcome(resultWith('ready')), 'ready');
  });

  it('maps every non-ready status to "attention"', () => {
    assert.equal(manifestInstallOutcome(resultWith('needs_system_files')), 'attention');
    assert.equal(manifestInstallOutcome(resultWith('needs_game_file')), 'attention');
    assert.equal(manifestInstallOutcome(resultWith('error')), 'attention');
  });
});

describe('extractErrorMessage', () => {
  it('reads the message from a Tauri structured error', () => {
    assert.equal(extractErrorMessage({ kind: 'network', message: 'host down' }), 'host down');
  });

  it('reads the message from an Error instance', () => {
    assert.equal(extractErrorMessage(new Error('boom')), 'boom');
  });

  it('stringifies anything else', () => {
    assert.equal(extractErrorMessage('plain string'), 'plain string');
  });
});
