import { Store } from '@tauri-apps/plugin-store';
import { MVP_PLATFORMS, PLATFORMS, type Platform } from '../types/platform.ts';
import type { EmulatorConfig } from '../types/repository.ts';
import { api } from './api.ts';
import { DEFAULT_LOCALE, normalizeLocale, type Locale } from './i18n.ts';
import { isPreviewRuntime, isTauriRuntime, requireDesktopBridge } from './runtime.ts';

const SETTINGS_FILE = 'settings.json';
const EMULATORS_KEY = 'emulators';
const LANGUAGE_KEY = 'language';
const METADATA_ONBOARDING_KEY = 'metadataOnboarding';
const LEGACY_DEFAULT_EMULATOR_PATH_KEY = 'defaultEmulatorPath';
const PREVIEW_SETTINGS_KEY = 'fusion-launcher.preview.settings';
const LEGACY_PREVIEW_SETTINGS_KEY = `${['retro', 'hydra'].join('')}.preview.settings`;

export type EmulatorPaths = Partial<Record<Platform, string>>;
export type EmulatorConfigs = Partial<Record<Platform, EmulatorConfig>>;
export type MetadataOnboardingStrategy = 'source' | 'screenscraper' | 'manual';

export interface MetadataOnboardingSettings {
  complete: boolean;
  strategy: MetadataOnboardingStrategy;
}

export interface AppSettings {
  emulators: EmulatorPaths;
  emulatorConfigs: EmulatorConfigs;
  language: Locale;
  metadataOnboarding: MetadataOnboardingSettings;
}

export const DEFAULT_SETTINGS: AppSettings = {
  emulators: {},
  emulatorConfigs: {},
  language: DEFAULT_LOCALE,
  metadataOnboarding: {
    complete: false,
    strategy: 'source'
  }
};

let settingsStore: Promise<Store> | null = null;
let previewSettings: AppSettings = DEFAULT_SETTINGS;

function getSettingsStore() {
  settingsStore ??= Store.load(SETTINGS_FILE, {
    defaults: {
      [EMULATORS_KEY]: DEFAULT_SETTINGS.emulators,
      [LANGUAGE_KEY]: DEFAULT_SETTINGS.language,
      [METADATA_ONBOARDING_KEY]: DEFAULT_SETTINGS.metadataOnboarding
    },
    autoSave: true
  });

  return settingsStore;
}

export async function loadSettings(): Promise<AppSettings> {
  if (!isTauriRuntime()) {
    if (isPreviewRuntime()) {
      const previewSettings = loadPreviewSettings();
      const configs = await api.listEmulatorConfigs();
      if (configs.length > 0) {
        return settingsFromEmulatorConfigs(configs, previewSettings.language, previewSettings.metadataOnboarding);
      }
      return previewSettings;
    }
    return requireDesktopBridge('Loading settings');
  }

  await migrateLegacyEmulatorSettings();
  const configs = await api.listEmulatorConfigs();
  return settingsFromEmulatorConfigs(configs, await loadStoredLanguage(), await loadStoredMetadataOnboarding());
}

export async function saveSettings(settings: AppSettings): Promise<AppSettings> {
  const normalizedSettings = normalizeSettings(settings);

  if (!isTauriRuntime()) {
    if (isPreviewRuntime()) {
      await Promise.all(MVP_PLATFORMS.map((platform) => {
        const emulatorPath = normalizedSettings.emulators[platform];
        if (emulatorPath) {
          return api.saveEmulatorConfig(
            platform,
            emulatorPath,
            normalizedSettings.emulatorConfigs[platform]?.launchArgsTemplate
          );
        }

        return api.deleteEmulatorConfig(platform);
      }));
      return savePreviewSettings(settingsFromEmulatorConfigs(
        await api.listEmulatorConfigs(),
        normalizedSettings.language,
        normalizedSettings.metadataOnboarding
      ));
    }
    return requireDesktopBridge('Saving settings');
  }

  await Promise.all(MVP_PLATFORMS.map((platform) => {
    const emulatorPath = normalizedSettings.emulators[platform];
    if (emulatorPath) {
      return api.saveEmulatorConfig(
        platform,
        emulatorPath,
        normalizedSettings.emulatorConfigs[platform]?.launchArgsTemplate
      );
    }

    return api.deleteEmulatorConfig(platform);
  }));

  await saveStoredLanguage(normalizedSettings.language);
  await saveStoredMetadataOnboarding(normalizedSettings.metadataOnboarding);

  return settingsFromEmulatorConfigs(
    await api.listEmulatorConfigs(),
    normalizedSettings.language,
    normalizedSettings.metadataOnboarding
  );
}

