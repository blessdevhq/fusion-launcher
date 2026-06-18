import { getEmulatorConfig, getEmulatorPath, setEmulatorPath, type AppSettings } from './settings.ts';
import { EMULATOR_MANAGER_PLATFORMS, type Platform } from '../types/platform.ts';
import { DEFAULT_LOCALE, getUiText, type Locale } from './i18n.ts';

export type EmulatorSaveIntent = 'unchanged' | 'save' | 'delete';
export type EmulatorDraftTone = 'empty' | 'unsaved' | 'valid' | 'missing' | 'invalid';

export interface EmulatorDraftState {
  label: string;
  tone: EmulatorDraftTone;
  saveIntent: EmulatorSaveIntent;
  detail: string;
}

export function updateDraftEmulatorPath(
  settings: AppSettings,
  platform: Platform,
  executablePath: string
): AppSettings {
  return setEmulatorPath(settings, platform, executablePath);
}

export function getEmulatorSaveIntent(
  draftSettings: AppSettings,
  savedSettings: AppSettings,
  platform: Platform
): EmulatorSaveIntent {
  const draftPath = getEmulatorPath(draftSettings, platform);
  const savedPath = getEmulatorPath(savedSettings, platform);

  if (draftPath === savedPath) return 'unchanged';
  return draftPath ? 'save' : 'delete';
}

export function getEmulatorDraftState(
  draftSettings: AppSettings,
  savedSettings: AppSettings,
  platform: Platform,
  locale: Locale = DEFAULT_LOCALE
): EmulatorDraftState {
  const text = getUiText(locale).settings.emulatorDraft;
  const draftPath = getEmulatorPath(draftSettings, platform);
  const savedPath = getEmulatorPath(savedSettings, platform);
  const saveIntent = getEmulatorSaveIntent(draftSettings, savedSettings, platform);

  if (!draftPath && savedPath) {
    return {
      label: text.removeOnSave.label,
      tone: 'unsaved',
      saveIntent,
      detail: text.removeOnSave.detail
    };
  }

  if (!draftPath) {
    return {
      label: text.notSet.label,
      tone: 'empty',
      saveIntent,
      detail: text.notSet.detail
    };
  }

  if (draftPath !== savedPath) {
    return {
      label: text.unsaved.label,
      tone: 'unsaved',
      saveIntent,
      detail: text.unsaved.detail
    };
  }

  const savedConfig = getEmulatorConfig(savedSettings, platform);
  if (savedConfig?.status === 'valid') {
    return {
      label: text.ready.label,
      tone: 'valid',
      saveIntent,
      detail: text.ready.detail
    };
  }

  if (savedConfig?.status === 'missing') {
    return {
      label: text.fileMoved.label,
      tone: 'missing',
      saveIntent,
      detail: text.fileMoved.detail
    };
  }

  if (savedConfig?.status === 'invalid') {
    return {
      label: text.invalid.label,
      tone: 'invalid',
      saveIntent,
      detail: text.invalid.detail
    };
  }

  return {
    label: text.saved.label,
    tone: 'valid',
    saveIntent,
    detail: text.saved.detail
  };
}

export function hasEmulatorDraftChanges(
  draftSettings: AppSettings,
  savedSettings: AppSettings
): boolean {
  return EMULATOR_MANAGER_PLATFORMS.some((platform) => (
    getEmulatorPath(draftSettings, platform) !== getEmulatorPath(savedSettings, platform)
  ));
}

export function countConfiguredEmulators(settings: AppSettings): number {
  return EMULATOR_MANAGER_PLATFORMS.filter((platform) => Boolean(getEmulatorPath(settings, platform))).length;
}
