import { create } from 'zustand';
import { fusionGameKey, type FusionCatalogEntry, type FusionGameState, type FusionOperationProgress, type FusionSourceSummary } from '../types/fusionSource.ts';

const INTEGRITY_ERROR_CODES = new Set(['untrusted_executable', 'missing_injector_dll', 'corrupted_critical_binary', 'local_base_modified', 'snapshot_conflict']);

export interface FusionReadinessBlocker {
  activeSnapshotId: string;
}

interface FusionSourceState {
  sources: FusionSourceSummary[];
  catalog: FusionCatalogEntry[];
  operations: Record<string, FusionOperationProgress>;
  retiredOperationIds: string[];
  readinessBlockers: Record<string, FusionReadinessBlocker>;
  setSourcesAndCatalog: (sources: FusionSourceSummary[], catalog: FusionCatalogEntry[]) => void;
  mergeOperation: (event: FusionOperationProgress) => void;
  recordGameState: (game: FusionGameState) => void;
  recordIntegrityError: (game: Pick<FusionCatalogEntry, 'sourceId' | 'gameId'>, activeSnapshotId: string | null, error: unknown) => void;
}

/** Fusion progress has its own identity and never marks a legacy download installed. */
export const useFusionSourceStore = create<FusionSourceState>((set) => ({
  sources: [],
  catalog: [],
  operations: {},
  retiredOperationIds: [],
  readinessBlockers: {},
  setSourcesAndCatalog: (sources, catalog) => set((state) => {
    const snapshots = new Map(catalog.map((game) => [fusionGameKey(game), game.activeSnapshotId]));
    const readinessBlockers = Object.fromEntries(Object.entries(state.readinessBlockers)
      .filter(([key, blocker]) => snapshots.get(key) === blocker.activeSnapshotId));
    return { sources, catalog, readinessBlockers };
  }),
  recordGameState: (game) => set((state) => {
    const key = fusionGameKey(game);
    const readinessBlockers = { ...state.readinessBlockers };
    if (game.installed && game.activeSnapshotId && game.blockers.length > 0) {
      readinessBlockers[key] = { activeSnapshotId: game.activeSnapshotId };
    } else if (!game.installed || readinessBlockers[key]?.activeSnapshotId === game.activeSnapshotId) {
      // A successful full state check can clear a previous guard failure. It
      // never promotes readiness above the independently refreshed catalog.
      delete readinessBlockers[key];
    }
    return { readinessBlockers };
  }),
  recordIntegrityError: (game, activeSnapshotId, error) => set((state) => {
    if (!activeSnapshotId || typeof error !== 'object' || error === null || !('code' in error)
      || typeof error.code !== 'string' || !INTEGRITY_ERROR_CODES.has(error.code)) return state;
    return { readinessBlockers: { ...state.readinessBlockers, [fusionGameKey(game)]: { activeSnapshotId } } };
  }),
  mergeOperation: (event) => set((state) => {
    const key = fusionGameKey(event);
    const current = state.operations[key];
    if (state.retiredOperationIds.includes(event.operationId)) return state;
    if (current?.operationId === event.operationId && (event.sequence <= current.sequence || isFusionOperationTerminal(current))) return state;
    const activeSnapshotId = state.catalog.find((game) => fusionGameKey(game) === key)?.activeSnapshotId;
    return {
      operations: { ...state.operations, [key]: event },
      readinessBlockers: event.error && INTEGRITY_ERROR_CODES.has(event.error.code) && activeSnapshotId
        ? { ...state.readinessBlockers, [key]: { activeSnapshotId } }
        : state.readinessBlockers,
      retiredOperationIds: current && current.operationId !== event.operationId
        ? [current.operationId, ...state.retiredOperationIds].slice(0, 512)
        : state.retiredOperationIds
    };
  })
}));

export function isFusionOperationTerminal(operation: FusionOperationProgress) {
  return ['completed', 'failed', 'cancelled'].includes(operation.status);
}
