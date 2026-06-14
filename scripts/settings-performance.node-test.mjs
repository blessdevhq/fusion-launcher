import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const settingsModalPath = new URL('../src/components/SettingsModal.tsx', import.meta.url);
const commandsPath = new URL('../src-tauri/src/commands/diagnostics.rs', import.meta.url);

test('settings modal opening stays lightweight', async () => {
  const source = await readFile(settingsModalPath, 'utf8');

  assert.match(source, /api\.getDiagnosticsPaths\(\)/);
  assert.doesNotMatch(source, /api\.runHealthCheck\(\)/);
  assert.doesNotMatch(source, /api\.getDiagnosticsBundle\(\)/);
  assert.doesNotMatch(source, /backdrop-blur/);
  assert.doesNotMatch(source, /<motion\./);
});

test('health and diagnostics work runs outside the UI thread', async () => {
  const source = await readFile(commandsPath, 'utf8');

  assert.match(
    source,
    /pub async fn run_health_check[\s\S]*?spawn_blocking\(move \|\| build_health_report\(&state\)\)/
  );
  assert.match(
    source,
    /pub async fn get_diagnostics_bundle[\s\S]*?spawn_blocking\(move \|\| build_diagnostics_bundle\(&state\)\)/
  );
});
