'use client';

import { useEffect, useRef, useState } from 'react';
import { useI18n } from '@/components/I18nProvider';
import { useCreatorDialogFocus } from '@/hooks/useCreatorDialogFocus';
import { useCreatorStore } from '@/stores/creatorStore';
import { CreatorStatus } from './CreatorStatus';

function AssignmentDialog({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const ref = useRef<HTMLElement>(null);
  useCreatorDialogFocus(ref, onClose);
  return <div className="fixed inset-0 z-[60] grid place-items-center bg-black/80 px-5">
    <section ref={ref} role="dialog" aria-modal="true" aria-labelledby="creator-assignment-title" data-testid="creator-assignment-dialog" tabIndex={-1} className="max-h-[88vh] w-full max-w-md overflow-y-auto rounded-2xl border border-fusion-line bg-fusion-surface p-5">
      <header className="mb-4 flex items-start justify-between gap-4">
        <h2 id="creator-assignment-title" className="text-xl font-semibold">{t.input.assignment}</h2>
        <button type="button" data-testid="creator-assignment-close" data-focus-id="creator:assignment-close" className="rh-filter-chip rh-focusable shrink-0" onClick={onClose}>{t.common.close}</button>
      </header>
      <p role="status" data-state="unavailable" className="text-sm text-[var(--rh-muted)]">{t.input.assignmentNote}</p>
      <p className="mt-4 text-xs text-[var(--rh-muted)]">{t.input.ownership}</p>
    </section>
  </div>;
}

export function CreatorDeviceDialog({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const state = useCreatorStore();
  const ref = useRef<HTMLElement>(null);
  const [assigning, setAssigning] = useState(false);
  const busy = state.inputOwnership === 'starting' || state.inputOwnership === 'releasing';
  const needsRelease = !['none', 'released'].includes(state.inputOwnership);
  const close = async () => {
    const store = useCreatorStore.getState();
    if (store.inputOwnership === 'starting' || store.inputOwnership === 'releasing') return;
    if (!['none', 'released'].includes(store.inputOwnership)) {
      try { await store.stopInputSession(); } catch { return; }
      if (!useCreatorStore.getState().inputHandlesClosed) return;
    }
    onClose();
  };
  useCreatorDialogFocus(ref, () => { void close(); });
  useEffect(() => {
    const store = useCreatorStore.getState();
    void store.loadInputDevices().catch(() => {});
    if (store.inputSessionId && store.inputOwnership === 'owned') void store.refreshInputSession().catch(() => {});
  }, []);
  return <div className="fixed inset-0 z-50 grid place-items-center bg-black/80 px-5">
    <section ref={ref} role="dialog" aria-modal="true" aria-labelledby="creator-device-title" data-testid="creator-device-dialog" tabIndex={-1} className="relative flex max-h-[88vh] w-full max-w-3xl flex-col overflow-hidden rounded-2xl border border-fusion-line bg-fusion-surface">
      <header className="flex items-start justify-between gap-4 border-b border-fusion-line p-5">
        <h2 id="creator-device-title" className="text-xl font-semibold">{t.input.title}</h2>
        <button type="button" data-testid="creator-device-close" data-focus-id="creator:device-close" disabled={busy} className="rh-filter-chip rh-focusable shrink-0 disabled:opacity-40" onClick={() => void close()}>{t.common.close}</button>
      </header>
      <div className="min-h-0 overflow-y-auto p-5">
        <p className="mb-4 text-xs text-[var(--rh-muted)]">{t.input.ownership}</p>
        <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
          <h3 className="font-semibold">{t.input.devices}</h3>
          <button type="button" data-focus-id="creator:devices-refresh" disabled={state.inputDevicesStatus === 'loading'} className="rh-filter-chip rh-focusable" onClick={() => void state.loadInputDevices().catch(() => {})}>{t.creator.ui.refresh}</button>
        </div>
        <CreatorStatus status={state.inputDevicesStatus} error={state.inputDevicesError} testId="creator-devices-state" />
        {state.inputDevicesStatus === 'ready' && state.inputDevices.length === 0 && <p className="text-sm text-[var(--rh-muted)]">{t.input.empty}</p>}
        {state.inputDevicesStatus === 'ready' && state.inputDevices.map((device) => <div key={device.deviceId} className="flex items-center justify-between gap-3 border-b border-fusion-line py-3" data-testid="creator-device-row">
          <div className="min-w-0"><p className="break-words text-sm font-semibold">{device.name}</p><p className="mt-1 text-xs text-[var(--rh-muted)]">{device.connected ? t.input.connected : t.input.disconnected} · {device.mappingKnown ? t.input.known : t.input.unknown}</p></div>
          <button type="button" data-focus-id={`creator:device-${device.deviceId}`} disabled={!device.connected || needsRelease} className="rh-filter-chip rh-focusable shrink-0 disabled:opacity-40" onClick={() => void state.startInputSession(device.deviceId).catch(() => {})}>{t.input.check}</button>
        </div>)}
        <section className="mt-4 border-t border-fusion-line pt-4">
          <CreatorStatus status={state.inputStatus} error={state.inputError} />
          {state.inputOwnership === 'uncertain' && <p role="alert" className="mt-3 text-sm text-[var(--rh-warning)]">{t.input.releaseRequired}</p>}
          {!state.inputSessionId && !needsRelease && <p className="text-sm text-[var(--rh-muted)]">{t.input.noSession}</p>}
          {state.inputSessionId && <p className="mb-2 break-all font-mono text-xs">{t.input.session}: {state.inputSessionId}</p>}
          {state.inputState && <div data-testid="creator-input-state" className="space-y-3 text-sm">
            <p>{t.input.statuses[state.inputState.status]} · {t.creator.ui.sequence}: {state.inputState.sequence}</p>
            <div><h4 className="mb-1 font-semibold">{t.input.buttons}</h4><dl className="flex flex-wrap gap-3">{Object.entries(state.inputState.buttons).map(([name, pressed]) => <div key={name} className="flex gap-1"><dt>{name}:</dt><dd>{pressed ? t.common.yes : t.common.no}</dd></div>)}</dl></div>
            <div><h4 className="mb-1 font-semibold">{t.input.axes}</h4><dl className="flex flex-wrap gap-3">{Object.entries(state.inputState.axes).map(([name, value]) => <div key={name} className="flex gap-1"><dt>{name}:</dt><dd>{value.toFixed(3)}</dd></div>)}</dl></div>
            {state.inputState.error && <CreatorStatus status={state.inputState.error.code === 'action_unavailable' ? 'unavailable' : 'error'} error={state.inputState.error} />}
          </div>}
        </section>
      </div>
      <footer className="flex shrink-0 flex-wrap justify-end gap-2 border-t border-fusion-line p-5">
        {needsRelease && <button type="button" data-testid="creator-input-stop" data-focus-id="creator:input-stop" disabled={busy || !state.inputSessionId} className="rh-filter-chip rh-focusable disabled:opacity-40" onClick={() => void state.stopInputSession().catch(() => {})}>{t.input.stop}</button>}
        <button type="button" data-testid="creator-assignment-open" data-focus-id="creator:assignment-open" className="rh-filter-chip rh-focusable" onClick={() => setAssigning(true)}>{t.input.assignment}</button>
      </footer>
    </section>
    {assigning && <AssignmentDialog onClose={() => setAssigning(false)} />}
  </div>;
}
