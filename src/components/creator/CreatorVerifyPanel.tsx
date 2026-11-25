'use client';

import { useRef, useState } from 'react';
import { useI18n } from '@/components/I18nProvider';
import {
  canResumeInterruptedJob,
  creatorReportKinds,
  currentRevisionReleases,
  declaredProfilePairs,
  summarizePairReports,
  supportsCleanVerify,
  verifyReleaseRequest,
  type CreatorStage
} from '@/lib/creatorUi';
import { useCreatorStore } from '@/stores/creatorStore';
import { CreatorStatus } from './CreatorStatus';

const actionClass = 'rh-filter-chip rh-focusable disabled:opacity-40';
const selectClass = 'rh-focusable h-9 min-w-0 max-w-full rounded-md border border-fusion-line bg-fusion-raised px-2 text-sm text-[var(--rh-text)]';

export function CreatorVerifyPanel({ onSelectStage }: { onSelectStage: (stage: CreatorStage) => void }) {
  const { t } = useI18n();
  const text = t.creator.ui.verify;
  const state = useCreatorStore();
  const [buildIdDraft, setBuildId] = useState('');
  const [controlsDraft, setControlsId] = useState('');
  const [starting, setStarting] = useState(false);
  const locked = useRef(false);

  const opened = state.currentProject;
  if (!opened) {
    if (state.projectsStatus === 'unavailable' || state.projectStatus === 'unavailable') {
      return <section className="border-t border-fusion-line py-4" aria-labelledby="creator-verify-title">
        <h2 id="creator-verify-title" className="mb-2 text-lg font-semibold">{text.title}</h2>
        <CreatorStatus status="unavailable" error={state.projectError ?? state.projectsError} testId="creator-verify-unavailable" />
      </section>;
    }
    return <section className="border-t border-fusion-line py-4" aria-labelledby="creator-verify-title" data-testid="creator-verify-no-project">
      <h2 id="creator-verify-title" className="text-lg font-semibold">{text.title}</h2>
      <p className="mt-1 text-sm text-[var(--rh-muted)]">{text.noProject}</p>
      <button type="button" data-testid="creator-verify-go-build" data-focus-id="creator:verify-go-build" className={`${actionClass} mt-3`} onClick={() => onSelectStage('base')}>{text.goToBuild}</button>
    </section>;
  }

  const { project } = opened;
  const descriptor = project.program_descriptor;
  const revReleases = currentRevisionReleases(state.releases, project);
  const selectedRelease = revReleases.find(release => release.releaseId === state.selectedReleaseId)
    ?? revReleases[revReleases.length - 1]
    ?? null;

  const buildId = descriptor.build_profiles.some(profile => profile.id === buildIdDraft)
    ? buildIdDraft
    : (descriptor.build_profiles[0]?.id ?? '');
  const applicableControls = descriptor.controls_profiles.filter(profile =>
    descriptor.build_profiles.find(build => build.id === buildId)?.controls_profile_ids.includes(profile.id));
  const controlsId = applicableControls.some(profile => profile.id === controlsDraft)
    ? controlsDraft
    : (applicableControls[0]?.id ?? '');

  const pairs = declaredProfilePairs(descriptor);
  const cleanVerifySupported = supportsCleanVerify(descriptor);
  const envNames = descriptor.entrypoint.environment_paths.map(entry => entry.name);

  const projectJobs = Object.values(state.jobs).filter(({ receipt }) =>
    receipt.projectId === project.project_id && receipt.projectRevision === project.revision);
  const writerBusy = projectJobs.some(({ kind, progress }) =>
    (kind === 'build' || kind === 'verify')
    && (!progress || progress.status === 'queued' || progress.status === 'running' || progress.status === 'cancelling'));

  const matchingVerifyJobs = projectJobs.filter(entry =>
    entry.kind === 'verify'
    && (!selectedRelease || !entry.progress?.releaseId || entry.progress.releaseId === selectedRelease.releaseId));
  const selectedVerify = state.selectedJobId
    ? matchingVerifyJobs.find(entry => entry.receipt.jobId === state.selectedJobId)
    : undefined;
  const activeVerifyJob = selectedVerify ?? matchingVerifyJobs[matchingVerifyJobs.length - 1] ?? null;

  const activeProgress = activeVerifyJob?.progress ?? null;
  const cancellable = Boolean(
    activeVerifyJob?.acceptingEvents
    && (activeProgress?.status === 'queued' || activeProgress?.status === 'running')
  );
  const interrupted = activeProgress?.status === 'interrupted';
  const resumable = Boolean(activeVerifyJob && canResumeInterruptedJob(activeVerifyJob));
  const request = selectedRelease && cleanVerifySupported
    ? verifyReleaseRequest(project, selectedRelease.releaseId, buildId, controlsId)
    : null;
  const verifyError = activeProgress?.error
    ?? activeVerifyJob?.error
    ?? (state.selectedJobId === activeVerifyJob?.receipt.jobId || !activeVerifyJob ? state.jobError : null);
  const passedStages = verifyError && Array.isArray(verifyError.details.passedStages)
    ? verifyError.details.passedStages.map(item => String(item))
    : [];

  async function startVerify() {
    if (!request || locked.current || writerBusy || starting) return;
    locked.current = true;
    setStarting(true);
    try {
      await useCreatorStore.getState().startJob('verify', request);
    } catch {
      // Store surfaces jobError cleanly for the active project revision.
    } finally {
      locked.current = false;
      setStarting(false);
    }
  }

  const pairResults = selectedRelease
    ? summarizePairReports(selectedRelease, state.reports, buildId, controlsId)
    : null;

  return <section className="border-t border-fusion-line py-4" aria-labelledby="creator-verify-title" data-testid="creator-verify-panel">
    <div className="flex flex-wrap items-center justify-between gap-2">
      <h2 id="creator-verify-title" className="text-lg font-semibold">{text.title}</h2>
      <button type="button" className={actionClass} data-focus-id="creator:verify-refresh" onClick={() => void state.loadReleases(project.project_id).catch(() => {})}>{t.creator.ui.refresh}</button>
    </div>
    <p className="mt-1 text-sm text-[var(--rh-muted)]">{text.note}</p>

    {state.releasesStatus !== 'ready' && state.releasesStatus !== 'idle' && (
      <div className="mt-2">
        <CreatorStatus status={state.releasesStatus} error={state.releasesError} />
      </div>
    )}

    {!cleanVerifySupported && (
      <div data-testid="creator-verify-unsupported" role="status" className="mt-3 border-l-2 border-fusion-line pl-3 text-sm text-[var(--rh-muted)]">
        <p>{envNames.length > 0 ? `${text.envApprovalRequired} (${envNames.join(', ')})` : text.unsupported}</p>
      </div>
    )}

    {!selectedRelease ? (
      <div className="mt-3 space-y-2 text-sm text-[var(--rh-muted)]">
        <p>{text.noRelease}</p>
        <button type="button" data-testid="creator-verify-go-build" data-focus-id="creator:verify-go-build" className={actionClass} onClick={() => onSelectStage('base')}>{text.goToBuild}</button>
      </div>
    ) : <>
      <div className="mt-4 flex flex-wrap gap-3">
        <label className="grid min-w-0 gap-1 text-xs text-[var(--rh-muted)]">{text.release}
          <select name="verifyRelease" data-testid="creator-verify-release" data-focus-id="creator:verify-release" className={selectClass} disabled={writerBusy || starting} value={selectedRelease.releaseId} onChange={event => state.selectRelease(event.target.value)}>
            {revReleases.map(release => <option key={release.releaseId} value={release.releaseId}>{release.releaseId}</option>)}
          </select>
        </label>
        <label className="grid min-w-0 gap-1 text-xs text-[var(--rh-muted)]">{text.buildProfile}
          <select name="verifyBuildProfile" data-testid="creator-verify-build-profile" data-focus-id="creator:verify-build-profile" className={selectClass} disabled={writerBusy || starting} value={buildId} onChange={event => setBuildId(event.target.value)}>
            {descriptor.build_profiles.map(profile => <option key={profile.id} value={profile.id}>{profile.title}</option>)}
          </select>
        </label>
        <label className="grid min-w-0 gap-1 text-xs text-[var(--rh-muted)]">{text.controlsProfile}
          <select name="verifyControlsProfile" data-testid="creator-verify-controls-profile" data-focus-id="creator:verify-controls-profile" className={selectClass} disabled={writerBusy || starting} value={controlsId} onChange={event => setControlsId(event.target.value)}>
            {applicableControls.map(profile => <option key={profile.id} value={profile.id}>{profile.title}</option>)}
          </select>
        </label>
      </div>

      <p data-testid="creator-verify-release-hash" className="mt-2 select-text break-all font-mono text-xs text-[var(--rh-muted)]">
        {text.manifestHash}: {selectedRelease.manifestBlake3}
      </p>

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <button type="button" data-testid="creator-verify-start" data-focus-id="creator:verify-start" className={actionClass} disabled={!request || writerBusy || starting} onClick={() => void startVerify()}>
          {activeProgress?.status === 'failed' || activeProgress?.status === 'cancelled' ? text.retry : text.action}
        </button>
        {cancellable && activeVerifyJob && (
          <button type="button" data-testid="creator-verify-cancel" data-focus-id="creator:verify-cancel" className={actionClass} onClick={() => void state.cancelJob(activeVerifyJob.receipt.jobId).catch(() => {})}>
            {t.creator.ui.cancel}
          </button>
        )}
        {interrupted && resumable && activeVerifyJob && (
          <button type="button" data-testid="creator-verify-resume" data-focus-id="creator:verify-resume" className={actionClass} disabled={writerBusy || starting} onClick={() => void state.resumeJob(activeVerifyJob.receipt.jobId).catch(() => {})}>
            {t.creator.ui.resume}
          </button>
        )}
        {interrupted && (
          <button type="button" data-testid="creator-verify-restart" data-focus-id="creator:verify-restart" className={actionClass} disabled={!request || writerBusy || starting} onClick={() => void startVerify()}>
            {text.restart}
          </button>
        )}
      </div>

      {interrupted && (
        <p role="status" className="mt-2 text-xs text-[var(--rh-muted)]">
          {resumable ? text.resumeNotice : text.restartOnlyNotice}
        </p>
      )}

      {verifyError && (
        <div data-testid="creator-verify-error" role="alert" className="mt-3 border-l-2 border-[var(--rh-danger)] pl-3 text-sm text-[var(--rh-danger)]">
          <p className="break-words font-mono text-xs">{verifyError.code} · {verifyError.message}</p>
          {passedStages.length > 0 && <p className="mt-1 text-xs">{text.passedStages}: {passedStages.join(', ')}</p>}
        </div>
      )}

      {activeVerifyJob && activeProgress && (
        <div data-testid="creator-verify-job" className="mt-4 border-t border-fusion-line pt-3 text-sm">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="font-medium">{text.jobTitle} · <span className="font-mono text-xs">{activeVerifyJob.receipt.jobId}</span></span>
            <span data-testid="creator-verify-job-status" data-status={activeProgress.status}>{t.creator.ui.statuses[activeProgress.status]}</span>
          </div>
          <p className="mt-1 text-xs text-[var(--rh-muted)]">
            {t.creator.ui.phase}: {activeProgress.phase} · {text.progress}: {activeProgress.completedUnits}{activeProgress.totalUnits !== null ? `/${activeProgress.totalUnits}` : ''} · {t.creator.ui.sequence}: {activeProgress.sequence}
          </p>
        </div>
      )}

      {pairResults && (
        <div className="mt-4 border-t border-fusion-line pt-3">
          <h3 className="mb-2 text-sm font-semibold">{text.resultsTitle}</h3>
          <ul data-testid="creator-verify-results" className="divide-y divide-fusion-line">
            {creatorReportKinds.map(kind => {
              const item = pairResults[kind];
              return <li key={kind} data-testid={`creator-verify-result-${kind}`} data-status={item.status} className="py-2 text-sm">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span>{t.creator.ui.reportKinds[kind]}</span>
                  <span className="text-xs text-[var(--rh-muted)]">{t.creator.ui.statuses[item.status]}</span>
                </div>
                {item.report && <p className="mt-1 break-all font-mono text-xs text-[var(--rh-muted)]">{text.reportId}: {item.report.reportId}</p>}
                {item.report && item.report.diagnostics.length > 0 && (
                  <p className="mt-1 break-words text-xs text-[var(--rh-danger)]">
                    {item.report.diagnostics.map(diag => `${diag.code}: ${diag.message}`).join(' · ')}
                  </p>
                )}
              </li>;
            })}
            <li
              key="input_verified"
              data-testid="creator-verify-result-input"
              data-status={controlsId && state.lastSavedBindings?.controlsProfileId === controlsId ? 'passed' : 'not_tested'}
              className="py-2 text-sm"
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <span>{text.inputVerified}</span>
                <span className="text-xs text-[var(--rh-muted)]">
                  {t.creator.ui.statuses[controlsId && state.lastSavedBindings?.controlsProfileId === controlsId ? 'passed' : 'not_tested']}
                </span>
              </div>
            </li>
          </ul>
        </div>
      )}

      {pairs.length > 0 && (
        <div className="mt-4 border-t border-fusion-line pt-3">
          <h3 className="mb-2 text-sm font-semibold">{text.pairsTitle}</h3>
          <ul data-testid="creator-verify-pairs" className="divide-y divide-fusion-line">
            {pairs.map(pair => {
              const summary = summarizePairReports(selectedRelease, state.reports, pair.buildId, pair.controlsId);
              return <li key={`${pair.buildId}-${pair.controlsId}`} className="flex flex-wrap items-center justify-between gap-2 py-2 text-xs">
                <span className="font-medium">{pair.buildTitle} · {pair.controlsTitle}</span>
                <span className="text-[var(--rh-muted)]">
                  {t.creator.ui.reportKinds.structure_integrity}: {t.creator.ui.statuses[summary.structure_integrity.status]} · {t.creator.ui.reportKinds.clean_install}: {t.creator.ui.statuses[summary.clean_install.status]}
                </span>
              </li>;
            })}
          </ul>
        </div>
      )}
    </>}
  </section>;
}
