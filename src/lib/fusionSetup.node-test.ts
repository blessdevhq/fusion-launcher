import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { fusionSetupPrimaryAction, fusionSetupRequest, fusionSetupRows } from './fusionSetup.ts';
import type { FusionGameState } from '../types/fusionSource.ts';

function fixture(): FusionGameState {
  return {
    sourceId: 'source', gameId: 'game', title: 'Game', version: '1', platform: 'pc',
    installed: false, activeVersion: null, ready: false, updateAvailable: false,
    activeSnapshotId: null, rollbackAvailable: false,
    options: { enabledAddons: [], runtimeChannel: null }, appliedOptions: null,
    defaultRuntimeChannel: null, externalWorkspaceRoot: null,
    addons: [{ id: 'coop', title: 'Cooperative mode', description: null, kind: 'addon', enabled: false }],
    runtimeChannels: [], executables: [], blockers: [], warnings: [], operation: null, crashReportIds: [], crashReports: [],
    userLayers: [
      { layerId: 'base', title: 'Your game', optional: false, instructions: 'Use your own files.', archive: 'raw', rawPathRequired: true },
      { layerId: 'coop', title: 'Cooperative content', optional: true, instructions: null, archive: 'zip', rawPathRequired: false },
    ],
  };
}

const emptyInput = () => ({ userFiles: {}, rawPaths: {}, externalWorkspaceRoot: null });
const noBlockers = { blocked: false, repair: false, trust: false };

it('makes a missing required input the first action on a new installation', () => {
  const state = fixture();
  const rows = fusionSetupRows(state, state.options, emptyInput());
  assert.deepEqual(fusionSetupPrimaryAction(state, rows, noBlockers), { kind: 'choose', layerId: 'base' });
});

it('offers install after selecting a required path even with an unselected optional file', () => {
  const state = fixture();
  const input = { ...emptyInput(), userFiles: { base: 'D:/Owned/Game' } };
  const rows = fusionSetupRows(state, state.options, input);
  assert.deepEqual(fusionSetupPrimaryAction(state, rows, noBlockers), { kind: 'install' });
  assert.equal(rows[0].status, 'selected');
});

it('does not require a fresh file choice to repair an existing snapshot', () => {
  const state = { ...fixture(), installed: true, activeSnapshotId: 'current' };
  const rows = fusionSetupRows(state, state.options, emptyInput());
  assert.deepEqual(fusionSetupPrimaryAction(state, rows, { ...noBlockers, repair: true }), { kind: 'repair' });
});

it('routes a newer trust blocker to review rather than launch', () => {
  const state = { ...fixture(), installed: true, ready: true };
  assert.deepEqual(fusionSetupPrimaryAction(state, [], { ...noBlockers, trust: true }), { kind: 'review' });
});

it('keeps play primary for a ready installation without a draft import path', () => {
  const state = { ...fixture(), installed: true, ready: true };
  assert.deepEqual(fusionSetupPrimaryAction(state, fusionSetupRows(state, state.options, emptyInput()), noBlockers), { kind: 'play' });
});

