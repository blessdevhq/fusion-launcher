import { create } from 'zustand';
import { fusionGameKey, type FusionCatalogEntry, type FusionOperationProgress, type FusionSourceSummary } from '../types/fusionSource.ts';

interface FusionSourceState {
  sources: FusionSourceSummary[];
  catalog: FusionCatalogEntry[];
  operations: Record<string, FusionOperationProgress>;
  retiredOperationIds: string[];
  setSourcesAndCatalog: (sources: FusionSourceSummary[], catalog: FusionCatalogEntry[]) => void;
  mergeOperation: (event: FusionOperationProgress) => void;
}

/** Fusion progress has its own identity and never marks a legacy download installed. */
export const useFusionSourceStore = create<FusionSourceState>((set) => ({
  sources: [],
  catalog: [],
  operations: {},
  retiredOperationIds: [],
  setSourcesAndCatalog: (sources, catalog) => set({ sources, catalog }),
  mergeOperation: (event) => set((state) => {
    const key = fusionGameKey(event);
    const current = state.operations[key];
    if (state.retiredOperationIds.includes(event.operationId)) return state;
    if (current?.operationId === event.operationId && (event.sequence <= current.sequence || isFusionOperationTerminal(current))) return state;
    return {
      operations: { ...state.operations, [key]: event },
      retiredOperationIds: current && current.operationId !== event.operationId
        ? [current.operationId, ...state.retiredOperationIds].slice(0, 512)
        : state.retiredOperationIds
    };
  })
}));

export function isFusionOperationTerminal(operation: FusionOperationProgress) {
  return ['completed', 'failed', 'cancelled'].includes(operation.status);
}
