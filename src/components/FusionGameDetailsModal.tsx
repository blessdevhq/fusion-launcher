'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { Check, Download, Loader2, Play, RotateCcw, ShieldCheck, X } from 'lucide-react';
import { useI18n } from './I18nProvider';
import { CreatorDeviceDialog } from './creator/CreatorDeviceDialog';
import { FusionSaveHistory, type FusionBackupStatus } from './FusionSaveHistory';
import { api } from '@/lib/api';
import { isInputOwnerActive } from '@/lib/focusManager';
import { effectiveFusionOptions, formatFusionBytes, fusionHasVersionUpdate, fusionIsIntegrityError, fusionLayerTitle, fusionOperationVisible, fusionProgressRatio, fusionSettingsDiffer, fusionStructuredError, selectableFusionAddons } from '@/lib/fusionGamePresentation';
import { fusionText } from '@/lib/fusionText';
import { fusionSetupDependency, fusionSetupPrimaryAction, fusionSetupRequest, fusionSetupRows, type FusionInputKinds } from '@/lib/fusionSetup';
import { isTauriRuntime } from '@/lib/runtime';
import { useCreatorStore } from '@/stores/creatorStore';
import { useFusionSourceStore, isFusionOperationTerminal } from '@/stores/fusionSourceStore';
import { fusionErrorMessage, fusionGameKey, type FusionCatalogEntry, type FusionCrashReport, type FusionError, type FusionGameOptions, type FusionGameState, type FusionInstallInput, type FusionOperationProgress } from '@/types/fusionSource';

