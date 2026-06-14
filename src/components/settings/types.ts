import type { MvpPlatform } from '@/types/platform';
import type { UpdateCheckError, UpdateCheckReport } from '@/types/repository';

export type BusyAction = string | null;
export type UpdatePanelPhase = 'idle' | 'checking' | 'up-to-date' | 'available' | 'installing' | 'error';

export interface UpdatePanelState {
  phase: UpdatePanelPhase;
  report: UpdateCheckReport | null;
  error: UpdateCheckError | null;
}

export type BusyState = `browse:${MvpPlatform}` | 'save' | null;
export type SettingsSection = 'general' | 'emulators' | 'metadata' | 'sources' | 'storage' | 'diagnostics' | 'updates';
