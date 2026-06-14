import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { readdir, rm, stat } from 'node:fs/promises';
import path from 'node:path';

const root = process.cwd();
const smokeDir = path.join(root, '.tmp', 'package-smoke');
const releaseDir = resolveReleaseDir();
const binaryPath = await resolveBinaryPath(releaseDir);
const nsisDir = path.join(releaseDir, 'bundle', 'nsis');

async function main() {
  await assertFile(binaryPath, 'release binary');
  const installers = await listNsisInstallers();
  assert.ok(installers.length > 0, 'Tauri NSIS installer artifact is missing. Run npm run tauri:build first.');

  await rm(smokeDir, { recursive: true, force: true });
  const output = await runSmokeBinary();

  console.log(`PASS release binary: ${path.relative(root, binaryPath)}`);
  console.log(`PASS NSIS artifact: ${path.relative(root, installers[0])}`);
  console.log(`PASS package smoke data: ${path.relative(root, smokeDir)}`);
  if (output.trim()) console.log(output.trim());
}

async function assertFile(filePath, label) {
  try {
    const info = await stat(filePath);
    assert.ok(info.isFile(), `${label} is not a file: ${filePath}`);
  } catch (error) {
    throw new Error(
      `${label} is missing: ${filePath}. Run npm run tauri:build first, or set FUSION_LAUNCHER_RELEASE_DIR to the Tauri release directory.\n${error.message}`
    );
  }
}

function resolveReleaseDir() {
  const configured = process.env.FUSION_LAUNCHER_RELEASE_DIR?.trim() || process.env.RETROHYDRA_RELEASE_DIR?.trim();
  if (configured) {
    return path.resolve(root, configured);
  }

  return path.join(root, 'src-tauri', 'target', 'release');
}

async function resolveBinaryPath(releaseDir) {
  const candidates = process.platform === 'win32'
    ? ['fusion-launcher.exe', 'retrohydra.exe']
    : ['fusion-launcher', 'retrohydra'];
  for (const candidate of candidates) {
    const filePath = path.join(releaseDir, candidate);
    try {
      const info = await stat(filePath);
      if (info.isFile()) return filePath;
    } catch {
      // Try the next transition-period binary name.
    }
  }
  return path.join(releaseDir, candidates[0]);
}

async function listNsisInstallers() {
  try {
    const entries = await readdir(nsisDir);
    return entries
      .filter((entry) => entry.toLowerCase().endsWith('.exe'))
      .map((entry) => path.join(nsisDir, entry));
  } catch {
    return [];
  }
}

function runSmokeBinary() {
  return new Promise((resolve, reject) => {
    const child = spawn(binaryPath, [], {
      cwd: root,
      env: {
        ...process.env,
        FUSION_LAUNCHER_PACKAGE_SMOKE: '1',
        FUSION_LAUNCHER_PACKAGE_SMOKE_DATA_DIR: smokeDir,
        RETROHYDRA_PACKAGE_SMOKE: process.env.RETROHYDRA_PACKAGE_SMOKE ?? '1',
        RETROHYDRA_PACKAGE_SMOKE_DATA_DIR: process.env.RETROHYDRA_PACKAGE_SMOKE_DATA_DIR ?? smokeDir
      },
      stdio: ['ignore', 'pipe', 'pipe']
    });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk.toString();
    });
    child.on('error', reject);
    child.on('exit', (code) => {
      if (code === 0) {
        resolve(stdout);
        return;
      }
      reject(new Error(`Package smoke exited with ${code}.\n${stdout}\n${stderr}`));
    });
  });
}

await main();