describe('Fusion setup inputs', () => {
  it('shows optional file requirements without making a disabled addon required', () => {
    const state = fixture();
    const rows = fusionSetupRows(state, state.options, emptyInput());
    assert.deepEqual(rows.map(({ layerId, active }) => [layerId, active]), [['base', true], ['coop', false]]);
  });

  it('does not demand a raw-file destination for a selected directory', () => {
    const state = fixture();
    const input = { ...emptyInput(), userFiles: { base: 'D:/Owned/Game' } };
    const [row] = fusionSetupRows(state, state.options, input, { base: 'directory' });
    assert.equal(row.destinationRequired, false);
    assert.equal(row.status, 'selected');
    assert.equal(row.path, 'D:/Owned/Game');
  });

  it('keeps plan-selected non-selectable optional layers active', () => {
    const state = fixture();
    state.addons.push({ id: 'patch', title: null, description: null, kind: 'patch', enabled: true });
    state.userLayers.push({ layerId: 'patch', optional: true, title: null, instructions: null, archive: 'raw', rawPathRequired: false });
    assert.equal(fusionSetupRows(state, state.options, emptyInput()).find((row) => row.layerId === 'patch')?.active, true);
  });

  it('submits only active imports and never submits a stale raw destination for a directory', () => {
    const state = fixture();
    const input = { userFiles: { base: 'D:/Owned/Game', coop: 'D:/Owned/coop.zip' }, rawPaths: { base: '../stale.bin', coop: 'unused.bin' }, externalWorkspaceRoot: ' D:/Workspace ' };
    const before = structuredClone(input);
    assert.deepEqual(fusionSetupRequest(state, state.options, input, { base: 'directory' }), {
      userFiles: { base: 'D:/Owned/Game' }, rawPaths: {}, externalWorkspaceRoot: 'D:/Workspace',
    });
    assert.deepEqual(input, before);
  });

  it('attaches an import failure only to its explicitly identified layer', () => {
    const state = fixture();
    const error = { code: 'invalid_digest', message: 'This file does not match the source.', details: { layerId: 'base' } };
    const rows = fusionSetupRows(state, state.options, emptyInput(), {}, error);
    assert.equal(rows[0].error, error.message);
    assert.equal(rows[1].error, null);
  });

  it('uses null addon defaults but respects an explicitly empty selection', () => {
    const state = fixture();
    state.addons[0].enabled = true;
    assert.equal(fusionSetupRows(state, { enabledAddons: null, runtimeChannel: null }, emptyInput())[1].active, true);
    assert.equal(fusionSetupRows(state, state.options, emptyInput())[1].active, false);
  });

  it('keeps selections in the draft when an addon is disabled and re-enabled', () => {
    const state = fixture();
    const input = { ...emptyInput(), userFiles: { coop: 'D:/Owned/co-op.zip' } };
    const before = structuredClone({ state, input });
    assert.equal(fusionSetupRows(state, state.options, input)[1].status, 'disabled');
    const enabled = { ...state.options, enabledAddons: ['coop'] };
    assert.equal(fusionSetupRows(state, enabled, input)[1].path, input.userFiles.coop);
    assert.deepEqual({ state, input }, before);
  });

  it('keeps original path bytes and locks an installed workspace to its actual root', () => {
    const state = fixture();
    state.installed = true;
    state.externalWorkspaceRoot = 'F:/Actual Workspace';
    const input = { userFiles: { base: 'F:/Owned/Йоши é.bin' }, rawPaths: { base: 'game/Йоши.bin' }, externalWorkspaceRoot: 'D:/Wrong Workspace' };
    const request = fusionSetupRequest(state, state.options, input, { base: 'file' });
    assert.equal(request.userFiles.base, input.userFiles.base);
    assert.equal(request.rawPaths.base, input.rawPaths.base);
    assert.equal(request.externalWorkspaceRoot, 'F:/Actual Workspace');
  });

  it('does not falsely blame one input for a generic operation failure', () => {
    const state = fixture();
    const error = { code: 'network', message: 'Network unavailable', details: {} };
    assert.ok(fusionSetupRows(state, state.options, emptyInput(), {}, error).every((row) => row.error === null));
  });

  it('identifies a disabled dependency instead of advising replacement of a valid input', () => {
    const state = fixture();
    state.addons.push({ id: 'support', title: 'Support package', description: null, kind: 'addon', enabled: false });
    const error = { code: 'missing_layer', message: 'A selected layer depends on a disabled or unavailable layer.', details: { layerId: 'coop', dependency: 'support' } };
    const options = { ...state.options, enabledAddons: ['coop'] };
    const row = fusionSetupRows(state, options, emptyInput(), {}, error)[1];
    assert.equal(row.dependencyTitle, 'Support package');
  });

  it('uses source titles with a legacy metadata fallback', () => {
    const state = fixture();
    delete state.userLayers[1].title;
    delete state.userLayers[1].optional;
    const rows = fusionSetupRows(state, state.options, emptyInput());
    assert.equal(rows[0].title, 'Your game');
    assert.equal(rows[1].title, 'Cooperative mode');
    assert.equal(rows[1].optional, true);
  });
});
