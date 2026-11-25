// Local authoring/launch entry point. Private inputs never become source artifacts.
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { access, copyFile, lstat, mkdir, readdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const sourceId = 'totk';
const gameId = 'f68';
const archiveHash = '6c1b53ce325170a026cc0f87098380027dc6170d94ba95f913aab7596fd097cb';
const exeHash = 'df08b0fe90ec07988e9bfbe598b32a6dbda3e4e81e35ed2120793e42dd67f565';
const defaultProfile = path.join(repo, '.tmp/totk-lab/m1-stable021-20261020');
const defaultOutput = path.join(repo, '.tmp/totk-fusion/ready');
const slash = p => p.replaceAll('\\', '/');

export async function sha256(file) {
  const digest = createHash('sha256');
  for await (const chunk of createReadStream(file)) digest.update(chunk);
  return digest.digest('hex');
}

async function plain(file, directory = false) {
  // Inspect every ancestor before resolving, so a junction cannot hide in a path.
  const absolute = path.resolve(file);
  for (let part = absolute; ; part = path.dirname(part)) {
    const info = await lstat(part);
    if (info.isSymbolicLink()) throw new Error(`Reparse/symlink input refused: ${part}`);
    if (part === path.dirname(part)) break;
  }
  const info = await lstat(absolute);
  if (directory ? !info.isDirectory() : !info.isFile() || info.nlink !== 1) throw new Error(`Expected plain ${directory ? 'directory' : 'single-link file'}: ${absolute}`);
  return absolute;
}

async function inventory(root) {
  await plain(root, true);
  const rows = [];
  async function visit(dir) {
    for (const name of (await readdir(dir)).sort()) {
      const file = path.join(dir, name);
      const info = await lstat(file);
      if (info.isSymbolicLink()) throw new Error(`Linked profile entry refused: ${file}`);
      if (info.isDirectory()) await visit(file);
      else {
        await plain(file);
        rows.push({ path: slash(path.relative(root, file)), bytes: info.size, sha256: await sha256(file) });
      }
    }
  }
  await visit(root);
  return rows;
}

async function copyVerified(source, dest) {
  const before = await inventory(source);
  await mkdir(dest, { recursive: true });
  for (const file of before) {
    const out = path.join(dest, file.path);
    await mkdir(path.dirname(out), { recursive: true });
    await copyFile(path.join(source, file.path), out);
  }
  if (JSON.stringify(before) !== JSON.stringify(await inventory(dest)) || JSON.stringify(before) !== JSON.stringify(await inventory(source))) {
    throw new Error('Profile changed while copying. Private partial output retained; source untouched. Retry with Eden closed and a new --out.');
  }
  return before;
}

export function portableConfig(text) {
  // QSettings default=true ignores stale stored absolute paths and computes
  // directories from this executable's portable user folder on every launch.
  let section = '';
  return text.split(/\r?\n/).map(line => {
    if (line.startsWith('[')) section = line.slice(1, -1);
    const eq = line.indexOf('=');
    if (eq < 0) return line;
    const key = line.slice(0, eq);
    if (section === 'Data%20Storage' && /_directory(?:\\default)?$/.test(key)) {
      return `${key}=${key.endsWith('\\default') ? 'true' : ''}`;
    }
    // The explicit -g argument selects the game; do not retain lab library or
    // recent-file locations in the new portable profile.
    if (section === 'UI' && /^(?:Paths\\gamedirs|Paths\\recentFiles)/.test(key)) return null;
    return line;
  }).filter(line => line !== null).join('\n');
}

function run(binary, args, { wait = true } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(binary, args, { cwd: repo, windowsHide: false, stdio: wait ? ['ignore', 'pipe', 'inherit'] : 'inherit', shell: false });
    let output = '';
    if (wait) child.stdout.on('data', chunk => { output += chunk; if (output.length > 32 * 1024 * 1024) child.kill(); });
    child.on('error', reject);
    child.on('close', code => {
      if (code !== 0) return reject(new Error(`${path.basename(binary)} exited ${code}`));
      try { resolve(wait ? JSON.parse(output) : null); } catch { reject(new Error(`Invalid CLI JSON from ${binary}`)); }
    });
  });
}

async function binaries() {
  const bin = path.join(repo, 'src-tauri/target/debug');
  const suffix = process.platform === 'win32' ? '.exe' : '';
  const packer = path.join(bin, `fusion-packer${suffix}`);
  const local = path.join(bin, `fusion-local${suffix}`);
  try { await access(packer); await access(local); }
  catch { throw new Error('Build first: cargo build --manifest-path src-tauri/Cargo.toml --bin fusion-local --bin fusion-packer'); }
  return { packer, local };
}

