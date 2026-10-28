'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { Check, Download, Loader2, Play, RotateCcw, ShieldCheck, X } from 'lucide-react';
import { useI18n } from './I18nProvider';
import { api } from '@/lib/api';
import { effectiveFusionOptions, formatFusionBytes, fusionHasVersionUpdate, fusionIsIntegrityError, fusionLayerTitle, fusionOperationVisible, fusionProgressRatio, fusionSettingsDiffer, fusionStructuredError, selectableFusionAddons } from '@/lib/fusionGamePresentation';
import { fusionText } from '@/lib/fusionText';
import { isTauriRuntime } from '@/lib/runtime';
import { useFusionSourceStore, isFusionOperationTerminal } from '@/stores/fusionSourceStore';
import { fusionErrorMessage, fusionGameKey, type FusionCatalogEntry, type FusionCrashReport, type FusionError, type FusionGameOptions, type FusionGameState, type FusionInstallInput, type FusionOperationProgress } from '@/types/fusionSource';

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
  const [guardError, setGuardError] = useState<{ snapshotId: string | null; error: FusionError } | null>(null);
  const [securityExpanded, setSecurityExpanded] = useState(false);
  const [report, setReport] = useState<FusionCrashReport | null>(null);
  const [loading, setLoading] = useState(true);
  const modalRef = useRef<HTMLElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const stateRef = useRef(state);
  stateRef.current = state;
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  const refreshRef = useRef(onRefresh);
  refreshRef.current = onRefresh;
  const operation = useFusionSourceStore((store) => store.operations[fusionGameKey(game)]);
  const activeOperation = operation ?? state?.operation;
  const running = Boolean(activeOperation && !isFusionOperationTerminal(activeOperation));
  const disabled = busy || running || loading;

  const adoptState = useCallback((next: FusionGameState, adoptDraft: boolean) => {
    if (next.operation) useFusionSourceStore.getState().mergeOperation(next.operation);
    useFusionSourceStore.getState().recordGameState(next);
    setState(next);
    setGuardError((previous) => {
      if (!previous || previous.snapshotId !== next.activeSnapshotId) return null;
      const stillBlocked = previous.error.code === 'untrusted_executable'
        ? next.executables.some((exe) => !exe.trusted) || next.blockers.some((blocker) => blocker.code === 'untrusted_executable')
        : next.blockers.some(fusionIsIntegrityError);
      return stillBlocked ? previous : null;
    });
    if (adoptDraft) {
      setOptions(effectiveFusionOptions(next, next.appliedOptions ?? next.options));
      setInput((current) => ({ ...current, externalWorkspaceRoot: next.externalWorkspaceRoot }));
    }
  }, []);
  const reload = useCallback(async (adoptDraft = false) => {
    const next = await api.fusionGameGetState(game.sourceId, game.gameId);
    adoptState(next, adoptDraft);
    return next;
  }, [game.sourceId, game.gameId, adoptState]);

  useEffect(() => {
    let active = true;
    setLoading(true); setState(null); setReport(null); setGuardError(null); setSecurityExpanded(false);
    setInput({ userFiles: {}, rawPaths: {}, externalWorkspaceRoot: null });
    void api.fusionGameGetState(game.sourceId, game.gameId).then((next) => {
      if (!active) return;
      adoptState(next, false);
      setOptions(effectiveFusionOptions(next));
      setInput({ userFiles: {}, rawPaths: {}, externalWorkspaceRoot: next.externalWorkspaceRoot });
      setMessage(null);
    }).catch((error) => { if (active) setMessage(fusionErrorMessage(error)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [game.sourceId, game.gameId, adoptState]);

  useEffect(() => {
    if (operation && isFusionOperationTerminal(operation)) {
      void reload().then(() => refreshRef.current()).catch((error) => setMessage(fusionErrorMessage(error)));
    }
  }, [operation, reload]);

  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    closeRef.current?.focus();
    const keydown = (event: KeyboardEvent) => {
      if (event.target instanceof Node && modalRef.current?.contains(event.target)) event.stopPropagation();
      if (event.key === 'Escape') { event.preventDefault(); onCloseRef.current(); }
      if (event.key !== 'Tab') return;
      const controls = Array.from(modalRef.current?.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled),summary,[tabindex="0"]') ?? []).filter((node) => node.getClientRects().length);
      const first = controls[0]; const last = controls.at(-1);
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    };
    document.addEventListener('keydown', keydown);
    return () => { document.removeEventListener('keydown', keydown); previous?.focus(); };
  }, []);

  const rememberFailure = (error: unknown) => {
    const structured = fusionStructuredError(error);
    if (structured && (fusionIsIntegrityError(structured) || structured.code === 'untrusted_executable')) {
      setGuardError({ snapshotId: stateRef.current?.activeSnapshotId ?? null, error: structured });
      useFusionSourceStore.getState().recordIntegrityError(game, stateRef.current?.activeSnapshotId ?? null, structured);
    }
    setMessage(fusionErrorMessage(error));
  };
  const run = async (action: () => Promise<unknown>, { notice, adoptDraft = false }: { notice?: string; adoptDraft?: boolean } = {}) => {
    setBusy(true); setMessage(null);
    try {
      const result = await action();
      const progress = typeof result === 'object' && result !== null && 'operationId' in result && 'sequence' in result && 'status' in result ? result as FusionOperationProgress : null;
      if (progress) useFusionSourceStore.getState().mergeOperation(progress);
      const succeeded = !progress || progress.status === 'completed';
      if (progress?.error) rememberFailure(progress.error);
      await reload(adoptDraft && succeeded);
      await onRefresh();
      if (notice && succeeded) setMessage(notice);
    } catch (error) {
      rememberFailure(error);
      await reload().catch(() => {});
    } finally { setBusy(false); }
  };
  const install = () => run(async () => {
    await api.fusionGameSetOptions(game.sourceId, game.gameId, options);
    const selected = new Set(options.enabledAddons ?? []);
    const acceptsInput = (id: string) => !state?.addons.some((addon) => addon.id === id) || selected.has(id);
    const request = { ...input, externalWorkspaceRoot: state?.installed ? state.externalWorkspaceRoot : input.externalWorkspaceRoot?.trim() || null,
      userFiles: Object.fromEntries(Object.entries(input.userFiles).filter(([id]) => acceptsInput(id))),
      rawPaths: Object.fromEntries(Object.entries(input.rawPaths).filter(([id]) => acceptsInput(id))) };
    return state?.installed ? api.fusionGameUpdate(game.sourceId, game.gameId, request) : api.fusionGameInstall(game.sourceId, game.gameId, request);
  }, { adoptDraft: true, notice: state?.installed ? text.optionsSaved : undefined });
  const chooseFile = async (layerId: string, directory = false) => {
    const selected = isTauriRuntime()
      ? await open({ multiple: false, directory, title: directory ? text.chooseFolder : text.choose })
      : `preview://UserFiles/${layerId}${directory ? '/' : '/game.bin'}`;
    if (typeof selected === 'string') setInput((current) => ({ ...current, userFiles: { ...current.userFiles, [layerId]: selected } }));
  };
  const cancel = async () => {
    setCancelPending(true);
    try { await api.fusionGameCancel(game.sourceId, game.gameId); }
    catch (error) { rememberFailure(error); }
    finally { setCancelPending(false); }
  };
  const discardReport = (reportId: string) => run(async () => {
    await api.fusionCrashDiscard(reportId);
    setReport((current) => current?.reportId === reportId ? null : current);
  }, { notice: text.discarded });
  const integrityErrors = [...state?.blockers ?? [], ...(guardError ? [guardError.error] : [])].filter(fusionIsIntegrityError);
  const repairRequired = integrityErrors.length > 0;
  const trustRequired = Boolean(state?.executables.some((exe) => !exe.trusted) || guardError?.error.code === 'untrusted_executable');
  const securityAttention = repairRequired || trustRequired;
  const blocked = Boolean(state?.blockers.some((blocker) => !['untrusted_executable', 'missing_injector_dll', 'corrupted_critical_binary', 'local_base_modified'].includes(blocker.code)));
  const otherBlockers = state?.blockers.filter((blocker) => !fusionIsIntegrityError(blocker) && blocker.code !== 'untrusted_executable') ?? [];
  const hasVersionUpdate = Boolean(state && fusionHasVersionUpdate(state));
  const settingsDirty = Boolean(state && fusionSettingsDiffer(state, options, input.externalWorkspaceRoot));
  const addons = state ? selectableFusionAddons(state.addons) : [];
  const progressRatio = fusionProgressRatio(activeOperation?.transfer);
  const crashReports = state?.crashReports ?? state?.crashReportIds.map((reportId) => ({ reportId, createdAt: '', exitCode: null, crashSignatureHash: null })) ?? [];
  const blockerMessage = (error: FusionError) => error.code === 'local_base_modified' ? text.localBaseModified : fusionIsIntegrityError(error) ? text.integrityRepair : error.code === 'untrusted_executable' ? text.integrityTrust : error.message;

  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/80 px-5">
      <section ref={modalRef} role="dialog" aria-modal="true" aria-labelledby="fusion-game-title" data-testid="fusion-game-modal" className="relative flex max-h-[88vh] w-full max-w-3xl flex-col overflow-hidden rounded-2xl border border-fusion-line bg-fusion-surface text-white">
        <header className="flex items-start gap-4 border-b border-fusion-line p-5">
          <div className="min-w-0 flex-1">
            <h2 id="fusion-game-title" className="break-words text-2xl font-bold">{game.title}</h2>
            <p className="mt-1 text-sm text-white/50">{state?.platform?.toUpperCase() ?? text.noPlatform} · {game.sourceName}</p>
          </div>
          <button ref={closeRef} type="button" onClick={onClose} className="rh-icon-button rh-focusable" aria-label={text.close} data-testid="fusion-game-close"><X className="h-4 w-4" /></button>
        </header>
        <div className="min-h-0 overflow-y-auto p-5">
          {loading && <p role="status" className="flex gap-2 text-sm text-white/60"><Loader2 className="h-4 w-4 animate-spin" />{text.loading}</p>}
          {message && <p role="alert" className="rh-banner select-text !m-0 !mb-4 break-words whitespace-pre-line" data-testid="fusion-game-message">{message}</p>}
          {!isTauriRuntime() && <p className="mb-4 text-xs text-white/50">{text.previewNote}</p>}
          {state && <>
            <div className="mb-5 flex flex-wrap items-center gap-x-5 gap-y-2 text-sm">
              <span data-testid="fusion-game-readiness" className={state.ready && !securityAttention ? 'text-fusion-accent' : 'text-white/70'}>{state.ready && !securityAttention ? text.ready : state.installed ? text.installed : text.available}</span>
              <span className="text-white/60" data-testid="fusion-game-version">{hasVersionUpdate ? `${text.version}: ${state.activeVersion} → ${state.version}` : `${text.version} ${state.activeVersion ?? state.version}`}</span>
              {hasVersionUpdate && <span className="rounded-sm bg-fusion-accent/10 px-2 py-1 text-xs font-semibold text-fusion-accent" data-testid="fusion-update-badge">{text.updateAvailable}</span>}
            </div>
            {otherBlockers.length > 0 && <div className="mb-5 border-l-2 border-[var(--rh-warning)] pl-3" data-testid="fusion-blockers">
              <h3 className="font-semibold">{text.blockers}</h3>
              {otherBlockers.map((blocker, index) => <p key={`${blocker.code}:${index}`} className="mt-2 select-text break-words text-sm text-white/70">{blockerMessage(blocker)}</p>)}
            </div>}
            {state.warnings.length > 0 && <details className="mb-5 text-sm text-white/60"><summary className="rh-focusable cursor-pointer rounded-sm text-white/70">{text.warnings}</summary>{state.warnings.map((warning, i) => <p className="mt-2" key={i}>{warning}</p>)}</details>}
            <div className="grid gap-5 border-t border-fusion-line py-5 sm:grid-cols-2">
              <fieldset disabled={disabled} className="min-w-0"><legend className="mb-3 text-sm font-semibold">{text.addons}</legend>
                {!addons.length && <p className="text-sm text-white/50">{text.noAddons}</p>}
                {addons.map((addon) => <label key={addon.id} className="mb-3 flex items-start gap-3 text-sm text-white/80"><input type="checkbox" className="mt-1 accent-fusion-accent" checked={options.enabledAddons?.includes(addon.id) ?? addon.enabled} onChange={(event) => {
                  const selected = new Set(options.enabledAddons ?? addons.filter((item) => item.enabled).map((item) => item.id));
                  if (event.target.checked) selected.add(addon.id); else selected.delete(addon.id);
                  setOptions({ ...options, enabledAddons: [...selected] });
                }} data-testid={`fusion-addon-${addon.id}`} /><span>{fusionLayerTitle(addon)}{addon.description && <span className="mt-1 block text-xs leading-relaxed text-white/50">{addon.description}</span>}</span></label>)}
              </fieldset>
              <div className="min-w-0"><label className="mb-3 block text-sm font-semibold" htmlFor="fusion-runtime-channel">{text.runtime}</label>
                {state.runtimeChannels.length > 0 ? <select id="fusion-runtime-channel" data-testid="fusion-runtime-channel" disabled={disabled} value={options.runtimeChannel ?? state.defaultRuntimeChannel ?? state.runtimeChannels[0]} onChange={(event) => setOptions({ ...options, runtimeChannel: event.target.value as 'pinned' | 'dynamic' })} className="rh-library-sort w-full">
                  {state.runtimeChannels.map((channel) => <option key={channel} value={channel}>{channel === 'dynamic' ? text.dynamic : text.pinned}</option>)}
                </select> : <p className="text-sm text-white/50">{text.noRuntime}</p>}
              </div>
              <details className="text-xs text-white/60 sm:col-span-2"><summary className="rh-focusable cursor-pointer rounded-sm">{text.externalRoot}</summary><input data-testid="fusion-workspace-root" aria-label={text.externalRoot} value={input.externalWorkspaceRoot ?? ''} placeholder={state.installed ? text.workspaceManaged : ''} onChange={(event) => setInput({ ...input, externalWorkspaceRoot: event.target.value || null })} className="mt-3 w-full rounded-sm border border-fusion-line bg-fusion-bg p-2 text-sm text-white disabled:text-white/60" readOnly={state.installed} disabled={disabled} />{state.installed && <p className="mt-2 leading-relaxed">{text.workspaceLocked}</p>}</details>
              <div className="sm:col-span-2">{state.installed ? <button type="button" data-testid="fusion-options-apply" className="rh-mini-action rh-focusable" disabled={disabled || blocked || !settingsDirty} onClick={() => void install()}>{settingsDirty ? text.apply : text.optionsApplied}</button> : <p className="text-xs text-white/50">{text.optionsOnInstall}</p>}</div>
            </div>
            {state.userLayers.length > 0 && <fieldset className="border-t border-fusion-line py-5" disabled={disabled}><legend className="text-sm font-semibold">{text.files}</legend>
              {state.userLayers.map((layer) => <div key={layer.layerId} className="mt-3" data-testid={`fusion-user-layer-${layer.layerId}`}>
                <p className="text-sm font-semibold">{fusionLayerTitle(state.addons.find((addon) => addon.id === layer.layerId) ?? { id: layer.layerId })}</p>{layer.instructions && <p className="mt-1 text-sm text-white/50">{layer.instructions}</p>}
                <div className="mt-2 flex flex-wrap items-center gap-2"><button type="button" onClick={() => void chooseFile(layer.layerId)} className="rh-mini-action rh-focusable">{text.choose}</button><button type="button" onClick={() => void chooseFile(layer.layerId, true)} className="rh-mini-action rh-focusable">{text.chooseFolder}</button><span className="min-w-0 break-all text-xs text-white/50">{input.userFiles[layer.layerId]}</span></div>
                {layer.rawPathRequired && <label className="mt-2 block text-xs text-white/60">{text.rawPath}<input value={input.rawPaths[layer.layerId] ?? ''} onChange={(event) => setInput((current) => ({ ...current, rawPaths: { ...current.rawPaths, [layer.layerId]: event.target.value } }))} className="mt-1 w-full rounded-sm border border-fusion-line bg-fusion-bg p-2 text-sm text-white" placeholder="game/game.bin" /></label>}
              </div>)}
            </fieldset>}
            {(state.executables.length > 0 || securityAttention) && <details open={securityAttention || securityExpanded} onToggle={(event) => { if (securityAttention) event.currentTarget.open = true; else setSecurityExpanded(event.currentTarget.open); }} data-testid="fusion-integrity-details" className={`mb-5 border-t pt-4 text-sm ${securityAttention ? 'border-[var(--rh-warning)] text-[var(--rh-warning)]' : 'border-fusion-line text-fusion-accent'}`}>
              <summary className="rh-focusable cursor-pointer rounded-sm font-semibold">{securityAttention ? text.integrityAttention : <><Check className="mr-1 inline h-4 w-4" />{text.verifiedFiles(state.executables.length)}</>}</summary>
              {securityAttention && <div className="mt-3" data-testid="fusion-integrity-attention"><p className="text-sm leading-relaxed">{repairRequired ? blockerMessage(integrityErrors[0]) : text.integrityTrust}</p>{repairRequired && <button type="button" className="rh-mini-action rh-focusable mt-3" disabled={disabled || blocked} data-testid="fusion-repair-files" onClick={() => void install()}><Download className="h-4 w-4" />{text.repair}</button>}</div>}
              {state.executables.map((exe) => <div key={`${exe.path}:${exe.blake3}`} className="mt-4 border-t border-fusion-line pt-3 text-xs">
                <p className="break-words font-medium text-white/80">{exe.path}</p><p className="mt-2 select-text break-all font-mono text-white/50">BLAKE3 {exe.blake3}</p>
                {exe.trusted ? <p className="mt-2 text-fusion-accent">{text.trusted}</p> : <button type="button" onClick={() => void run(() => api.fusionGameTrustExecutable(game.sourceId, game.gameId, exe.path, exe.blake3))} className="rh-mini-action rh-focusable mt-3" disabled={disabled || repairRequired} data-testid="fusion-trust-executable"><ShieldCheck className="h-4 w-4" />{text.trust}</button>}
              </div>)}
            </details>}
            {fusionOperationVisible(activeOperation) && activeOperation && <div className="mb-5 border-t border-fusion-line pt-4" role="status" data-testid="fusion-operation-progress">
              <p className={`text-sm font-semibold ${activeOperation.status === 'failed' ? 'text-[var(--rh-warning)]' : 'text-white/80'}`}>{text.operationPhase(activeOperation.status === 'failed' ? 'failed' : activeOperation.phase)}</p>
              {activeOperation.layerId && <p className="mt-1 text-xs text-white/60">{fusionLayerTitle(state.addons.find((addon) => addon.id === activeOperation.layerId) ?? { id: activeOperation.layerId })}</p>}
              {activeOperation.status !== 'failed' && <progress aria-label={text.operationPhase(activeOperation.phase)} max={1} value={progressRatio ?? undefined} className="mt-3 h-2 w-full accent-fusion-accent" />}
              {activeOperation.transfer && <p className="mt-2 text-xs text-white/60">{formatFusionBytes(activeOperation.transfer.downloadedBytes, locale)}{activeOperation.transfer.totalBytes > 0 ? ` / ${formatFusionBytes(activeOperation.transfer.totalBytes, locale)}` : ''} · {formatFusionBytes(activeOperation.transfer.speedBytesPerSecond, locale)}{text.perSecond}</p>}
              {activeOperation.error && <p className="mt-2 select-text break-words text-sm text-white/60">{blockerMessage(activeOperation.error)}</p>}
            </div>}
            {crashReports.length > 0 && <div className="border-t border-fusion-line py-4"><h3 className="mb-2 text-sm font-semibold">{text.crashes}</h3><ul className="divide-y divide-fusion-line">{crashReports.map((crash) => {
              const date = new Date(crash.createdAt);
              return <li key={crash.reportId} className="flex flex-wrap items-center justify-between gap-3 py-3" data-testid="fusion-crash-row" data-report-id={crash.reportId}><div className="min-w-0 text-xs text-white/60"><p className="text-sm text-white/80">{Number.isNaN(date.getTime()) ? `${text.crashReport} ${crash.reportId.slice(0, 8)}` : <time dateTime={crash.createdAt}>{new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'short' }).format(date)}</time>}</p><p className="mt-1">{crash.exitCode !== null ? `${text.exitCode}: ${crash.exitCode}` : `${text.crashSignature}: ${(crash.crashSignatureHash ?? crash.reportId).slice(0, 8)}`}{crash.exitCode !== null && crash.crashSignatureHash ? ` · ${crash.crashSignatureHash.slice(0, 8)}` : ''}</p></div><div className="flex flex-wrap gap-2"><button type="button" disabled={disabled} className="rh-mini-action rh-focusable" onClick={() => void run(async () => setReport(await api.fusionCrashPreview(crash.reportId)))} data-testid="fusion-crash-preview">{text.previewCrash}</button><button type="button" disabled={disabled} className="rh-mini-action rh-mini-action-danger rh-focusable" onClick={() => void discardReport(crash.reportId)} data-testid="fusion-crash-discard">{text.discard}</button></div></li>;
            })}</ul></div>}
            {report && <div className="border-t border-fusion-line py-4" data-testid="fusion-crash-report"><pre className="max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-sm bg-fusion-bg p-3 text-xs text-white/70">{JSON.stringify(report.report, null, 2)}</pre><p className="mt-2 break-all text-xs text-white/50">{report.canSend ? `${text.endpoint}: ${report.endpoint}` : text.noEndpoint}</p><button type="button" disabled={disabled || !report.canSend || !report.endpoint} className="rh-mini-action rh-focusable mt-3" onClick={() => void run(async () => { await api.fusionCrashSend(report.reportId, report.endpoint!); setReport(null); }, { notice: text.reported })} data-testid="fusion-crash-send">{text.send}</button></div>}
          </>}
        </div>
        <footer className="shrink-0 border-t border-fusion-line p-5"><div className="flex flex-wrap gap-2">
          <button type="button" disabled={disabled || !state?.ready || securityAttention} onClick={() => void run(() => api.fusionGameLaunch(game.sourceId, game.gameId))} className="rh-primary-action rh-focusable" data-testid="fusion-game-launch"><Play className="h-4 w-4" />{text.launch}</button>
          <button type="button" disabled={disabled || loading || !state || blocked} onClick={() => void install()} className={`${hasVersionUpdate || repairRequired ? 'rh-primary-action' : 'rh-mini-action'} rh-focusable`} data-testid="fusion-game-install"><Download className="h-4 w-4" />{state?.installed ? hasVersionUpdate || repairRequired ? text.update : text.rebuild : text.install}</button>
          <button type="button" disabled={disabled || !state?.rollbackAvailable} onClick={() => void run(() => api.fusionGameRollback(game.sourceId, game.gameId), { adoptDraft: true })} className="rh-mini-action rh-focusable" data-testid="fusion-game-rollback"><RotateCcw className="h-4 w-4" />{text.rollback}</button>
          {running && <button type="button" disabled={cancelPending} onClick={() => void cancel()} className="rh-mini-action rh-focusable" data-testid="fusion-game-cancel">{text.cancel}</button>}
          <button type="button" disabled={disabled || !state?.installed} className="rh-mini-action rh-mini-action-danger rh-focusable" onClick={() => { if (window.confirm(text.removeConfirm)) void run(() => api.fusionGameRemove(game.sourceId, game.gameId), { adoptDraft: true }); }} data-testid="fusion-game-remove">{text.remove}</button>
        </div><p className="mt-3 text-xs text-white/40">{text.rollbackNote}</p></footer>
      </section>
    </div>
  );
}
