'use client';

import { useEffect, useId, useState } from 'react';
import { api } from '@/lib/api';
import { formatFusionBytes } from '@/lib/fusionGamePresentation';
import { fusionText } from '@/lib/fusionText';
import type { FusionSaveSnapshotInventory } from '@/types/fusionSource';
import { useI18n } from './I18nProvider';

type LoadState = { status: 'loading' | 'error' } | { status: 'loaded'; inventory: FusionSaveSnapshotInventory };
export type FusionBackupStatus = 'idle' | 'pending' | 'completed' | 'cancelled' | 'busy' | 'noFiles' | 'failed';
type SaveHistoryProps = {
  sourceId: string;
  gameId: string;
  revision: string | null;
  backupDisabled: boolean;
  backupStatus: FusionBackupStatus;
  onBackup: () => void;
};

export function FusionSaveHistory(props: SaveHistoryProps) {
  const { locale } = useI18n();
  const [expanded, setExpanded] = useState(false);
  return (
    <details onToggle={(event) => setExpanded(event.currentTarget.open)} data-testid="fusion-save-history" className="border-t border-fusion-line py-4">
      <summary className="rh-focusable cursor-pointer rounded-sm text-sm font-semibold">{fusionText(locale).saveHistory}</summary>
      {expanded && <SaveHistoryContents {...props} />}
    </details>
  );
}

function SaveHistoryContents({ sourceId, gameId, revision, backupDisabled, backupStatus, onBackup }: SaveHistoryProps) {
  const { locale } = useI18n();
  const text = fusionText(locale);
  const noteId = useId();
  const [state, setState] = useState<LoadState>({ status: 'loading' });
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let active = true;
    void api.fusionGameListSaveSnapshots(sourceId, gameId).then(
      (inventory) => { if (active) setState({ status: 'loaded', inventory }); },
      () => { if (active) setState({ status: 'error' }); },
    );
    return () => { active = false; };
  }, [sourceId, gameId, revision, attempt]);

  const inventory = state.status === 'loaded' ? state.inventory : null;
  const backupNotice = backupStatus === 'pending' ? text.saveBackupPending : backupStatus === 'completed' ? text.saveBackupCompleted : backupStatus === 'cancelled' ? text.saveBackupCancelled : null;
  const backupError = backupStatus === 'busy' ? text.saveBackupBusy : backupStatus === 'noFiles' ? text.saveBackupNoFiles : backupStatus === 'failed' ? text.saveBackupFailed : null;
  const formatDate = new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'medium' });
  return (
    <div className="mt-3 text-sm text-white/70">
      <p id={noteId} className="text-xs leading-relaxed text-white/60">{text.saveHistoryNote}</p>
      <button type="button" aria-describedby={noteId} aria-disabled={backupDisabled} onClick={backupDisabled ? undefined : onBackup} className="rh-mini-action rh-focusable mt-2 aria-disabled:cursor-not-allowed aria-disabled:opacity-50">{text.saveBackupAction}</button>
      <div role="status" className="mt-2">
        {backupNotice && <p>{backupNotice}</p>}
        {state.status === 'loading' && <p>{text.saveHistoryLoading}</p>}
        {inventory && inventory.snapshots.length === 0 && <p>{inventory.skippedCount || inventory.hasMore ? text.saveHistoryNoReadable : text.saveHistoryEmpty}</p>}
      </div>
      {backupError && <p role="alert" className="mt-2 text-[var(--rh-warning)]">{backupError}</p>}
      {state.status === 'error' && <>
        <p role="alert" className="text-[var(--rh-warning)]">{text.saveHistoryError}</p>
        <button type="button" className="rh-mini-action rh-focusable mt-2" onClick={(event) => {
          event.currentTarget.closest('details')?.querySelector('summary')?.focus();
          setState({ status: 'loading' });
          setAttempt((current) => current + 1);
        }}>{text.saveHistoryRetry}</button>
      </>}
      {inventory && <>
        {inventory.snapshots.length > 0 && <ul aria-label={text.saveHistory} className="divide-y divide-fusion-line">
          {inventory.snapshots.map((snapshot) => <li key={snapshot.id} data-testid="fusion-save-row" className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 py-2">
            <time className="text-white/80" dateTime={snapshot.createdAt}>{formatDate.format(new Date(snapshot.createdAt))}</time>
            <span>{text.saveHistoryFileCount(snapshot.fileCount)} · {formatFusionBytes(snapshot.totalBytes, locale)}</span>
          </li>)}
        </ul>}
        {inventory.skippedCount > 0 && <p className="mt-2 text-xs text-[var(--rh-warning)]">{text.saveHistorySkipped(inventory.skippedCount)}</p>}
        {inventory.hasMore && <p className="mt-2 text-xs text-white/60">{text.saveHistoryLimited}</p>}
      </>}
    </div>
  );
}
