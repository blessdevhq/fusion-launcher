import { cpSync, existsSync, mkdtempSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const tests = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(tests, '../..');
const scratch = path.join(root, '.tmp');
const out = path.join(root, 'out');
if (!existsSync(path.join(out, 'index.html'))) {
  throw new Error('Frontend export missing. Run npm run build from the project root first.');
}
mkdirSync(scratch, { recursive: true });
const stage = mkdtempSync(path.join(scratch, 'desktop-e2e-'));
const stageTauri = path.join(stage, 'src-tauri');
cpSync(path.join(root, 'src-tauri'), stageTauri, {
  recursive: true,
  filter: (src) => src !== path.join(root, 'src-tauri', 'target'),
});
cpSync(out, path.join(stage, 'out'), { recursive: true });
cpSync(path.join(root, 'public'), path.join(stage, 'public'), { recursive: true });

const configPath = path.join(stageTauri, 'tauri.conf.json');
const config = JSON.parse(readFileSync(configPath, 'utf8'));
const suffix = path.basename(stage).replace(/[^a-zA-Z0-9]/g, '').toLowerCase();
config.identifier = `app.fusion.launcher.e2e.${suffix}`;
config.productName = 'Fusion Launcher E2E';
config.app.withGlobalTauri = true;
config.build.beforeBuildCommand = '';
writeFileSync(configPath, JSON.stringify(config, null, 2) + '\n');

function replaceExactly(file, before, after) {
  const text = readFileSync(file, 'utf8');
  if (text.split(before).length !== 2) throw new Error(`Expected exactly one match in ${file}`);
  writeFileSync(file, text.replace(before, after));
}
replaceExactly(path.join(stageTauri, 'Cargo.toml'), '[dependencies]\n', '[dependencies]\ntauri-plugin-wdio-webdriver = "=1.5.1"\n');
const lib = path.join(stageTauri, 'src', 'lib.rs');
replaceExactly(lib, '    tauri::Builder::default()\n', '    tauri::Builder::default()\n        .plugin(tauri_plugin_wdio_webdriver::init())\n');
replaceExactly(lib, '            #[cfg(windows)]\n            let _ = deep_link::register_windows_protocol();', '            // The isolated E2E binary must never replace the user\'s fusion:// registration.');
replaceExactly(lib, '            let database_path = prepare_database_path(&data_dir).map_err(setup_error)?;', '            let database_path = data_dir.join("fusion-launcher.db"); // Never import a real legacy database in E2E.');
replaceExactly(lib, '            scraper::auto_scrape_missing_artwork(app.handle().clone(), app_state);', '            // No background artwork network requests in E2E.');

const target = path.join(scratch, 'desktop-e2e-target');
const env = { ...process.env, CARGO_TARGET_DIR: target };
console.log(`Isolated E2E build: ${stage}`);
console.log(`Isolated application identifier: ${config.identifier}`);
const cargo = spawnSync('cargo', ['build', '--manifest-path', path.join(stageTauri, 'Cargo.toml'), '--bin', 'fusion-launcher', '--features', 'tauri/custom-protocol'], { cwd: stage, env, stdio: 'inherit' });
if (cargo.error) throw cargo.error;
if (cargo.status !== 0) process.exit(cargo.status ?? 1);
const binary = path.join(target, 'debug', process.platform === 'win32' ? 'fusion-launcher.exe' : 'fusion-launcher');
const wdio = path.join(tests, 'node_modules', '@wdio', 'cli', 'bin', 'wdio.js');
const run = spawnSync(process.execPath, [wdio, 'run', path.join(tests, 'wdio.conf.mjs')], {
  cwd: tests,
  stdio: 'inherit',
  env: { ...env, FUSION_E2E_BINARY: binary, FUSION_E2E_ARTIFACTS: path.join(stage, 'artifacts') },
});
if (run.error) throw run.error;
console.log(`E2E artifacts: ${path.join(stage, 'artifacts')}`);
process.exit(run.status ?? 1);
