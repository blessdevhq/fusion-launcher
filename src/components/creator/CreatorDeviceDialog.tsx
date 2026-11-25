'use client';

import type { LogicalBinding } from '@/types/fusionCreator';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useI18n } from '@/components/I18nProvider';
import { useCreatorDialogFocus } from '@/hooks/useCreatorDialogFocus';
import { useCreatorStore } from '@/stores/creatorStore';
import { CreatorStatus } from './CreatorStatus';

const SDL_DEFAULT_BINDINGS: Record<string, LogicalBinding> = {
  a: { kind: 'controller_button', button: 'b' },
  b: { kind: 'controller_button', button: 'a' },
  x: { kind: 'controller_button', button: 'y' },
  y: { kind: 'controller_button', button: 'x' },
  l: { kind: 'controller_button', button: 'leftshoulder' },
  r: { kind: 'controller_button', button: 'rightshoulder' },
  zl: { kind: 'controller_axis', axis: 'lefttrigger', direction: 'positive' },
  zr: { kind: 'controller_axis', axis: 'righttrigger', direction: 'positive' },
  plus: { kind: 'controller_button', button: 'start' },
  minus: { kind: 'controller_button', button: 'back' },
  dup: { kind: 'controller_button', button: 'dpup' },
  ddown: { kind: 'controller_button', button: 'dpdown' },
  dleft: { kind: 'controller_button', button: 'dpleft' },
  dright: { kind: 'controller_button', button: 'dpright' },
  lstick: { kind: 'controller_button', button: 'leftstick' },
  rstick: { kind: 'controller_button', button: 'rightstick' },
  lstick_up: { kind: 'controller_axis', axis: 'lefty', direction: 'negative' },
  lstick_down: { kind: 'controller_axis', axis: 'lefty', direction: 'positive' },
  lstick_left: { kind: 'controller_axis', axis: 'leftx', direction: 'negative' },
  lstick_right: { kind: 'controller_axis', axis: 'leftx', direction: 'positive' },
  rstick_up: { kind: 'controller_axis', axis: 'righty', direction: 'negative' },
  rstick_down: { kind: 'controller_axis', axis: 'righty', direction: 'positive' },
  rstick_left: { kind: 'controller_axis', axis: 'rightx', direction: 'negative' },
  rstick_right: { kind: 'controller_axis', axis: 'rightx', direction: 'positive' },
};

const SDL_DEADZONE_AXES = ['leftx', 'lefty', 'rightx', 'righty', 'lefttrigger', 'righttrigger'] as const;

function formatLogicalBinding(binding: LogicalBinding): string {
  switch (binding.kind) {
    case 'controller_button': return `button:${binding.button}`;
    case 'controller_axis': return `axis:${binding.axis} (${binding.direction})`;
    case 'keyboard': return `key:${binding.code}`;
    case 'mouse_button': return `mouse:${binding.button}`;
    case 'mouse_axis': return `mouse:${binding.axis} (${binding.direction})`;
  }
}

function initialBindingsFor(seed?: Record<string, unknown>): Record<string, LogicalBinding> {
  const next: Record<string, LogicalBinding> = { ...SDL_DEFAULT_BINDINGS };
  if (!seed) return next;
  for (const [action, raw] of Object.entries(seed)) {
    if (typeof raw !== 'object' || raw === null || !('kind' in raw)) continue;
    const candidate = raw as Partial<LogicalBinding>;
    if (candidate.kind === 'controller_button' && typeof candidate.button === 'string') {
      next[action] = { kind: 'controller_button', button: candidate.button };
    } else if (
      candidate.kind === 'controller_axis'
      && typeof candidate.axis === 'string'
      && (candidate.direction === 'positive' || candidate.direction === 'negative')
    ) {
      next[action] = { kind: 'controller_axis', axis: candidate.axis, direction: candidate.direction };
    }
  }
  return next;
}

