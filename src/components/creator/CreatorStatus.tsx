'use client';

import { useI18n } from '@/components/I18nProvider';
import type { CreatorLoadStatus } from '@/stores/creatorStore';
import type { FusionError } from '@/types/fusionSource';

export function CreatorStatus({ status, error, testId }: { status: CreatorLoadStatus; error: FusionError | null; testId?: string }) {
  const { t } = useI18n();
  if (status === 'ready') return null;
  return <div data-testid={testId} data-state={status} role={error ? 'alert' : 'status'} className="select-text break-words text-sm text-[var(--rh-muted)]">
    {status === 'loading' ? t.common.loading : status === 'unavailable' ? t.creator.common.unavailable : status === 'error' ? t.creator.common.error : t.creator.common.idle}
    {error && <span className="mt-1 block break-all font-mono text-xs">{error.code}{typeof error.details.command === 'string' ? ` · ${error.details.command}` : ''}{error.code !== 'action_unavailable' ? ` · ${error.message}` : ''}</span>}
  </div>;
}
