// Builds the libtorrent download sidecar (sidecar/torrent_sidecar.py) into a
// standalone executable with PyInstaller and places it where Tauri's
// externalBin overlay (src-tauri/tauri.sidecar.conf.json) expects it:
//
//     src-tauri/binaries/fusion-torrent-<target-triple>[.exe]
//
// Prerequisites (CI installs these from sidecar/requirements.txt):
//   pip install -r sidecar/requirements.txt   # libtorrent + pyinstaller
//
// Usage: npm run sidecar:build
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, copyFileSync, rmSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const isWindows = process.platform === 'win32';
const exeSuffix = isWindows ? '.exe' : '';

// On Windows we need `shell: true` so PATHEXT resolves the `pyinstaller`/`rustc`
// launchers, but a shell joins args with spaces — so quote any arg that contains
// whitespace (e.g. the "Fusion Launcher" path) or PyInstaller sees split args.
function quoteForShell(arg) {
  return isWindows && /\s/.test(arg) ? `"${arg}"` : arg;
}

function run(command, args, label) {
  // On Windows pass one quoted command string to the shell (avoids Node's
  // DEP0190 warning that fires for an args array + shell:true). Elsewhere pass
  // the args array directly with no shell.
  const [program, programArgs, useShell] = isWindows
    ? [[command, ...args].map(quoteForShell).join(' '), undefined, true]
    : [command, args, false];
  const result = spawnSync(program, programArgs, { cwd: root, stdio: 'inherit', shell: useShell });
  if (result.error || result.status !== 0) {
    throw new Error(
      `${label} failed (${command} ${args.join(' ')}). ` +
        (result.error ? result.error.message : `exit code ${result.status}`)
    );
  }
}

// Resolve the host target triple so the binary name matches what Tauri looks for.
function hostTargetTriple() {
  if (process.env.SIDECAR_TARGET_TRIPLE) {
    return process.env.SIDECAR_TARGET_TRIPLE;
  }
  const rustc = isWindows
    ? spawnSync('rustc -vV', { encoding: 'utf8', shell: true })
    : spawnSync('rustc', ['-vV'], { encoding: 'utf8' });
  if (rustc.status !== 0) {
    throw new Error('Could not run `rustc -vV` to determine the host target triple.');
  }
  const match = rustc.stdout.match(/^host:\s*(.+)$/m);
  if (!match) {
    throw new Error('Could not parse the host target triple from `rustc -vV`.');
  }
  return match[1].trim();
}

const triple = hostTargetTriple();
const scriptPath = path.join(root, 'sidecar', 'torrent_sidecar.py');
if (!existsSync(scriptPath)) {
  throw new Error(`Sidecar source not found: ${scriptPath}`);
}

const buildDir = path.join(root, '.tmp', 'sidecar-build');
rmSync(buildDir, { recursive: true, force: true });
mkdirSync(buildDir, { recursive: true });

// --console keeps stdout/stdin wired so the parent can read the JSON stream;
// --windowed/--noconsole would detach stdout and break the contract.
run(
  'pyinstaller',
  [
    '--onefile',
    '--console',
    '--name',
    'fusion-torrent',
    '--distpath',
    path.join(buildDir, 'dist'),
    '--workpath',
    path.join(buildDir, 'work'),
    '--specpath',
    buildDir,
    scriptPath,
  ],
  'PyInstaller'
);

const built = path.join(buildDir, 'dist', `fusion-torrent${exeSuffix}`);
if (!existsSync(built)) {
  throw new Error(`PyInstaller did not produce the expected binary: ${built}`);
}

const outDir = path.join(root, 'src-tauri', 'binaries');
mkdirSync(outDir, { recursive: true });
const dest = path.join(outDir, `fusion-torrent-${triple}${exeSuffix}`);
copyFileSync(built, dest);

console.log(`Sidecar built: ${path.relative(root, dest)}`);