function AssignmentDialog({ onClose, controlsProfileId }: { onClose: () => void; controlsProfileId?: string }) {
  const { t } = useI18n();
  const state = useCreatorStore();
  const ref = useRef<HTMLElement>(null);
  const [capturingAction, setCapturingAction] = useState<string | null>(null);
  useCreatorDialogFocus(ref, () => {
    if (capturingAction) setCapturingAction(null);
    else onClose();
  }, { capture: capturingAction !== null });

  const sdlProfiles = useMemo(() => {
    const declared = state.currentProject?.project.program_descriptor.controls_profiles ?? [];
    const filtered = declared
      .filter((profile): profile is Extract<typeof profile, { kind: 'eden_param_package_v1' }> =>
        profile.kind === 'eden_param_package_v1' && profile.device === 'sdl_gamecontroller')
      .map((profile) => ({ id: profile.id, title: profile.title, bindings: profile.bindings }));
    if (controlsProfileId && !filtered.some((profile) => profile.id === controlsProfileId)) {
      return [{ id: controlsProfileId, title: controlsProfileId, bindings: {} }, ...filtered];
    }
    return filtered;
  }, [state.currentProject, controlsProfileId]);

  const [selectedProfileId, setSelectedProfileId] = useState(controlsProfileId ?? sdlProfiles[0]?.id ?? '');
  const activeProfile = useMemo(
    () => sdlProfiles.find((profile) => profile.id === (controlsProfileId ?? selectedProfileId)) ?? sdlProfiles[0] ?? null,
    [sdlProfiles, controlsProfileId, selectedProfileId]
  );
  const [bindings, setBindings] = useState<Record<string, LogicalBinding>>(() => initialBindingsFor(activeProfile?.bindings));
  const [deadzones, setDeadzones] = useState<Record<string, number>>({
    leftx: 0.15, lefty: 0.15, rightx: 0.15, righty: 0.15, lefttrigger: 0.05, righttrigger: 0.05,
  });

  useEffect(() => {
    if (activeProfile) setBindings(initialBindingsFor(activeProfile.bindings));
  }, [activeProfile]);

  useEffect(() => {
    if (!capturingAction || !state.inputState || state.inputState.status !== 'active') return;
    const pressedButton = Object.entries(state.inputState.buttons).find(([, pressed]) => pressed)?.[0];
    if (pressedButton) {
      setBindings((current) => ({ ...current, [capturingAction]: { kind: 'controller_button', button: pressedButton } }));
      setCapturingAction(null);
      return;
    }
    const deflectedAxis = Object.entries(state.inputState.axes).find(([, value]) => Math.abs(value) >= 0.6);
    if (deflectedAxis) {
      const [axis, value] = deflectedAxis;
      setBindings((current) => ({
        ...current,
        [capturingAction]: { kind: 'controller_axis', axis, direction: value > 0 ? 'positive' : 'negative' },
      }));
      setCapturingAction(null);
    }
  }, [capturingAction, state.inputState]);

  const liveOwned = Boolean(state.inputSessionId && state.inputOwnership === 'owned');

  return <div className="fixed inset-0 z-[60] grid place-items-center bg-black/80 px-5">
    <section ref={ref} role="dialog" aria-modal="true" aria-labelledby="creator-assignment-title" data-testid="creator-assignment-dialog" tabIndex={-1} className={`max-h-[88vh] w-full ${liveOwned ? 'max-w-lg' : 'max-w-md'} overflow-y-auto rounded-2xl border border-fusion-line bg-fusion-surface p-5`}>
      <header className="mb-4 flex items-start justify-between gap-4">
        <h2 id="creator-assignment-title" className="text-xl font-semibold">{t.input.assignment}</h2>
        <button type="button" data-testid="creator-assignment-close" data-focus-id="creator:assignment-close" className="rh-filter-chip rh-focusable shrink-0" onClick={onClose}>{t.common.close}</button>
      </header>
      {!liveOwned ? (
        <p role="status" data-state="unavailable" className="text-sm text-[var(--rh-muted)]">{t.input.assignmentNote}</p>
      ) : (
        <div className="space-y-4">
          {sdlProfiles.length === 0 ? (
            <p role="status" data-state="unavailable" className="text-sm text-[var(--rh-muted)]">{t.input.noControlsProfile}</p>
          ) : (
            <>
              <label className="block text-sm">
                <span className="mb-1 block font-medium">{t.input.profileLabel}</span>
                <select
                  aria-label={t.input.selectProfile}
                  data-testid="creator-assignment-profile"
                  disabled={Boolean(controlsProfileId)}
                  value={activeProfile?.id ?? ''}
                  onChange={(event) => setSelectedProfileId(event.target.value)}
                  className="rh-focusable h-9 w-full rounded-md border border-fusion-line bg-fusion-raised px-2 text-sm text-[var(--rh-text)]"
                >
                  {sdlProfiles.map((profile) => <option key={profile.id} value={profile.id}>{profile.title} ({profile.id})</option>)}
                </select>
              </label>
              <div className="divide-y divide-fusion-line border-t border-fusion-line">
                {Object.keys(SDL_DEFAULT_BINDINGS).map((action) => (
                  <div key={action} className="flex flex-wrap items-center justify-between gap-2 py-2 text-xs">
                    <span className="font-mono font-semibold">{action}</span>
                    <span className="text-[var(--rh-muted)]">{formatLogicalBinding(bindings[action])}</span>
                    <button
                      type="button"
                      data-testid={`creator-capture-${action}`}
                      data-focus-id={`creator:capture-${action}`}
                      className="rh-filter-chip rh-focusable"
                      onClick={() => setCapturingAction((prev) => prev === action ? null : action)}
                    >
                      {capturingAction === action ? t.input.capturing : t.input.capture}
                    </button>
                  </div>
                ))}
              </div>
              {capturingAction && (
                <button type="button" className="rh-filter-chip rh-focusable text-xs" onClick={() => setCapturingAction(null)}>
                  {t.input.cancelCapture}
                </button>
              )}
              <div className="border-t border-fusion-line pt-3">
                <h3 className="mb-2 text-sm font-semibold">{t.input.deadzones}</h3>
                <div className="grid gap-2 sm:grid-cols-2">
                  {SDL_DEADZONE_AXES.map((axis) => (
                    <label key={axis} className="flex items-center justify-between gap-2 text-xs">
                      <span className="font-mono">{axis} ({(deadzones[axis] ?? 0).toFixed(2)})</span>
                      <input
                        type="range"
                        min={0}
                        max={0.5}
                        step={0.05}
                        value={deadzones[axis] ?? 0}
                        data-testid={`creator-deadzone-${axis}`}
                        onChange={(event) => setDeadzones((prev) => ({ ...prev, [axis]: Number(event.target.value) }))}
                        className="accent-fusion-accent"
                      />
                    </label>
                  ))}
                </div>
              </div>
              <CreatorStatus status={state.inputSaveStatus} error={state.inputSaveError} testId="creator-assignment-status" />
              {state.lastSavedBindings?.controlsProfileId === activeProfile?.id && (
                <p role="status" data-testid="creator-assignment-saved" className="text-xs text-fusion-accent">{t.input.savedBindings}</p>
              )}
              <button
                type="button"
                data-testid="creator-assignment-save"
                data-focus-id="creator:assignment-save"
                disabled={!activeProfile || state.inputSaveStatus === 'loading'}
                className="rh-filter-chip rh-focusable disabled:opacity-40"
                onClick={() => {
                  if (!activeProfile || !state.inputSessionId) return;
                  void state.saveInputBindings({
                    sessionId: state.inputSessionId,
                    controlsProfileId: activeProfile.id,
                    bindings,
                    deadzones,
                    rawBindings: {},
                  }).catch(() => {});
                }}
              >
                {t.input.saveBindings}
              </button>
            </>
          )}
        </div>
      )}
      <p className="mt-4 text-xs text-[var(--rh-muted)]">{t.input.ownership}</p>
    </section>
  </div>;
}

export function CreatorDeviceDialog({
  onClose,
  controlsProfileId,
  keepSessionOnSave = false,
}: {
  onClose: () => void;
  controlsProfileId?: string;
  keepSessionOnSave?: boolean;
}) {
  const { t } = useI18n();
  const state = useCreatorStore();
  const ref = useRef<HTMLElement>(null);
  const [assigning, setAssigning] = useState(false);
  const busy = state.inputOwnership === 'starting' || state.inputOwnership === 'releasing';
  const needsRelease = !['none', 'released'].includes(state.inputOwnership);
  const close = async () => {
    const store = useCreatorStore.getState();
    if (store.inputOwnership === 'starting' || store.inputOwnership === 'releasing') return;
    if (
      keepSessionOnSave
      && controlsProfileId
      && store.lastSavedBindings?.controlsProfileId === controlsProfileId
      && store.lastSavedBindings.sessionId === store.inputSessionId
      && store.inputOwnership === 'owned'
    ) {
      onClose();
      return;
    }
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
    {assigning && <AssignmentDialog onClose={() => setAssigning(false)} controlsProfileId={controlsProfileId} />}
  </div>;
}
