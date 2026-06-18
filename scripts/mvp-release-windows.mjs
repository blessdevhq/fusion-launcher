import { spawn } from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

const hasUpdaterSigningKey = Boolean(process.env.TAURI_SIGNING_PRIVATE_KEY);
const root = process.cwd();
const smokeConfigPath = path.join(root, '.tmp', 'tauri-package-smoke.conf.json');

if (!hasUpdaterSigningKey) {
  await mkdir(path.dirname(smokeConfigPath), { recursive: true });
  await writeFile(
    smokeConfigPath,
    `${JSON.stringify({ bundle: { createUpdaterArtifacts: false } }, null, 2)}\n`
  );
}

const tauriCliPath = path.join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js');
const packageSmokePath = path.join(root, 'scripts', 'mvp-package-smoke.mjs');
const buildSidecarPath = path.join(root, 'scripts', 'build-sidecar.mjs');
// The default engine is the libtorrent sidecar, so the bundle must include it.
const sidecarConfigPath = path.join(root, 'src-tauri', 'tauri.sidecar.conf.json');
const buildArgs = hasUpdaterSigningKey
  ? ['build', '--config', sidecarConfigPath]
  : ['build', '--config', smokeConfigPath, '--config', sidecarConfigPath];

if (!hasUpdaterSigningKey) {
  console.log(
    'TAURI_SIGNING_PRIVATE_KEY is not set; building the Windows smoke package without updater artifacts.'
  );
}

// Build the torrent sidecar first; the externalBin overlay above expects it.
await run(process.execPath, [buildSidecarPath]);
await run(process.execPath, [tauriCliPath, ...buildArgs]);
await run(process.execPath, [packageSmokePath]);

function run(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: root,
      env: process.env,
      stdio: 'inherit'
    });

    child.on('error', reject);
    child.on('exit', (code) => {
      if (code === 0) {
        resolve();
        return;
      }

      reject(new Error(`${command} ${args.join(' ')} exited with ${code}`));
    });
  });
}