async function migrateLegacyEmulatorSettings(): Promise<void> {
  const store = await getSettingsStore();
  const legacyEmulators = normalizeEmulators(await store.get<unknown>(EMULATORS_KEY));
  const legacyDefaultEmulatorPath = await store.get<unknown>(LEGACY_DEFAULT_EMULATOR_PATH_KEY);
  const existingConfigs = await api.listEmulatorConfigs();
  const existingPlatforms = new Set(existingConfigs.map((config) => config.platform));

  if (
    Object.keys(legacyEmulators).length === 0
    && typeof legacyDefaultEmulatorPath === 'string'
    && legacyDefaultEmulatorPath.trim()
  ) {
    legacyEmulators.switch = legacyDefaultEmulatorPath.trim();
  }

  const migrations = MVP_PLATFORMS.flatMap((platform) => {
    const emulatorPath = legacyEmulators[platform];
    if (!emulatorPath || existingPlatforms.has(platform)) return [];
    return [api.saveEmulatorConfig(platform, emulatorPath)];
  });

  if (migrations.length > 0) {
    await Promise.all(migrations);
  }

  await store.delete(EMULATORS_KEY);
  await store.delete(LEGACY_DEFAULT_EMULATOR_PATH_KEY);
  await store.save();
}

async function loadStoredLanguage(): Promise<Locale> {
  const store = await getSettingsStore();
  return normalizeLocale(await store.get<unknown>(LANGUAGE_KEY));
}

async function saveStoredLanguage(language: Locale): Promise<void> {
  const store = await getSettingsStore();
  await store.set(LANGUAGE_KEY, language);
  await store.save();
}

async function loadStoredMetadataOnboarding(): Promise<MetadataOnboardingSettings> {
  const store = await getSettingsStore();
  return normalizeMetadataOnboarding(await store.get<unknown>(METADATA_ONBOARDING_KEY));
}

async function saveStoredMetadataOnboarding(metadataOnboarding: MetadataOnboardingSettings): Promise<void> {
  const store = await getSettingsStore();
  await store.set(METADATA_ONBOARDING_KEY, metadataOnboarding);
  await store.save();
}

export function getEmulatorPath(settings: AppSettings, platform: Platform): string {
  return settings.emulatorConfigs[platform]?.exePath?.trim() ?? settings.emulators[platform]?.trim() ?? '';
}

export function getEmulatorConfig(settings: AppSettings, platform: Platform): EmulatorConfig | undefined {
  return settings.emulatorConfigs[platform];
}

export function setEmulatorPath(
  settings: AppSettings,
  platform: Platform,
  emulatorPath: string
): AppSettings {
  const trimmed = emulatorPath.trim();
  const existingConfig = settings.emulatorConfigs[platform];

  return {
    ...settings,
    emulators: {
      ...settings.emulators,
      [platform]: trimmed
    },
    emulatorConfigs: {
      ...settings.emulatorConfigs,
      [platform]: existingConfig
        ? { ...existingConfig, exePath: trimmed }
        : undefined
    }
  };
}

function settingsFromEmulatorConfigs(
  configs: EmulatorConfig[],
  language: Locale = DEFAULT_SETTINGS.language,
  metadataOnboarding: unknown = DEFAULT_SETTINGS.metadataOnboarding
): AppSettings {
  const emulatorConfigs: EmulatorConfigs = {};
  const emulators: EmulatorPaths = {};

  for (const config of configs) {
    if (!PLATFORMS.includes(config.platform)) continue;
    emulatorConfigs[config.platform] = config;
    if (config.exePath?.trim()) {
      emulators[config.platform] = config.exePath.trim();
    }
  }

  return { emulators, emulatorConfigs, language, metadataOnboarding: normalizeMetadataOnboarding(metadataOnboarding) };
}

