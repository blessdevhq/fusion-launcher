import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { previewApi } from './previewData.ts';

describe('preview one-click setup support', () => {
  it('installs the NES emulator through the orchestrator preview path', async () => {
    await previewApi.deleteEmulatorConfig('nes');

    const before = await previewApi.getEmulatorStatus('nes');
    assert.equal(before.installed, false);

    const result = await previewApi.installEmulator('nes');
    const after = await previewApi.getEmulatorStatus('nes');

    assert.equal(result.profileId, 'nes-mesen');
    assert.equal(after.installed, true);
    assert.equal(after.exePath, result.exePath);
  });

  it('removes a downloaded NES emulator through the profile removal path', async () => {
    await previewApi.deleteEmulatorConfig('nes');
    await previewApi.installEmulator('nes');

    const report = await previewApi.removeProfileEmulator('nes-mesen');
    const after = await previewApi.getEmulatorStatus('nes');

    assert.equal(report.profileId, 'nes-mesen');
    assert.equal(report.platform, 'nes');
    assert.equal(report.removedConfig, true);
    assert.equal(report.deletedFiles, true);
    assert.equal(after.installed, false);
  });

  it('can prepare the preview demo download for launch', async () => {
    await previewApi.installEmulator('nes');

    const report = await previewApi.startGameDownload('fusion_launcher_nes_smoke');
    const launch = await previewApi.launchGame('fusion_launcher_nes_smoke');

    assert.equal(report.sourceKind, 'bundled');
    assert.equal(report.torrent?.status, 'completed');
    assert.equal(launch.executable, 'preview://Emulators/nes/nes-mesen/Mesen.exe');
  });

  it('uses preview override roots for selected download destinations', async () => {
    await previewApi.deleteEmulatorConfig('nes');
    const emulator = await previewApi.installProfileEmulator('nes-mesen', 'preview://CustomEmulators');
    const report = await previewApi.startGameDownload('fusion_launcher_nes_smoke', 'preview://CustomGames');

    assert.equal(emulator.exePath, 'preview://CustomEmulators/nes/nes-mesen/Mesen.exe');
    assert.equal(report.saveDir, 'preview://CustomGames/fusion_launcher_nes_smoke.zip');
  });

  it('runs the full zero-friction install flow', async () => {
    await previewApi.deleteEmulatorConfig('nes');
    await previewApi.removeGame('fusion_launcher_nes_smoke', true);

    const result = await previewApi.installGame('fusion_launcher_nes_smoke');
    const emulator = await previewApi.getEmulatorStatus('nes');
    const setup = await previewApi.getGameSetupState('fusion_launcher_nes_smoke');

    assert.equal(result.status, 'ready');
    assert.equal(emulator.installed, true);
    assert.equal(setup.launch.status, 'ready');
  });

  it('keeps Switch emulator selection manual', async () => {
    await previewApi.deleteEmulatorConfig('switch');

    const result = await previewApi.installGame('star-orbit');

    assert.equal(result.status, 'error');
    assert.equal(result.errorCode, 'switch_emulator_not_configured');
  });

  it('reports local game import after the Switch emulator is selected', async () => {
    await previewApi.selectProfileEmulator('switch-manual', 'preview://external/eden.exe');
    await previewApi.importProfileSystemFile('star-orbit', 'switch-prod-keys', 'C:/keys/prod.keys');

    const result = await previewApi.installGame('star-orbit');

    assert.equal(result.status, 'needs_game_file');
    assert.equal(result.errorCode, 'game_requires_import');
  });

  it('forgets a manual Switch emulator without deleting files', async () => {
    await previewApi.selectProfileEmulator('switch-manual', 'preview://external/eden.exe');

    const report = await previewApi.removeProfileEmulator('switch-manual');
    const after = await previewApi.getEmulatorStatus('switch');

    assert.equal(report.profileId, 'switch-manual');
    assert.equal(report.removedConfig, true);
    assert.equal(report.deletedFiles, false);
    assert.equal(report.removedPath, null);
    assert.equal(after.installed, false);
  });

  it('saves manual metadata into the preview catalog', async () => {
    await previewApi.saveManualMetadata('terra-pico', {
      title: 'Terra Pico Deluxe',
      description: 'Manual description from the user.',
      cover: 'https://example.com/manual/terra-pico.jpg',
      metadata: {
        releaseYear: 2025,
        developer: 'Manual Studio',
        genres: ['Puzzle', 'Homebrew'],
        tags: ['curated'],
        players: '1 player'
      }
    });

    const game = await previewApi.getGame('terra-pico');
    const state = await previewApi.getScrapeState('terra-pico');

    assert.equal(game?.title, 'Terra Pico Deluxe');
    assert.equal(game?.description, 'Manual description from the user.');
    assert.equal(game?.coverImageUrl, 'https://example.com/manual/terra-pico.jpg');
    assert.equal(game?.metadata?.developer, 'Manual Studio');
    assert.deepEqual(game?.metadata?.genres, ['Puzzle', 'Homebrew']);
    assert.equal(state.status, 'ready');
    assert.equal(state.matchKind, 'override');
  });
});
