import assert from 'node:assert/strict';

import { previewApi } from '../src/lib/previewData.ts';

const checks = [];

async function check(label, action) {
  try {
    const detail = await action();
    checks.push({ label, ok: true, detail });
    console.log(`PASS ${label}${detail ? ` - ${detail}` : ''}`);
  } catch (error) {
    checks.push({ label, ok: false, error });
    console.error(`FAIL ${label}`);
    console.error(error instanceof Error ? error.message : String(error));
  }
}

await check('repository preview and connect', async () => {
  const url = 'https://community.example/fusion-launcher-repository.json';
  const preview = await previewApi.previewRepository(url);
  assert.equal(preview.url, url);
  assert.ok(preview.catalogCount > 0, 'preview must expose catalog items');
  assert.ok(preview.contentHash, 'preview must expose content hash');

  const connected = await previewApi.connectRepository(url);
  const repositories = await previewApi.listRepositories();
  assert.ok(repositories.some((repository) => repository.id === connected.id));
  assert.ok(connected.catalogCount >= preview.catalogCount);

  const catalog = await previewApi.getCatalog();
  assert.ok(catalog.length >= preview.catalogCount);
  assert.ok(catalog.some((game) => game.id === 'fusion_launcher_nes_smoke'));
  const profiles = await previewApi.listPlatformSetupProfiles();
  assert.ok(profiles.some((profile) => profile.id === 'nes-mesen'));
  assert.ok(profiles.some((profile) => profile.id === 'switch-manual'));

  return `${connected.name}, ${catalog.length} catalog games`;
});

await check('one-click NES demo setup path', async () => {
  await previewApi.deleteEmulatorConfig('nes');
  await previewApi.removeGame('fusion_launcher_nes_smoke', true);

  const before = await previewApi.getEmulatorStatus('nes');
  assert.equal(before.installed, false);

  const emulator = await previewApi.installEmulator('nes');
  assert.equal(emulator.profileId, 'nes-mesen');

  const download = await previewApi.startGameDownload('fusion_launcher_nes_smoke');
  assert.equal(download.sourceKind, 'bundled');
  assert.equal(download.torrent?.status, 'completed');

  const requirements = await previewApi.checkRequirements('fusion_launcher_nes_smoke');
  assert.equal(requirements.ready, true);
  const setup = await previewApi.getGameSetupState('fusion_launcher_nes_smoke');
  assert.equal(setup.profileId, 'nes-mesen');
  assert.equal(setup.launch.status, 'ready');

  const launch = await previewApi.launchGame('fusion_launcher_nes_smoke');
  assert.equal(launch.executable, 'preview://emulators/nes/Mesen.exe');
  assert.ok(launch.resolvedGamePath.includes('fusion_launcher_nes_smoke'));

  return `${download.sourceKind} download -> ${launch.executable}`;
});

await check('user-provided showcase game import', async () => {
  const gameId = 'star-orbit';
  await previewApi.removeGame(gameId, true);
  await previewApi.saveEmulatorConfig('switch', 'preview://emulators/switch.exe', '{game_path}');
  const keys = await previewApi.importProfileSystemFile(gameId, 'switch-prod-keys', 'F:\\Fusion Launcher\\Fixtures\\prod.keys');
  assert.equal(keys.status, 'installed');

  const report = await previewApi.importGameFile(gameId, 'F:\\Fusion Launcher\\Fixtures\\star-orbit.xci');
  assert.equal(report.status, 'installed');
  assert.equal(report.gameId, gameId);
  assert.ok(report.installedPath.endsWith('star-orbit.xci'));
  assert.match(report.sha256, /^[a-f0-9]{64}$/i);

  const download = await previewApi.getGameDownload(gameId);
  assert.equal(download?.status, 'completed');

  const statuses = await previewApi.getLibraryStatuses();
  const status = statuses.find((item) => item.gameId === gameId);
  assert.equal(status?.installed, true);
  assert.equal(status?.systemRequirementsReady, true);
  const setup = await previewApi.getGameSetupState(gameId);
  assert.equal(setup.profileId, 'switch-manual');
  assert.equal(setup.launch.status, 'ready');

  const launch = await previewApi.launchGame(gameId);
  assert.equal(launch.executable, 'preview://emulators/switch.exe');
  assert.ok(launch.resolvedGamePath.includes('star-orbit.xci'));

  return `${gameId} import -> ${launch.executable}`;
});

await check('magnet-like download pause and resume', async () => {
  const gameId = 'forest-quest';
  const started = await previewApi.startGameDownload(gameId);
  assert.equal(started.sourceKind, 'magnet');

  const paused = await previewApi.pauseDownload(gameId);
  assert.equal(paused.status, 'paused');

  const resumed = await previewApi.resumeDownload(gameId);
  assert.equal(resumed.status, 'downloading');
  assert.ok(resumed.downloadSpeedBytesPerSec > 0);

  const downloads = await previewApi.listTorrentDownloads();
  assert.ok(downloads.some((download) => download.gameId === gameId && download.status === 'downloading'));

  return `${gameId} ${paused.status}->${resumed.status}`;
});

await check('health and diagnostics coverage', async () => {
  const health = await previewApi.runHealthCheck();
  assert.ok(health.repositories.length > 0, 'health must include repositories');
  assert.equal(health.downloader.status, 'ready');
  assert.ok(health.platformSetup.some((item) => item.id === 'profile:nes-mesen' && item.status === 'ready'));
  assert.ok(health.emulators.some((item) => item.id === 'emulator:nes' && item.status === 'ready'));
  assert.ok(health.gameFiles.some((item) => item.id === 'game:fusion_launcher_nes_smoke'));

  const diagnostics = await previewApi.getDiagnosticsBundle();
  assert.ok(diagnostics.downloads.length > 0, 'diagnostics must include downloads');
  assert.equal(diagnostics.health.downloader.id, health.downloader.id);
  assert.ok(diagnostics.logs.length > 0, 'diagnostics must include log excerpts');

  return `${diagnostics.downloads.length} downloads in diagnostics`;
});

const failed = checks.filter((item) => !item.ok);
if (failed.length > 0) {
  console.error(`MVP smoke failed: ${failed.length}/${checks.length} checks failed.`);
  process.exit(1);
}

console.log(`MVP smoke passed: ${checks.length} checks.`);
