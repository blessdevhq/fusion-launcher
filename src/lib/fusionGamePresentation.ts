import type { FusionAddon, FusionError, FusionGameOptions, FusionGameState, FusionOperationProgress } from '../types/fusionSource.ts';

const selectableKinds = new Set(['addon', 'mod', 'shader_cache']);
const integrityCodes = new Set(['missing_injector_dll', 'corrupted_critical_binary', 'local_base_modified']);

export function selectableFusionAddons(addons: FusionAddon[]) {
  return addons.filter((addon) => selectableKinds.has(addon.kind));
}

export function fusionLayerTitle(layer: { id: string; title?: string | null }) {
  if (layer.title?.trim()) return layer.title.trim();
  const label = layer.id.replace(/^(?:addon|mod|shader[-_]cache)[-_]/i, '').replace(/[-_]+/g, ' ').trim();
  return (label || layer.id).replace(/\b(\d+)\s*fps\b/gi, '$1 FPS').replace(/(^|\s)([a-z])/g, (_, space: string, letter: string) => `${space}${letter.toUpperCase()}`);
}

export function effectiveFusionOptions(state: FusionGameState, options = state.options): FusionGameOptions {
  return {
    enabledAddons: [...new Set(options.enabledAddons ?? selectableFusionAddons(state.addons).filter((addon) => addon.enabled).map((addon) => addon.id))].sort(),
    runtimeChannel: options.runtimeChannel ?? state.defaultRuntimeChannel ?? null,
  };
}

export function fusionSettingsDiffer(state: FusionGameState, draft: FusionGameOptions, workspaceRoot: string | null) {
  const current = effectiveFusionOptions(state, draft);
  const baseline = effectiveFusionOptions(state, state.installed ? state.appliedOptions ?? state.options : state.options);
  return JSON.stringify(current) !== JSON.stringify(baseline)
    || (workspaceRoot?.trim() || null) !== (state.externalWorkspaceRoot?.trim() || null);
}

export function fusionHasVersionUpdate(state: Pick<FusionGameState, 'installed' | 'activeVersion' | 'version'>) {
  return state.installed && state.activeVersion !== null && state.activeVersion !== state.version;
}

export function fusionStructuredError(error: unknown): FusionError | null {
  if (typeof error !== 'object' || error === null || !('code' in error) || typeof error.code !== 'string' || !('message' in error) || typeof error.message !== 'string') return null;
  return { code: error.code, message: error.message, details: 'details' in error && typeof error.details === 'object' && error.details !== null ? error.details as Record<string, unknown> : {} };
}

export function fusionIsIntegrityError(error: Pick<FusionError, 'code'> | null | undefined) {
  return Boolean(error && integrityCodes.has(error.code));
}

export function fusionOperationVisible(operation: FusionOperationProgress | null | undefined) {
  return Boolean(operation && ['running', 'cancelling', 'failed'].includes(operation.status));
}

export function fusionProgressRatio(transfer: FusionOperationProgress['transfer']) {
  if (!transfer || !Number.isFinite(transfer.totalBytes) || transfer.totalBytes <= 0) return null;
  const downloaded = Number.isFinite(transfer.downloadedBytes) ? transfer.downloadedBytes : 0;
  return Math.max(0, Math.min(1, downloaded / transfer.totalBytes));
}

export function formatFusionBytes(value: number, locale: 'ru' | 'en') {
  const bytes = Number.isFinite(value) ? Math.max(0, value) : 0;
  const units = locale === 'ru' ? ['Б', 'КБ', 'МБ', 'ГБ', 'ТБ'] : ['B', 'KB', 'MB', 'GB', 'TB'];
  const unit = bytes > 0 ? Math.max(0, Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1)) : 0;
  return `${new Intl.NumberFormat(locale, { maximumFractionDigits: unit === 0 ? 0 : 1 }).format(bytes / 1024 ** unit)} ${units[unit]}`;
}
