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
import { existsSync, mkdirSync, copyFileSync, rmSync, readFileSync, writeFileSync } from 'node:fs';
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

// Read package version to embed valid PE version resources on Windows.
// Legitimate metadata (CompanyName, FileDescription, LegalCopyright) avoids generic AV heuristics.
let versionArgs = [];
if (isWindows) {
  try {
    const pkg = JSON.parse(readFileSync(path.join(root, 'package.json'), 'utf8'));
    const semver = (pkg.version || '0.4.0').split('.').map((p) => parseInt(p, 10) || 0);
    const [vMajor, vMinor, vPatch] = [semver[0] || 0, semver[1] || 0, semver[2] || 0];
    const versionInfoPath = path.join(buildDir, 'version_info.txt');
    const versionInfoContent = `# UTF-8
VSVersionInfo(
  ffi=FixedFileInfo(
    filevers=(${vMajor}, ${vMinor}, ${vPatch}, 0),
    prodvers=(${vMajor}, ${vMinor}, ${vPatch}, 0),
    mask=0x3f,
    flags=0x0,
    OS=0x40004,
    fileType=0x1,
    subtype=0x0,
    date=(0, 0)
  ),
  kids=[
    StringFileInfo(
      [
        StringTable(
          '040904B0',
          [
            StringStruct('CompanyName', 'Fusion Launcher'),
            StringStruct('FileDescription', 'Fusion Launcher P2P Download Engine'),
            StringStruct('FileVersion', '${pkg.version}.0'),
            StringStruct('InternalName', 'fusion-torrent'),
            StringStruct('LegalCopyright', 'Licensed under GPL-3.0-or-later'),
            StringStruct('OriginalFilename', 'fusion-torrent.exe'),
            StringStruct('ProductName', 'Fusion Launcher'),
            StringStruct('ProductVersion', '${pkg.version}.0'),
          ]
        )
      ]
    ),
    VarFileInfo([VarStruct('Translation', [1033, 1200])])
  ]
)
`;
    writeFileSync(versionInfoPath, versionInfoContent, 'utf8');
    versionArgs = ['--version-file', versionInfoPath];
  } catch (err) {
    console.warn('Could not generate version_info.txt for sidecar:', err.message);
  }
}

const hasPyInstaller = isWindows
  ? spawnSync('where pyinstaller', { encoding: 'utf8', shell: true }).status === 0
  : spawnSync('which', ['pyinstaller'], { encoding: 'utf8' }).status === 0;

const isRelease = process.env.NODE_ENV === 'production' || !!process.env.CI || process.argv.includes('--release');

if (!hasPyInstaller && isRelease) {
  throw new Error(
    'Production release build requires PyInstaller with python-libtorrent to guarantee MSE/PE encryption against DPI. Silently falling back to unencrypted librqbit is prohibited in release mode.'
  );
}

const outDir = path.join(root, 'src-tauri', 'binaries');
mkdirSync(outDir, { recursive: true });
const dest = path.join(outDir, `fusion-torrent-${triple}${exeSuffix}`);

if (hasPyInstaller) {
  console.log(`Building production sidecar with PyInstaller for target ${triple}...`);
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
      ...versionArgs,
      scriptPath,
    ],
    'PyInstaller'
  );

  const built = path.join(buildDir, 'dist', `fusion-torrent${exeSuffix}`);
  if (!existsSync(built)) {
    throw new Error(`PyInstaller did not produce the expected binary: ${built}`);
  }
  copyFileSync(built, dest);
  console.log(`PyInstaller sidecar built: ${path.relative(root, dest)}`);
} else {
  console.warn(
    `[DEV WARNING] PyInstaller not found. Falling back to native Rust sidecar (bin: fusion-torrent) without MSE/PE obfuscation.`
  );
  run(
    'cargo',
    ['build', '--release', '--bin', 'fusion-torrent', '--manifest-path', 'src-tauri/Cargo.toml'],
    'Cargo'
  );

  const built = path.join(root, 'src-tauri', 'target', 'release', `fusion-torrent${exeSuffix}`);
  if (!existsSync(built)) {
    throw new Error(`Cargo did not produce the expected binary: ${built}`);
  }
  copyFileSync(built, dest);
  console.log(`Native dev fallback sidecar built: ${path.relative(root, dest)}`);
}
