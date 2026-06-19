// Downloads the Microsoft Visual C++ 2015-2022 x64 Redistributable into
//
//     src-tauri/redist/vc_redist.x64.exe
//
// so the NSIS installer hook (src-tauri/nsis/hooks.nsh) can install it silently
// on machines that lack the runtime. The libtorrent torrent sidecar is a
// PyInstaller binary that dynamically links the MSVC runtime; without it
// `import libtorrent` fails with an ImportError (DLL load failed) and downloads
// die with "Torrent sidecar exited unexpectedly (code 1)".
//
// Like the sidecar binary, this is a build artifact and is git-ignored; CI and
// the local release flow fetch it fresh.
//
// Usage: npm run redist:fetch
import { existsSync, mkdirSync } from 'node:fs';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const downloadUrl =
  process.env.FUSION_VCREDIST_URL || 'https://aka.ms/vs/17/release/vc_redist.x64.exe';
const outDir = path.join(root, 'src-tauri', 'redist');
const outFile = path.join(outDir, 'vc_redist.x64.exe');

if (existsSync(outFile) && !process.env.FUSION_VCREDIST_FORCE) {
  console.log(
    `vc_redist already present: ${path.relative(root, outFile)} ` +
      '(set FUSION_VCREDIST_FORCE=1 to re-download)'
  );
  process.exit(0);
}

mkdirSync(outDir, { recursive: true });
console.log(`Downloading vc_redist.x64.exe from ${downloadUrl} ...`);

const response = await fetch(downloadUrl, { redirect: 'follow' });
if (!response.ok) {
  throw new Error(`Failed to download vc_redist.x64.exe (HTTP ${response.status}).`);
}

const buffer = Buffer.from(await response.arrayBuffer());
// The redistributable is ~25 MB; a tiny payload means we fetched an error page
// or a redirect stub instead of the real installer.
if (buffer.length < 1_000_000) {
  throw new Error(
    `Downloaded vc_redist.x64.exe is suspiciously small (${buffer.length} bytes); aborting.`
  );
}

await writeFile(outFile, buffer);
console.log(
  `vc_redist saved: ${path.relative(root, outFile)} (${(buffer.length / 1048576).toFixed(1)} MB)`
);
