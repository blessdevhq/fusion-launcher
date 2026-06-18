import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  countConfiguredEmulators,
  getEmulatorDraftState,
  getEmulatorSaveIntent,
  hasEmulatorDraftChanges,
  updateDraftEmulatorPath
} from './settingsModalState.ts';
import type { AppSettings } from './settings.ts';

const savedSettings: AppSettings = {
  emulators: {
    nes: 'C:/Emulators/Mesen.exe',
    ps1: 'C:/Emulators/DuckStation.exe'
  },
  emulatorConfigs: {
    nes: {
      platform: 'nes',
      exePath: 'C:/Emulators/Mesen.exe',
      status: 'valid'
    },
    ps1: {
      platform: 'ps1',
      exePath: 'C:/Emulators/DuckStation.exe',
      status: 'missing'
    }
  },
  language: 'en'
};

describe('settings modal draft state', () => {
  it('updates only the selected platform path', () => {
    const draft = updateDraftEmulatorPath(savedSettings, 'nes', 'D:/Tools/Mesen.exe');

    assert.equal(draft.emulators.nes, 'D:/Tools/Mesen.exe');
    assert.equal(draft.emulators.ps1, savedSettings.emulators.ps1);
    assert.equal(savedSettings.emulators.nes, 'C:/Emulators/Mesen.exe');
  });

  it('marks changed paths as unsaved until Save Changes persists them', () => {
    const draft = updateDraftEmulatorPath(savedSettings, 'nes', 'D:/Tools/Mesen.exe');
    const state = getEmulatorDraftState(draft, savedSettings, 'nes');

    assert.equal(state.label, 'Unsaved');
    assert.equal(state.tone, 'unsaved');
    assert.equal(state.saveIntent, 'save');
    assert.equal(hasEmulatorDraftChanges(draft, savedSettings), true);
  });

  it('marks cleared saved paths as delete-on-save', () => {
    const draft = updateDraftEmulatorPath(savedSettings, 'nes', '');
    const state = getEmulatorDraftState(draft, savedSettings, 'nes');

    assert.equal(state.label, 'Remove on save');
    assert.equal(state.tone, 'unsaved');
    assert.equal(getEmulatorSaveIntent(draft, savedSettings, 'nes'), 'delete');
  });

  it('reflects saved backend validation states', () => {
    assert.equal(getEmulatorDraftState(savedSettings, savedSettings, 'nes').label, 'Ready');
    assert.equal(getEmulatorDraftState(savedSettings, savedSettings, 'ps1').label, 'File moved');
  });

  it('counts configured emulator manager paths', () => {
    assert.equal(countConfiguredEmulators(savedSettings), 2);
  });
});