async function prepare(profile, output) {
  profile = await plain(profile, true);
  const runtime = path.join(profile, 'runtime');
  const user = path.join(runtime, 'user');
  const archive = path.join(profile, 'downloads/Eden-Windows-v0.2.1-amd64-clang-pgo.zip');
  if (await sha256(await plain(archive)) !== archiveHash || await sha256(await plain(path.join(runtime, 'eden.exe'))) !== exeHash) throw new Error('Eden archive or EXE differs from the approved 0.2.1 candidate.');
  const configText = await readFile(await plain(path.join(user, 'config/qt-config.ini')), 'utf8');
  for (const axis of ['x', 'y']) if (!configText.includes(`mouse_panning_${axis}_sensitivity=68`)) throw new Error('Expected accepted F68 profile.');
  if ((await inventory(path.join(user, 'load'))).length) throw new Error('Expected vanilla profile without mods.');
  const games = await plain(path.join(profile, '../m1-baseline-20261019/games'), true);
  const names = await readdir(games);
  const base = names.find(name => name.includes('[0100F2C0115B6000]') && name.endsWith('.nsp'));
  const update = names.find(name => name.includes('[0100F2C0115B6800]') && name.endsWith('.nsp'));
  if (!base || !update) throw new Error('Expected lab-owned TOTK NSP and update.');
  // mkdir is deliberately non-recursive at the leaf: never replace an install.
  await mkdir(path.dirname(output), { recursive: true });
  await mkdir(output);
  const input = path.join(output, 'private/input');
  const localGames = path.join(input, 'game');
  await mkdir(localGames, { recursive: true });
  console.error('Copying owned game inputs to private safe-name paths; originals are read-only inputs.');
  for (const [original, alias] of [[base, 'game.nsp'], [update, 'update.nsp']]) {
    const source = await plain(path.join(games, original));
    await copyFile(source, path.join(localGames, alias));
    const expected = await sha256(source);
    if (expected !== await sha256(path.join(localGames, alias))) throw new Error('Owned game copy hash mismatch.');
  }
  const seed = path.join(input, 'runtime/user');
  await mkdir(path.join(seed, 'config'), { recursive: true });
  const inventories = {};
  for (const directory of ['nand', 'sdmc', 'keys']) inventories[directory] = await copyVerified(path.join(user, directory), path.join(seed, directory));
  for (const directory of ['load', 'cache', 'shader', 'log', 'dump']) await mkdir(path.join(seed, directory), { recursive: true });
  await writeFile(path.join(seed, 'config/qt-config.ini'), portableConfig(configText), { flag: 'wx' });
  await writeFile(path.join(input, 'fusion-profile.txt'), 'Fusion local TOTK 1.1.0 / Eden0.2.1 / F68\n', { flag: 'wx' });
  await writeFile(path.join(output, 'private/source-inventory.json'), JSON.stringify(inventories, null, 2), { flag: 'wx' });
  const { packer } = await binaries();
  const recipe = {
    source_info: { id: sourceId, name: 'TOTK — локальная библиотека', maintainer: 'Local user' },
    base_url: `https://offline.fusion.invalid/${sourceId}/`,
    games: [{ id: gameId, title: 'The Legend of Zelda: Tears of the Kingdom', version: '1.1.0-eden0.2.1-f68.1', platform: 'switch',
      layers: [
        { id: 'game', kind: 'base', mount_path: '', writable_copy_paths: ['runtime/user/**'], local_locator: { reference_dir: slash(input), store: 'manual', path_hints: [slash(input)], key_files: ['game/game.nsp', 'game/update.nsp', 'fusion-profile.txt'].map(p => ({ path: p, role: 'core_library' })) } },
        { id: 'eden', kind: 'runtime', local_path: slash(archive), archive: 'zip', mount_path: 'runtime', depends_on: ['game'], critical_files: [{ path: 'eden.exe', role: 'main_executable' }, { path: 'eden-cli.exe', role: 'main_executable' }] },
      ],
      runtime: { layer_id: 'eden', executable: 'runtime/eden.exe', args: ['-g', '{{WORKSPACE_DIR}}/game/game.nsp'], default_channel: 'pinned' },
      persistence: { saves_paths: ['workspace/runtime/user'] },
      local_placement: 'workspace_on_base_volume',
    }],
  };
  await writeFile(path.join(output, 'recipe.json'), JSON.stringify(recipe, null, 2), { flag: 'wx' });
  console.error('Building private Fusion Source: hashing local ROM inputs; no ROM/key/save artifacts.');
  const build = await run(packer, ['build', '--recipe', path.join(output, 'recipe.json'), '--out', path.join(output, 'source')]);
  await run(packer, ['verify', '--source-dir', path.join(output, 'source')]);
  await writeFile(path.join(output, 'build-report.json'), JSON.stringify(build, null, 2), { flag: 'wx' });
  const index = JSON.parse(await readFile(path.join(output, 'source/source-index.json'), 'utf8'));
  const manifestUrl = new URL(index.games[0].manifest_url);
  const rel = manifestUrl.pathname.split(`/${sourceId}/`)[1];
  const manifest = JSON.parse(await readFile(path.join(output, 'source', rel), 'utf8'));
  // Trust inputs are created locally only after checking the pinned archive/EXE.
  // The runner never automatically trusts a downloaded source's critical list.
  const approved = manifest.layers.flatMap(layer => {
    const keys = layer.source.kind === 'local_locator' ? layer.source.locator.key_files : [];
    return [...(layer.critical_files ?? []), ...keys].map(file => ({ path: [layer.mount_path, file.path].filter(Boolean).join('/'), blake3: file.blake3 }));
  });
  await writeFile(path.join(output, 'private/approved-local-files.json'), JSON.stringify(approved, null, 2), { flag: 'wx' });
  await writeFile(path.join(output, 'local.json'), JSON.stringify({ sourceId, gameId, profile: slash(profile), archiveHash, exeHash, createdAt: new Date().toISOString(), privateOnly: true }, null, 2), { flag: 'wx' });
  await writeShortcut(output);
  return { prepared: true, directory: output, next: 'npm run totk:local -- install' };
}

