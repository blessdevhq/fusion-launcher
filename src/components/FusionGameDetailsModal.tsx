'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { Download, Loader2, Play, RotateCcw, ShieldCheck, X } from 'lucide-react';
import { useI18n } from './I18nProvider';
import { api } from '@/lib/api';
import { fusionText } from '@/lib/fusionText';
import { isTauriRuntime } from '@/lib/runtime';
import { useFusionSourceStore, isFusionOperationTerminal } from '@/stores/fusionSourceStore';
import { fusionErrorMessage, fusionGameKey, type FusionCatalogEntry, type FusionCrashReport, type FusionGameOptions, type FusionGameState, type FusionInstallInput } from '@/types/fusionSource';

export function FusionGameDetailsModal({ game, onClose, onRefresh }: {
  game: FusionCatalogEntry;
  onClose: () => void;
  onRefresh: () => Promise<void>;
}) {
  const { locale } = useI18n();
  const text = fusionText(locale);
  const [state, setState] = useState<FusionGameState | null>(null);
  const [options, setOptions] = useState<FusionGameOptions>({ enabledAddons: null, runtimeChannel: null });
  const [input, setInput] = useState<FusionInstallInput>({ userFiles: {}, rawPaths: {}, externalWorkspaceRoot: null });
  const [busy, setBusy] = useState(false);
  const [cancelPending, setCancelPending] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [report, setReport] = useState<FusionCrashReport | null>(null);
  const [loading, setLoading] = useState(true);
  const modalRef = useRef<HTMLElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  const refreshRef = useRef(onRefresh);
  refreshRef.current = onRefresh;
  const operation = useFusionSourceStore((store) => store.operations[fusionGameKey(game)]);
  const running = Boolean(operation && !isFusionOperationTerminal(operation)) || Boolean(state?.operation && !isFusionOperationTerminal(state.operation) && !operation);
  const disabled = busy || running;
  const reload = useCallback(async () => {
    const next = await api.fusionGameGetState(game.sourceId, game.gameId);
    if (next.operation) useFusionSourceStore.getState().mergeOperation(next.operation);
    setState(next);
    setOptions(next.options);
  }, [game.sourceId, game.gameId]);

  useEffect(() => {
    let active = true;
    setLoading(true);
    void api.fusionGameGetState(game.sourceId, game.gameId).then((next) => {
      if (active) { if (next.operation) useFusionSourceStore.getState().mergeOperation(next.operation); setState(next); setOptions(next.options); setMessage(null); }
    }).catch((error) => { if (active) setMessage(fusionErrorMessage(error)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [game.sourceId, game.gameId]);

  useEffect(() => {
    if (operation && isFusionOperationTerminal(operation)) {
      void reload().then(() => refreshRef.current()).catch((error) => setMessage(fusionErrorMessage(error)));
    }
  }, [operation, reload]);

  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    closeRef.current?.focus();
    const keydown = (event: KeyboardEvent) => {
      // Keep shell shortcuts from activating the catalog behind this dialog.
      if (event.target instanceof Node && modalRef.current?.contains(event.target)) event.stopPropagation();
      if (event.key === 'Escape') { event.preventDefault(); onCloseRef.current(); }
      if (event.key !== 'Tab') return;
      const controls = Array.from(modalRef.current?.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled),[tabindex="0"]') ?? []).filter((node) => node.getClientRects().length);
      const first = controls[0]; const last = controls.at(-1);
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    };
    document.addEventListener('keydown', keydown);
    return () => { document.removeEventListener('keydown', keydown); previous?.focus(); };
  }, []);

  const run = async (action: () => Promise<unknown>, notice?: string) => {
    setBusy(true); setMessage(null);
    try {
      const result = await action();
      if (typeof result === 'object' && result !== null && 'operationId' in result && 'sequence' in result && 'status' in result) {
        useFusionSourceStore.getState().mergeOperation(result as import('@/types/fusionSource').FusionOperationProgress);
      }
      await reload(); await onRefresh(); if (notice) setMessage(notice);
    }
    catch (error) { setMessage(fusionErrorMessage(error)); }
    finally { setBusy(false); }
  };
  const saveOptions = () => api.fusionGameSetOptions(game.sourceId, game.gameId, options);
  const install = (update: boolean) => run(async () => {
    await saveOptions();
    return update ? api.fusionGameUpdate(game.sourceId, game.gameId, input) : api.fusionGameInstall(game.sourceId, game.gameId, input);
  });
  const chooseFile = async (layerId: string, directory = false) => {
    const selected = isTauriRuntime()
      ? await open({ multiple: false, directory, title: directory ? text.chooseFolder : text.choose })
      : `preview://UserFiles/${layerId}${directory ? '/' : '/game.bin'}`;
    if (typeof selected === 'string') setInput((current) => ({ ...current, userFiles: { ...current.userFiles, [layerId]: selected } }));
  };
  const cancel = async () => {
    setCancelPending(true);
    try { await api.fusionGameCancel(game.sourceId, game.gameId); }
    catch (error) { setMessage(fusionErrorMessage(error)); }
    finally { setCancelPending(false); }
  };
  const blocked = Boolean(state?.blockers.some((blocker) => !['untrusted_executable', 'missing_injector_dll', 'corrupted_critical_binary', 'local_base_modified'].includes(blocker.code)));
  const activeOperation = operation ?? state?.operation;

  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/80 px-5">
      <section ref={modalRef} role="dialog" aria-modal="true" aria-labelledby="fusion-game-title" data-testid="fusion-game-modal" className="relative flex max-h-[88vh] w-full max-w-3xl flex-col overflow-hidden rounded-2xl border border-fusion-line bg-fusion-surface text-white">
        <header className="flex items-start gap-4 border-b border-fusion-line p-5">
          <div className="min-w-0 flex-1">
            <h2 id="fusion-game-title" className="break-words text-2xl font-bold">{game.title}</h2>
            <p className="mt-1 text-sm text-white/50">{state?.platform ?? text.noPlatform} · {game.sourceName}</p>
          </div>
          <button ref={closeRef} type="button" onClick={onClose} className="rh-icon-button rh-focusable" aria-label={text.close} data-testid="fusion-game-close"><X className="h-4 w-4" /></button>
        </header>
        <div className="min-h-0 overflow-y-auto p-5">
          {loading && <p role="status" className="flex gap-2 text-sm text-white/60"><Loader2 className="h-4 w-4 animate-spin" />{text.loading}</p>}
          {message && <p role="alert" className="rh-banner !m-0 !mb-4 break-words whitespace-pre-line" data-testid="fusion-game-message">{message}</p>}
          {!isTauriRuntime() && <p className="mb-4 text-xs text-white/50">{text.previewNote}</p>}
          {state && <>
            <div className="mb-5 flex flex-wrap gap-x-6 gap-y-2 text-sm">
              <span data-testid="fusion-game-readiness" className={state.ready ? 'text-fusion-accent' : 'text-white/70'}>{state.ready ? text.ready : state.installed ? text.installed : text.available}</span>
              <span className="text-white/50">{text.version}: {state.activeVersion ?? '—'} → {state.version}</span>
            </div>
            {state.blockers.length > 0 && <div className="mb-5 border-l-2 border-[var(--rh-warning)] pl-3" data-testid="fusion-blockers">
              <h3 className="font-semibold">{text.blockers}</h3>
              {state.blockers.map((blocker, index) => <p key={`${blocker.code}:${index}`} className="mt-2 break-words text-sm text-white/70"><span className="font-mono text-xs">{blocker.code}</span> · {blocker.message}</p>)}
            </div>}
            {state.warnings.length > 0 && <div className="mb-5 text-sm text-white/60"><h3 className="font-semibold text-white/80">{text.warnings}</h3>{state.warnings.map((warning, i) => <p className="mt-2" key={i}>{warning}</p>)}</div>}
            <div className="grid gap-5 border-t border-fusion-line py-5 sm:grid-cols-2">
              <fieldset disabled={disabled} className="min-w-0"><legend className="mb-3 text-sm font-semibold">{text.addons}</legend>
                {state.addons.map((addon) => <label key={addon.id} className="mb-3 flex items-center gap-3 text-sm text-white/70"><input type="checkbox" className="accent-fusion-accent" checked={options.enabledAddons?.includes(addon.id) ?? addon.enabled} onChange={(event) => {
                  const selected = new Set(options.enabledAddons ?? state.addons.filter((item) => item.enabled).map((item) => item.id));
                  if (event.target.checked) selected.add(addon.id); else selected.delete(addon.id);
                  setOptions({ ...options, enabledAddons: [...selected] });
                }} data-testid={`fusion-addon-${addon.id}`} />{addon.id} <span className="text-xs text-white/40">{addon.kind}</span></label>)}
              </fieldset>
              <div className="min-w-0"><label className="mb-3 block text-sm font-semibold" htmlFor="fusion-runtime-channel">{text.runtime}</label>
                {state.runtimeChannels.length > 0 && <select id="fusion-runtime-channel" data-testid="fusion-runtime-channel" disabled={disabled} value={options.runtimeChannel ?? state.runtimeChannels[0]} onChange={(event) => setOptions({ ...options, runtimeChannel: event.target.value as 'pinned' | 'dynamic' })} className="rh-library-sort w-full">
                  {state.runtimeChannels.map((channel) => <option key={channel} value={channel}>{channel === 'dynamic' ? text.dynamic : text.pinned}</option>)}
                </select>}
                <button type="button" className="rh-mini-action rh-focusable mt-3" disabled={disabled} onClick={() => void run(saveOptions, text.optionsSaved)}>{text.apply}</button>
              </div>
            </div>
            {state.userLayers.length > 0 && <fieldset className="border-t border-fusion-line py-5" disabled={disabled}><legend className="text-sm font-semibold">{text.files}</legend>
              {state.userLayers.map((layer) => <div key={layer.layerId} className="mt-3" data-testid={`fusion-user-layer-${layer.layerId}`}>
                <p className="text-sm font-semibold">{layer.layerId}</p>{layer.instructions && <p className="mt-1 text-sm text-white/50">{layer.instructions}</p>}
                <div className="mt-2 flex flex-wrap items-center gap-2"><button type="button" onClick={() => void chooseFile(layer.layerId)} className="rh-mini-action rh-focusable">{text.choose}</button><button type="button" onClick={() => void chooseFile(layer.layerId, true)} className="rh-mini-action rh-focusable">{text.chooseFolder}</button><span className="min-w-0 break-all text-xs text-white/50">{input.userFiles[layer.layerId]}</span></div>
                {layer.rawPathRequired && <label className="mt-2 block text-xs text-white/60">{text.rawPath}<input value={input.rawPaths[layer.layerId] ?? ''} onChange={(event) => setInput((current) => ({ ...current, rawPaths: { ...current.rawPaths, [layer.layerId]: event.target.value } }))} className="mt-1 w-full rounded-sm border border-fusion-line bg-fusion-bg p-2 text-sm text-white" placeholder="game/game.bin" /></label>}
              </div>)}
            </fieldset>}
            <details className="mb-5 text-xs text-white/50"><summary>{text.externalRoot}</summary><input aria-label={text.externalRoot} value={input.externalWorkspaceRoot ?? ''} onChange={(event) => setInput({ ...input, externalWorkspaceRoot: event.target.value || null })} className="mt-2 w-full rounded-sm border border-fusion-line bg-fusion-bg p-2 text-sm text-white" disabled={disabled} /></details>
            {state.executables.map((exe) => <div key={`${exe.path}:${exe.blake3}`} className="mb-4 border-t border-fusion-line pt-3 text-xs">
              <p className="break-words text-white/70">{exe.path}</p><p className="mt-1 select-text break-all font-mono text-white/40">BLAKE3 {exe.blake3}</p>
              {exe.trusted ? <p className="mt-2 text-fusion-accent">{text.trusted}</p> : <button type="button" onClick={() => void run(() => api.fusionGameTrustExecutable(game.sourceId, game.gameId, exe.path, exe.blake3))} className="rh-mini-action rh-focusable mt-2" disabled={disabled} data-testid="fusion-trust-executable"><ShieldCheck className="h-4 w-4" />{text.trust}</button>}
            </div>)}
            {activeOperation && <div className="mb-5 border-t border-fusion-line pt-3" role="status" data-testid="fusion-operation-progress"><p className="text-sm text-white/70">{text.operation}: {activeOperation.phase}{activeOperation.layerId ? ` · ${activeOperation.layerId}` : ''} · {activeOperation.status}</p>{activeOperation.error && <p className="mt-2 text-sm text-white/60">{activeOperation.error.message}</p>}</div>}
            {state.crashReportIds?.length > 0 && <div className="border-t border-fusion-line py-4"><h3 className="mb-2 text-sm font-semibold">{text.crashes}</h3>{state.crashReportIds.map((reportId) => <button key={reportId} type="button" className="rh-mini-action rh-focusable" onClick={() => void run(async () => setReport(await api.fusionCrashPreview(reportId)))} data-testid="fusion-crash-preview">{text.previewCrash}</button>)}</div>}
            {report && <div className="border-t border-fusion-line py-4" data-testid="fusion-crash-report"><pre className="max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-sm bg-fusion-bg p-3 text-xs text-white/70">{JSON.stringify(report, null, 2)}</pre><p className="mt-2 break-all text-xs text-white/50">{report.canSend ? `${text.endpoint}: ${report.endpoint}` : text.noEndpoint}</p><div className="mt-3 flex flex-wrap gap-2"><button type="button" disabled={disabled || !report.canSend || !report.endpoint} className="rh-mini-action rh-focusable" onClick={() => void run(() => api.fusionCrashSend(report.reportId, report.endpoint!), text.reported)} data-testid="fusion-crash-send">{text.send}</button><button type="button" disabled={disabled} className="rh-mini-action rh-focusable" onClick={() => void run(async () => { await api.fusionCrashDiscard(report.reportId); setReport(null); }, text.discarded)} data-testid="fusion-crash-discard">{text.discard}</button></div></div>}
          </>}
        </div>
        <footer className="shrink-0 border-t border-fusion-line p-5"><div className="flex flex-wrap gap-2">
          <button type="button" disabled={disabled || !state?.ready} onClick={() => void run(() => api.fusionGameLaunch(game.sourceId, game.gameId))} className="rh-primary-action rh-focusable" data-testid="fusion-game-launch"><Play className="h-4 w-4" />{text.launch}</button>
          <button type="button" disabled={disabled || loading || !state || blocked} onClick={() => void install(Boolean(state?.installed))} className="rh-mini-action rh-focusable" data-testid="fusion-game-install"><Download className="h-4 w-4" />{state?.installed ? text.update : text.install}</button>
          <button type="button" disabled={disabled || !state?.rollbackAvailable} onClick={() => void run(() => api.fusionGameRollback(game.sourceId, game.gameId))} className="rh-mini-action rh-focusable" data-testid="fusion-game-rollback"><RotateCcw className="h-4 w-4" />{text.rollback}</button>
          {running && <button type="button" disabled={cancelPending} onClick={() => void cancel()} className="rh-mini-action rh-focusable" data-testid="fusion-game-cancel">{text.cancel}</button>}
          <button type="button" disabled={disabled || !state?.installed} className="rh-mini-action rh-focusable" onClick={() => { if (window.confirm(text.removeConfirm)) void run(() => api.fusionGameRemove(game.sourceId, game.gameId)); }} data-testid="fusion-game-remove">{text.remove}</button>
        </div><p className="mt-3 text-xs text-white/40">{text.rollbackNote}</p></footer>
      </section>
    </div>
  );
}