export function normalizeSettings(settings: Partial<AppSettings>): AppSettings {
  const emulators = normalizeEmulators(settings.emulators);
  const emulatorConfigs: EmulatorConfigs = {};

  for (const platform of PLATFORMS) {
    const config = settings.emulatorConfigs?.[platform];
    if (config) {
      emulatorConfigs[platform] = {
        ...config,
        exePath: emulators[platform] ?? config.exePath?.trim()
      };
    }
  }

  return {
    emulators,
    emulatorConfigs,
    language: normalizeLocale(settings.language),
    metadataOnboarding: normalizeMetadataOnboarding(settings.metadataOnboarding)
  };
}

function normalizeEmulators(value: unknown): EmulatorPaths {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return {};
  }

  const input = value as Record<string, unknown>;
  const emulators: EmulatorPaths = {};

  for (const platform of PLATFORMS) {
    const emulatorPath = input[platform];
    if (typeof emulatorPath === 'string' && emulatorPath.trim()) {
      emulators[platform] = emulatorPath.trim();
    }
  }

  return emulators;
}

function loadPreviewSettings(): AppSettings {
  if (typeof window === 'undefined') return previewSettings;

  try {
    const raw = window.localStorage.getItem(PREVIEW_SETTINGS_KEY) ?? window.localStorage.getItem(LEGACY_PREVIEW_SETTINGS_KEY);
    if (!raw) return previewSettings;

    const parsed = JSON.parse(raw) as Partial<AppSettings>;
    previewSettings = normalizeSettings({
      emulators: parsed.emulators,
      emulatorConfigs: normalizeEmulatorConfigs(parsed.emulatorConfigs),
      language: parsed.language,
      metadataOnboarding: parsed.metadataOnboarding
    });
  } catch {
    previewSettings = DEFAULT_SETTINGS;
  }

  return previewSettings;
}

function savePreviewSettings(settings: AppSettings): AppSettings {
  previewSettings = normalizeSettings(settings);

  if (typeof window !== 'undefined') {
    window.localStorage.setItem(PREVIEW_SETTINGS_KEY, JSON.stringify(previewSettings));
    window.localStorage.removeItem(LEGACY_PREVIEW_SETTINGS_KEY);
  }

  return previewSettings;
}

function normalizeMetadataOnboarding(value: unknown): MetadataOnboardingSettings {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return DEFAULT_SETTINGS.metadataOnboarding;
  }

  const input = value as Partial<MetadataOnboardingSettings>;
  const strategy = input.strategy === 'screenscraper' || input.strategy === 'manual'
    ? input.strategy
    : DEFAULT_SETTINGS.metadataOnboarding.strategy;

  return {
    complete: input.complete === true,
    strategy
  };
}

function normalizeEmulatorConfigs(value: unknown): EmulatorConfigs {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return {};
  }

  const input = value as Record<string, unknown>;
  const configs: EmulatorConfigs = {};

  for (const platform of PLATFORMS) {
    const config = input[platform];
    if (!config || typeof config !== 'object' || Array.isArray(config)) continue;
    const record = config as Partial<EmulatorConfig>;
    const exePath = typeof record.exePath === 'string' ? record.exePath.trim() : undefined;
    configs[platform] = {
      platform,
      exePath,
      status: record.status === 'missing' || record.status === 'invalid' ? record.status : 'valid',
      lastValidatedAt: typeof record.lastValidatedAt === 'string' ? record.lastValidatedAt : undefined,
      version: typeof record.version === 'string' ? record.version : undefined,
      launchArgsTemplate: typeof record.launchArgsTemplate === 'string' ? record.launchArgsTemplate : undefined
    };
  }

  return configs;
}