export async function writeShortcut(output) {
  if (process.platform !== 'win32') return;
  const quote = value => `"${value.replaceAll('%', '%%')}"`;
  const script = '@echo off\r\n' + `${quote(process.execPath)} ${quote(path.join(repo, 'scripts/totk-local.mjs'))} launch --out ${quote(output)}\r\nif errorlevel 1 pause\r\n`;
  await writeFile(path.join(output, 'Play-TOTK.cmd'), script, { flag: 'wx' });
}

export async function main(args) {
  const command = args.shift() ?? 'help';
  let output = defaultOutput;
  let profile = defaultProfile;
  while (args.length) {
    const option = args.shift(), value = args.shift();
    if (!value || !['--out', '--profile'].includes(option)) throw new Error('Use --out <new-private-directory> or --profile <approved-lab-profile>.');
    if (option === '--out') output = path.resolve(value);
    else profile = path.resolve(value);
  }
  if (command === 'help') return { usage: 'npm run totk:local -- prepare | install | status | launch | rollback [--out <private-dir>]', note: 'Default .tmp/totk-fusion/ready. No downloads. No source files overwritten. F68 retained; Ctrl+F9 still enables free mouse camera.' };
  if (command === 'prepare') return prepare(profile, output);
  if (!['install', 'status', 'launch', 'rollback'].includes(command)) throw new Error('Unknown command.');
  await plain(output, true);
  const metadata = JSON.parse(await readFile(path.join(output, 'local.json'), 'utf8'));
  if (metadata.sourceId !== sourceId || metadata.gameId !== gameId) throw new Error('Not a prepared TOTK local source.');
  const { local } = await binaries();
  const sourceDir = path.join(output, metadata.sourceDirectory ?? 'source');
  const common = ['--source-dir', sourceDir, '--library', path.join(output, 'library'), '--game', gameId];
  if (command === 'install') {
    const workspaceRoot = path.join(repo, '.tmp', `tw-${createHash('sha256').update(output).digest('hex').slice(0, 8)}`);
    const result = await run(local, ['install', ...common, '--workspace-root', workspaceRoot]);
    const approvals = await plain(path.join(output, 'private/approved-local-files.json'));
    await run(local, ['trust', ...common, '--trust-file', approvals]);
    await copyFile(local, path.join(output, 'Play-TOTK.exe'));
    await writeFile(path.join(output, 'fusion-local-launch.json'), JSON.stringify({ source_dir: sourceDir, library: path.join(output, 'library'), game: gameId, eden: true }, null, 2));
    return { installed: true, operation: result, state: await run(local, ['status', ...common]) };
  }
  if (command === 'launch') {
    console.error('Starting TOTK through Fusion IntegrityGuard/Supervisor. Keep this command running until Eden exits. Ctrl+F9 enables free mouse camera.');
    return run(local, ['launch-eden', ...common], { wait: false });
  }
  return run(local, [command, ...common]);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).then(value => { if (value !== null) console.log(JSON.stringify(value, null, 2)); }).catch(error => { console.error(error.message); process.exitCode = 1; });
}