export function FusionGameDetailsModal({ game, onClose, onRefresh, onCreateBuild }: {
  game: FusionCatalogEntry;
  onClose: () => void;
  onRefresh: () => Promise<void>;
  onCreateBuild?: () => void;
}) {
  const { locale, t } = useI18n();
  const text = fusionText(locale);
  const [state, setState] = useState<FusionGameState | null>(null);
  const [saveHistoryRevision, setSaveHistoryRevision] = useState<string | null>(null);
  const [backupStatus, setBackupStatus] = useState<FusionBackupStatus>('idle');
  const [options, setOptions] = useState<FusionGameOptions>({ enabledAddons: null, runtimeChannel: null });
  const [input, setInput] = useState<FusionInstallInput>({ userFiles: {}, rawPaths: {}, externalWorkspaceRoot: null });
  const [inputKinds, setInputKinds] = useState<FusionInputKinds>({});
  const [inputError, setInputError] = useState<FusionError | null>(null);
  const [optionalInputsExpanded, setOptionalInputsExpanded] = useState(false);
  const [busy, setBusy] = useState(false);
  const [cancelPending, setCancelPending] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [guardError, setGuardError] = useState<{ snapshotId: string | null; error: FusionError } | null>(null);
  const [securityExpanded, setSecurityExpanded] = useState(false);
  const [report, setReport] = useState<FusionCrashReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [deviceOpen, setDeviceOpen] = useState(false);
  const lastSavedBindings = useCreatorStore((store) => store.lastSavedBindings);
  const modalRef = useRef<HTMLElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const missingInputRef = useRef<HTMLButtonElement>(null);
  const integrityRef = useRef<HTMLDetailsElement>(null);
  const backupOperationId = useRef<string | null>(null);
  const backupInFlight = useRef(false);
  const stateRef = useRef(state);
  stateRef.current = state;
  const handleClose = useCallback(() => {
    const creator = useCreatorStore.getState();
    if (creator.inputSessionId && creator.inputOwnership === 'owned') {
      void creator.stopInputSession().catch(() => {});
    }
    onClose();
  }, [onClose]);
  const onCloseRef = useRef(handleClose);
  onCloseRef.current = handleClose;
  const refreshRef = useRef(onRefresh);
  refreshRef.current = onRefresh;
  const operation = useFusionSourceStore((store) => store.operations[fusionGameKey(game)]);
  const activeOperation = operation ?? state?.operation;
  const running = Boolean(activeOperation && !isFusionOperationTerminal(activeOperation));
  const isBackupOperation = Boolean(activeOperation && (activeOperation.phase === 'saveBackup' || activeOperation.operationId === backupOperationId.current));
  const disabled = busy || running || loading || backupStatus === 'pending';

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
    setLoading(true); setState(null); setReport(null); setGuardError(null); setSecurityExpanded(false); setDeviceOpen(false);
    setInput({ userFiles: {}, rawPaths: {}, externalWorkspaceRoot: null });
    setInputKinds({}); setInputError(null); setOptionalInputsExpanded(false);
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
      // A rebuild can publish a save backup even if its workspace never activates.
      setSaveHistoryRevision(operation.operationId);
      void reload().then(() => refreshRef.current()).catch((error) => setMessage(fusionErrorMessage(error)));
    }
  }, [operation, reload]);

  useEffect(() => {
    if (activeOperation?.phase === 'saveBackup') backupOperationId.current = activeOperation.operationId;
    if (!isBackupOperation && activeOperation?.status === 'failed' && activeOperation.error) setInputError(activeOperation.error);
  }, [activeOperation, isBackupOperation]);

  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    closeRef.current?.focus();
    const keydown = (event: KeyboardEvent) => {
      if (!isInputOwnerActive()) return;
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

  const describeFailure = (error: unknown) => {
    const structured = fusionStructuredError(error);
    const dependency = fusionSetupDependency(structured, stateRef.current?.addons ?? []);
    return dependency ? `${fusionErrorMessage(error)}\n${text.enableDependency}: ${dependency}.` : fusionErrorMessage(error);
  };
  const rememberFailure = (error: unknown) => {
    const structured = fusionStructuredError(error);
    setInputError(structured);
    if (structured && (fusionIsIntegrityError(structured) || structured.code === 'untrusted_executable')) {
      setGuardError({ snapshotId: stateRef.current?.activeSnapshotId ?? null, error: structured });
      useFusionSourceStore.getState().recordIntegrityError(game, stateRef.current?.activeSnapshotId ?? null, structured);
    }
    setMessage(describeFailure(error));
  };
  const run = async (action: () => Promise<unknown>, { notice, adoptDraft = false }: { notice?: string; adoptDraft?: boolean } = {}) => {
    setBusy(true); setMessage(null); setInputError(null);
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
    if (!state) return;
    await api.fusionGameSetOptions(game.sourceId, game.gameId, options);
    const request = fusionSetupRequest(state, options, input, inputKinds);
    return state.installed ? api.fusionGameUpdate(game.sourceId, game.gameId, request) : api.fusionGameInstall(game.sourceId, game.gameId, request);
  }, { adoptDraft: true, notice: state?.installed ? text.optionsSaved : undefined });
  const chooseFile = async (layerId: string, directory = false) => {
    setBusy(true);
    try {
      const selected = isTauriRuntime()
        ? await open({ multiple: false, directory, title: directory ? text.chooseFolder : text.choose })
        : `preview://UserFiles/${layerId}${directory ? '/' : '/game.bin'}`;
      if (typeof selected !== 'string') return;
      setInput((current) => ({ ...current, userFiles: { ...current.userFiles, [layerId]: selected } }));
      setInputKinds((current) => ({ ...current, [layerId]: directory ? 'directory' : 'file' }));
      setInputError((current) => current?.details.layerId === layerId ? null : current);
      setMessage(null);
    } catch (error) { rememberFailure(error); }
    finally { setBusy(false); }
  };
  const cancel = async () => {
    setCancelPending(true);
    try { await api.fusionGameCancel(game.sourceId, game.gameId); }
    catch (error) { rememberFailure(error); }
    finally { setCancelPending(false); }
  };
  const createBackup = async () => {
    if (disabled || backupInFlight.current || !state?.activeSnapshotId) return;
    backupInFlight.current = true;
    setBackupStatus('pending');
    const gameKey = fusionGameKey(game);
    // Native operations can finish before React renders the running event.
    const unsubscribe = useFusionSourceStore.subscribe(({ operations }) => {
      const progress = operations[gameKey];
      if (progress?.phase === 'saveBackup') backupOperationId.current = progress.operationId;
    });
    try {
      const result = await api.fusionGameCreateSaveSnapshot(game.sourceId, game.gameId);
      backupOperationId.current = result.operationId;
      useFusionSourceStore.getState().mergeOperation(result);
      setBackupStatus(result.status === 'completed' ? 'completed' : result.status === 'cancelled' ? 'cancelled' : backupFailureStatus(result.error));
    } catch (error) {
      setBackupStatus(backupFailureStatus(fusionStructuredError(error)));
    } finally {
      unsubscribe();
      backupInFlight.current = false;
    }
  };
  const discardReport = (reportId: string) => run(async () => {
    await api.fusionCrashDiscard(reportId);
    setReport((current) => current?.reportId === reportId ? null : current);
  }, { notice: text.discarded });
  const integrityErrors = [...state?.blockers ?? [], ...(guardError ? [guardError.error] : [])].filter(fusionIsIntegrityError);
  const repairRequired = integrityErrors.length > 0;
  const trustRequired = Boolean(state?.executables.some((exe) => !exe.trusted) || state?.blockers.some((blocker) => blocker.code === 'untrusted_executable') || guardError?.error.code === 'untrusted_executable');
  const securityAttention = repairRequired || trustRequired;
  const blocked = Boolean(state?.blockers.some((blocker) => !['untrusted_executable', 'missing_injector_dll', 'corrupted_critical_binary', 'local_base_modified'].includes(blocker.code)));
  const otherBlockers = state?.blockers.filter((blocker) => !fusionIsIntegrityError(blocker) && blocker.code !== 'untrusted_executable') ?? [];
  const hasVersionUpdate = Boolean(state && fusionHasVersionUpdate(state));
  const settingsDirty = Boolean(state && fusionSettingsDiffer(state, options, input.externalWorkspaceRoot));
  const addons = state ? selectableFusionAddons(state.addons) : [];
  const buildProfiles = state?.buildProfiles ?? [];
  const controlsProfiles = state?.controlsProfiles ?? [];
  const selectedBuild = buildProfiles.find((profile) => profile.id === options.buildProfileId) ?? buildProfiles[0] ?? null;
  const allowedControls = selectedBuild
    ? controlsProfiles.filter((profile) => selectedBuild.controlsProfileIds.includes(profile.id))
    : controlsProfiles;
  const selectedControls = allowedControls.find((profile) => profile.id === options.controlsProfileId) ?? allowedControls[0] ?? null;
  const progressRatio = fusionProgressRatio(activeOperation?.transfer);
  const crashReports = state?.crashReports ?? state?.crashReportIds.map((reportId) => ({ reportId, createdAt: '', exitCode: null, crashSignatureHash: null })) ?? [];
  const blockerMessage = (error: FusionError) => error.code === 'local_base_modified' ? text.localBaseModified : fusionIsIntegrityError(error) ? text.integrityRepair : error.code === 'untrusted_executable' ? text.integrityTrust : describeFailure(error);
  const setupRows = state ? fusionSetupRows(state, options, input, inputKinds, inputError) : [];
  const requiredRows = setupRows.filter((row) => !row.optional);
  const optionalRows = setupRows.filter((row) => row.optional);
  const primary = state && fusionSetupPrimaryAction(state, setupRows, { blocked, repair: repairRequired, trust: trustRequired });
  const primaryKind = primary?.kind;
  const installLabel = state?.installed ? hasVersionUpdate ? text.update : text.rebuild : text.install;
  const primaryLabel = primaryKind === 'choose' ? text.chooseGameFiles : primaryKind === 'review' ? text.reviewLaunch : primaryKind === 'play' ? text.playGame : primaryKind === 'repair' ? text.repair : installLabel;
  const PrimaryIcon = primaryKind === 'play' ? Play : primaryKind === 'review' ? ShieldCheck : Download;
  const launchGame = () => run(async () => {
    const creator = useCreatorStore.getState();
    if (creator.inputSessionId && creator.inputOwnership === 'owned') {
      await creator.releaseInputForLaunch();
    }
    return api.fusionGameLaunch(game.sourceId, game.gameId);
  });
  const actOnPrimary = () => {
    if (disabled || !primary) return;
    switch (primary.kind) {
      case 'choose': missingInputRef.current?.focus(); break;
      case 'review': (integrityRef.current?.querySelector<HTMLButtonElement>('button:not(:disabled)') ?? integrityRef.current?.querySelector('summary'))?.focus(); break;
      case 'play': void launchGame(); break;
      case 'repair':
      case 'install': void install(); break;
    }
  };
  const renderSetupRow = (layer: typeof setupRows[number]) => (
    <div key={layer.layerId} className="mt-4 min-w-0" data-testid={`fusion-user-layer-${layer.layerId}`}>
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <p className="break-words text-sm font-semibold">{layer.title}</p>
        <span data-testid={`fusion-input-status-${layer.layerId}`} className="text-xs text-white/60">{layer.status === 'disabled' ? text.inputDisabled : layer.status === 'selected' ? text.inputSelected : text.inputNotSelected}</span>
      </div>
      {layer.instructions && <p className="mt-1 break-words text-sm text-white/60">{layer.instructions}</p>}
      <p className="mt-1 text-xs text-white/50">{text.inputFormat}: {layer.archive === 'raw' ? text.rawInputFormat : layer.archive.toUpperCase()}</p>
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <button type="button" ref={primary?.kind === 'choose' && primary.layerId === layer.layerId ? missingInputRef : undefined} disabled={!layer.active} onClick={() => void chooseFile(layer.layerId)} className="rh-mini-action rh-focusable">{text.choose}</button>
        {layer.archive === 'raw' && <button type="button" disabled={!layer.active} onClick={() => void chooseFile(layer.layerId, true)} className="rh-mini-action rh-focusable">{text.chooseFolder}</button>}
        {layer.path && <button type="button" disabled={!layer.active} onClick={() => {
          setInput((current) => ({ ...current, userFiles: Object.fromEntries(Object.entries(current.userFiles).filter(([id]) => id !== layer.layerId)), rawPaths: Object.fromEntries(Object.entries(current.rawPaths).filter(([id]) => id !== layer.layerId)) }));
          setInputKinds((current) => Object.fromEntries(Object.entries(current).filter(([id]) => id !== layer.layerId)));
          setInputError((current) => current?.details.layerId === layer.layerId ? null : current);
        }} className="rh-mini-action rh-focusable">{text.clearInput}</button>}
      </div>
      {layer.path && <p data-testid={`fusion-input-path-${layer.layerId}`} className="mt-2 select-text break-all text-xs text-white/70">{layer.path}</p>}
      {layer.active && layer.destinationRequired && <label className="mt-3 block text-xs text-white/60">{text.rawPath}<input data-testid={`fusion-input-destination-${layer.layerId}`} value={input.rawPaths[layer.layerId] ?? ''} onChange={(event) => {
        setInput((current) => ({ ...current, rawPaths: { ...current.rawPaths, [layer.layerId]: event.target.value } }));
        setInputError((current) => current?.details.layerId === layer.layerId ? null : current);
      }} className="mt-1 w-full rounded-sm border border-fusion-line bg-fusion-bg p-2 text-sm text-white" placeholder="game/game.bin" /><span className="mt-1 block leading-relaxed">{text.inputDestination}</span></label>}
      {layer.error && <p role="alert" data-testid={`fusion-input-error-${layer.layerId}`} className="mt-2 select-text break-words text-sm text-[var(--rh-warning)]">{layer.error}<span className="mt-1 block text-xs">{layer.dependencyTitle ? `${text.enableDependency}: ${layer.dependencyTitle}.` : text.inputRetry}</span></p>}
    </div>
  );

  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/80 px-5">
      <section ref={modalRef} role="dialog" aria-modal="true" aria-labelledby="fusion-game-title" data-testid="fusion-game-modal" className="relative flex max-h-[88vh] w-full max-w-3xl flex-col overflow-hidden rounded-2xl border border-fusion-line bg-fusion-surface text-white">
        <header className="flex items-start gap-4 border-b border-fusion-line p-5">
          <div className="min-w-0 flex-1">
            <h2 id="fusion-game-title" className="break-words text-2xl font-bold">{game.title}</h2>
            <p className="mt-1 text-sm text-white/50">{state?.platform?.toUpperCase() ?? text.noPlatform} · {game.sourceName}</p>
            {onCreateBuild && <button type="button" data-testid="creator-context-action" onClick={onCreateBuild} className="rh-filter-chip rh-focusable mt-3">{t.creator.ui.create}</button>}
          </div>
          <button ref={closeRef} type="button" onClick={handleClose} className="rh-icon-button rh-focusable" aria-label={text.close} data-testid="fusion-game-close"><X className="h-4 w-4" /></button>
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
            {requiredRows.length > 0 && <section aria-labelledby="fusion-before-play" className="border-t border-fusion-line py-4">
              <h3 id="fusion-before-play" className="text-sm font-semibold">{text.beforePlay}</h3>
              <p className="mt-2 text-xs leading-relaxed text-white/70">{text.inputNote}</p>
              {state.installed && <p className="mt-2 text-xs leading-relaxed text-white/60">{text.inputReuse}</p>}
              <fieldset disabled={disabled} aria-label={text.requiredInputs} data-testid="fusion-setup-required">{requiredRows.map(renderSetupRow)}</fieldset>
            </section>}
            {state.warnings.length > 0 && <details className="mb-5 text-sm text-white/60"><summary className="rh-focusable cursor-pointer rounded-sm text-white/70">{text.warnings}</summary>{state.warnings.map((warning, i) => <p className="mt-2" key={i}>{warning}</p>)}</details>}
            <details className="border-t border-fusion-line py-4 text-sm" data-testid="fusion-game-options">
              <summary className="rh-focusable cursor-pointer rounded-sm font-semibold text-white/80">{text.gameOptions}</summary>
              <div className="grid gap-5 pt-4 sm:grid-cols-2">
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
              {buildProfiles.length > 0 && <div className="min-w-0"><label className="mb-2 block text-sm font-semibold" htmlFor="fusion-build-profile">{text.buildProfile}</label>
                <select id="fusion-build-profile" data-testid="fusion-build-profile" disabled={disabled} value={selectedBuild?.id ?? ''} onChange={(event) => {
                  const nextBuild = buildProfiles.find((profile) => profile.id === event.target.value) ?? buildProfiles[0];
                  const nextControls = options.controlsProfileId && nextBuild.controlsProfileIds.includes(options.controlsProfileId)
                    ? options.controlsProfileId
                    : (nextBuild.controlsProfileIds[0] ?? null);
                  setOptions({ buildProfileId: nextBuild.id, controlsProfileId: nextControls, enabledAddons: null, runtimeChannel: null });
                }} className="rh-library-sort w-full">
                  {buildProfiles.map((profile) => <option key={profile.id} value={profile.id}>{profile.title || profile.id}</option>)}
                </select>
              </div>}
              {allowedControls.length > 0 && <div className="min-w-0"><label className="mb-2 block text-sm font-semibold" htmlFor="fusion-controls-profile">{text.controlsProfile}</label>
                <select id="fusion-controls-profile" data-testid="fusion-controls-profile" disabled={disabled} value={selectedControls?.id ?? ''} onChange={(event) => {
                  setOptions({
                    buildProfileId: selectedBuild?.id ?? null,
                    controlsProfileId: event.target.value,
                    enabledAddons: null,
                    runtimeChannel: null,
                  });
                }} className="rh-library-sort w-full">
                  {allowedControls.map((profile) => <option key={profile.id} value={profile.id}>{profile.title || profile.id}</option>)}
                </select>
                {selectedControls?.device === 'sdl_gamecontroller' && <div className="mt-2 flex flex-wrap items-center gap-2">
                  <button type="button" data-testid="fusion-configure-controller" disabled={disabled} onClick={() => setDeviceOpen(true)} className="rh-mini-action rh-focusable">{text.configureController}</button>
                  {lastSavedBindings?.controlsProfileId === selectedControls.id && <span className="text-xs text-fusion-accent">{text.controllerSaved}</span>}
                </div>}
              </div>}
              <details className="text-xs text-white/60 sm:col-span-2"><summary className="rh-focusable cursor-pointer rounded-sm">{text.externalRoot}</summary><input data-testid="fusion-workspace-root" aria-label={text.externalRoot} value={input.externalWorkspaceRoot ?? ''} placeholder={state.installed ? text.workspaceManaged : ''} onChange={(event) => setInput({ ...input, externalWorkspaceRoot: event.target.value || null })} className="mt-3 w-full rounded-sm border border-fusion-line bg-fusion-bg p-2 text-sm text-white disabled:text-white/60" readOnly={state.installed} disabled={disabled} />{state.installed && <p className="mt-2 leading-relaxed">{text.workspaceLocked}</p>}</details>
              <div className="sm:col-span-2">{state.installed ? <button type="button" data-testid="fusion-options-apply" className="rh-mini-action rh-focusable" disabled={disabled || blocked || !settingsDirty} onClick={() => void install()}>{settingsDirty ? text.apply : text.optionsApplied}</button> : <p className="text-xs text-white/50">{text.optionsOnInstall}</p>}</div>
              </div>
            </details>
            {optionalRows.length > 0 && <details data-testid="fusion-setup-optional" open={optionalInputsExpanded || optionalRows.some((layer) => layer.error)} onToggle={(event) => setOptionalInputsExpanded(event.currentTarget.open)} className="border-t border-fusion-line py-4 text-sm"><summary className="rh-focusable cursor-pointer rounded-sm font-semibold text-white/80">{text.optionalInputs}</summary><fieldset aria-label={text.optionalInputs} disabled={disabled}>{optionalRows.map(renderSetupRow)}</fieldset></details>}
            {(state.executables.length > 0 || securityAttention) && <details ref={integrityRef} open={securityAttention || securityExpanded} onToggle={(event) => { if (securityAttention) event.currentTarget.open = true; else setSecurityExpanded(event.currentTarget.open); }} data-testid="fusion-integrity-details" className={`mb-5 border-t pt-4 text-sm ${securityAttention ? 'border-[var(--rh-warning)] text-[var(--rh-warning)]' : 'border-fusion-line text-fusion-accent'}`}>
              <summary className="rh-focusable cursor-pointer rounded-sm font-semibold">{securityAttention ? text.integrityAttention : <><Check className="mr-1 inline h-4 w-4" />{text.verifiedFiles(state.executables.length)}</>}</summary>
              {securityAttention && <div className="mt-3" data-testid="fusion-integrity-attention"><p className="text-sm leading-relaxed">{repairRequired ? blockerMessage(integrityErrors[0]) : text.integrityTrust}</p></div>}
              {state.executables.map((exe) => <div key={`${exe.path}:${exe.blake3}`} className="mt-4 border-t border-fusion-line pt-3 text-xs">
                <p className="break-words font-medium text-white/80">{exe.path}</p><p className="mt-2 select-text break-all font-mono text-white/50">BLAKE3 {exe.blake3}</p>
                {exe.trusted ? <p className="mt-2 text-fusion-accent">{text.trusted}</p> : <button type="button" onClick={() => void run(() => api.fusionGameTrustExecutable(game.sourceId, game.gameId, exe.path, exe.blake3))} className="rh-mini-action rh-focusable mt-3" disabled={disabled || repairRequired} data-testid="fusion-trust-executable"><ShieldCheck className="h-4 w-4" />{text.trust}</button>}
              </div>)}
            </details>}
            {fusionOperationVisible(activeOperation) && activeOperation && !(isBackupOperation && activeOperation.status === 'failed') && <div className="mb-5 border-t border-fusion-line pt-4" role="status" data-testid="fusion-operation-progress">
              <p className={`text-sm font-semibold ${activeOperation.status === 'failed' ? 'text-[var(--rh-warning)]' : 'text-white/80'}`}>{text.operationPhase(activeOperation.status === 'failed' ? 'failed' : activeOperation.phase)}</p>
              {activeOperation.layerId && <p className="mt-1 text-xs text-white/60">{fusionLayerTitle(state.addons.find((addon) => addon.id === activeOperation.layerId) ?? { id: activeOperation.layerId })}</p>}
              {activeOperation.status !== 'failed' && <progress aria-label={text.operationPhase(activeOperation.phase)} max={1} value={progressRatio ?? undefined} className="mt-3 h-2 w-full accent-fusion-accent" />}
              {activeOperation.transfer && <p className="mt-2 text-xs text-white/60">{formatFusionBytes(activeOperation.transfer.downloadedBytes, locale)}{activeOperation.transfer.totalBytes > 0 ? ` / ${formatFusionBytes(activeOperation.transfer.totalBytes, locale)}` : ''} · {formatFusionBytes(activeOperation.transfer.speedBytesPerSecond, locale)}{text.perSecond}</p>}
              {activeOperation.error && <p className="mt-2 select-text break-words text-sm text-white/60">{blockerMessage(activeOperation.error)}</p>}
            </div>}
            {state.installed && <FusionSaveHistory key={fusionGameKey(game)} sourceId={game.sourceId} gameId={game.gameId} revision={saveHistoryRevision} backupDisabled={disabled || !state.activeSnapshotId} backupStatus={backupStatus} onBackup={() => void createBackup()} />}
            {crashReports.length > 0 && <div className="border-t border-fusion-line py-4"><h3 className="mb-2 text-sm font-semibold">{text.crashes}</h3><ul className="divide-y divide-fusion-line">{crashReports.map((crash) => {
              const date = new Date(crash.createdAt);
              return <li key={crash.reportId} className="flex flex-wrap items-center justify-between gap-3 py-3" data-testid="fusion-crash-row" data-report-id={crash.reportId}><div className="min-w-0 text-xs text-white/60"><p className="text-sm text-white/80">{Number.isNaN(date.getTime()) ? `${text.crashReport} ${crash.reportId.slice(0, 8)}` : <time dateTime={crash.createdAt}>{new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'short' }).format(date)}</time>}</p><p className="mt-1">{crash.exitCode !== null ? `${text.exitCode}: ${crash.exitCode}` : `${text.crashSignature}: ${(crash.crashSignatureHash ?? crash.reportId).slice(0, 8)}`}{crash.exitCode !== null && crash.crashSignatureHash ? ` · ${crash.crashSignatureHash.slice(0, 8)}` : ''}</p></div><div className="flex flex-wrap gap-2"><button type="button" disabled={disabled} className="rh-mini-action rh-focusable" onClick={() => void run(async () => setReport(await api.fusionCrashPreview(crash.reportId)))} data-testid="fusion-crash-preview">{text.previewCrash}</button><button type="button" disabled={disabled} className="rh-mini-action rh-mini-action-danger rh-focusable" onClick={() => void discardReport(crash.reportId)} data-testid="fusion-crash-discard">{text.discard}</button></div></li>;
            })}</ul></div>}
            {report && <div className="border-t border-fusion-line py-4" data-testid="fusion-crash-report"><pre className="max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-sm bg-fusion-bg p-3 text-xs text-white/70">{JSON.stringify(report.report, null, 2)}</pre><p className="mt-2 break-all text-xs text-white/50">{report.canSend ? `${text.endpoint}: ${report.endpoint}` : text.noEndpoint}</p><button type="button" disabled={disabled || !report.canSend || !report.endpoint} className="rh-mini-action rh-focusable mt-3" onClick={() => void run(async () => { await api.fusionCrashSend(report.reportId, report.endpoint!); setReport(null); }, { notice: text.reported })} data-testid="fusion-crash-send">{text.send}</button></div>}
          </>}
        </div>
        <footer className="shrink-0 border-t border-fusion-line p-5"><div className="flex flex-wrap gap-2">
          <button type="button" disabled={disabled || !primary} onClick={actOnPrimary} className="rh-primary-action rh-focusable" data-fusion-primary="" data-testid={primaryKind === 'play' ? 'fusion-game-launch' : primaryKind === 'repair' ? 'fusion-repair-files' : primaryKind === 'choose' || primaryKind === 'review' ? 'fusion-game-next' : 'fusion-game-install'}><PrimaryIcon className="h-4 w-4" />{primaryLabel}</button>
          {primaryKind === 'play' && <button type="button" disabled={disabled} onClick={() => void install()} className="rh-mini-action rh-focusable" data-testid="fusion-game-install"><Download className="h-4 w-4" />{installLabel}</button>}
          <button type="button" disabled={disabled || !state?.rollbackAvailable} onClick={() => void run(() => api.fusionGameRollback(game.sourceId, game.gameId), { adoptDraft: true })} className="rh-mini-action rh-focusable" data-testid="fusion-game-rollback"><RotateCcw className="h-4 w-4" />{text.rollback}</button>
          {running && <button type="button" disabled={cancelPending} onClick={() => void cancel()} className="rh-mini-action rh-focusable" data-testid="fusion-game-cancel">{text.cancel}</button>}
          <button type="button" disabled={disabled || !state?.installed} className="rh-mini-action rh-mini-action-danger rh-focusable" onClick={() => { if (window.confirm(text.removeConfirm)) void run(() => api.fusionGameRemove(game.sourceId, game.gameId), { adoptDraft: true }); }} data-testid="fusion-game-remove">{text.remove}</button>
        </div><p className="mt-3 text-xs text-white/40">{text.rollbackNote}</p></footer>
      </section>
      {deviceOpen && selectedControls && <CreatorDeviceDialog controlsProfileId={selectedControls.id} keepSessionOnSave onClose={() => setDeviceOpen(false)} />}
    </div>
  );
}

function backupFailureStatus(error: FusionError | null): FusionBackupStatus {
  if (error?.code === 'operation_in_progress') return 'busy';
  if (error?.code === 'cancelled') return 'cancelled';
  if (error?.code === 'persistence_conflict' && error.details.reason === 'no_save_files') return 'noFiles';
  return 'failed';
}
