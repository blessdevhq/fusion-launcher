import type { FusionAddon, FusionError, FusionGameOptions, FusionGameState, FusionInstallInput } from '../types/fusionSource.ts';
import { fusionLayerTitle, selectableFusionAddons } from './fusionGamePresentation.ts';

export type FusionInputKinds = Record<string, 'file' | 'directory'>;

export function fusionSetupDependency(error: FusionError | null, addons: FusionAddon[]) {
  const id = error?.details.dependency;
  if (typeof id !== 'string' || !id) return null;
  return fusionLayerTitle(addons.find((addon) => addon.id === id) ?? { id });
}

export function fusionSetupRequest(state: FusionGameState, options: FusionGameOptions, input: FusionInstallInput, kinds: FusionInputKinds = {}): FusionInstallInput {
  const active = new Set(fusionSetupRows(state, options, input, kinds).filter((row) => row.active).map((row) => row.layerId));
  return {
    userFiles: Object.fromEntries(Object.entries(input.userFiles).filter(([id, path]) => active.has(id) && path)),
    rawPaths: Object.fromEntries(Object.entries(input.rawPaths).filter(([id, path]) => active.has(id) && kinds[id] !== 'directory' && path.trim())),
    externalWorkspaceRoot: state.installed ? state.externalWorkspaceRoot : input.externalWorkspaceRoot?.trim() || null,
  };
}

export function fusionSetupRows(state: FusionGameState, options: FusionGameOptions, input: FusionInstallInput, kinds: FusionInputKinds = {}, error: FusionError | null = null) {
  const selectable = new Set(selectableFusionAddons(state.addons).map((addon) => addon.id));
  return state.userLayers.map((layer) => {
    const addon = state.addons.find((item) => item.id === layer.layerId);
    const optional = layer.optional ?? Boolean(addon);
    const active = !selectable.has(layer.layerId) || (options.enabledAddons?.includes(layer.layerId) ?? addon?.enabled ?? false);
    const path = input.userFiles[layer.layerId] ?? '';
    return {
      ...layer, optional, active, path,
      title: fusionLayerTitle({ id: layer.layerId, title: layer.title ?? addon?.title }),
      error: active && error?.details.layerId === layer.layerId ? error.message : null,
      dependencyTitle: active && error?.details.layerId === layer.layerId ? fusionSetupDependency(error, state.addons) : null,
      status: !active ? 'disabled' : path ? 'selected' : 'not_selected',
      destinationRequired: layer.rawPathRequired && kinds[layer.layerId] === 'file',
    };
  });
}

type SetupAction = { kind: 'choose'; layerId: string } | { kind: 'install' | 'repair' | 'review' | 'play' };

export function fusionSetupPrimaryAction(
  state: Pick<FusionGameState, 'installed' | 'activeSnapshotId' | 'ready'>,
  rows: Pick<ReturnType<typeof fusionSetupRows>[number], 'active' | 'optional' | 'path' | 'layerId'>[],
  { blocked, repair, trust }: { blocked: boolean; repair: boolean; trust: boolean },
): SetupAction | null {
  if (blocked) return null;
  if (repair) return { kind: 'repair' };
  if (trust) return { kind: 'review' };
  if (state.ready) return { kind: 'play' };
  // Existing snapshots may reuse imports without a fresh path selection.
  if (!state.installed && !state.activeSnapshotId) {
    const missing = rows.find((row) => row.active && !row.optional && !row.path);
    if (missing) return { kind: 'choose', layerId: missing.layerId };
  }
  return { kind: 'install' };
}
