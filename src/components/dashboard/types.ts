import type { UpdateCheckError, UpdateCheckReport } from '@/types/repository';

export type BusyAction = string | null;
export type UpdatePanelPhase = 'idle' | 'checking' | 'up-to-date' | 'available' | 'installing' | 'error';

export interface UpdatePanelState {
  phase: UpdatePanelPhase;
  report: UpdateCheckReport | null;
  error: UpdateCheckError | null;
}
