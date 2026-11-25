'use client';

import { open } from '@tauri-apps/plugin-dialog';
import { useRef, useState } from 'react';
import { useI18n } from '@/components/I18nProvider';
import {
  currentRevisionReleases,
  exportReleaseRequest,
  isReleaseExportReady,
  summarizePairReports,
  supportsCleanVerify,
  verifyHttpsRequest,
  type CreatorStage
} from '@/lib/creatorUi';
import { useCreatorStore } from '@/stores/creatorStore';
import { CreatorStatus } from './CreatorStatus';

const actionClass = 'rh-filter-chip rh-focusable disabled:opacity-40';
const selectClass = 'rh-focusable h-9 min-w-0 max-w-full rounded-md border border-fusion-line bg-fusion-raised px-2 text-sm text-[var(--rh-text)]';

export function CreatorReleasePanel({ onSelectStage }: { onSelectStage: (stage: CreatorStage) => void }) {
  const { t } = useI18n();
  const text = t.creator.ui.release;
  const state = useCreatorStore();
  const [targetDirectory, setTargetDirectory] = useState('');
  const [sourceUrl, setSourceUrl] = useState('');
  const [buildIdDraft, setBuildId] = useState('');
  const [controlsDraft, setControlsId] = useState('');
  const [startingExport, setStartingExport] = useState(false);
  const [startingHttps, setStartingHttps] = useState(false);
  const locked = useRef(false);

  const opened = state.currentProject;
  if (!opened) {
    if (state.projectsStatus === 'unavailable' || state.projectStatus === 'unavailable') {
      return <section className="border-t border-fusion-line py-4" aria-labelledby="creator-release-title">
        <h2 id="creator-release-title" className="mb-2 text-lg font-semibold">{text.title}</h2>
        <CreatorStatus status="unavailable" error={state.projectError ?? state.projectsError} testId="creator-release-unavailable" />
      </section>;
    }
    return <section className="border-t border-fusion-line py-4" aria-labelledby="creator-release-title" data-testid="creator-release-no-project">
      <h2 id="creator-release-title" className="text-lg font-semibold">{text.title}</h2>
      <p className="mt-1 text-sm text-[var(--rh-muted)]">{text.noProject}</p>
      <button type="button" data-focus-id="creator:release-go-base" className={`${actionClass} mt-3`} onClick={() => onSelectStage('base')}>{t.creator.ui.verify.goToBuild}</button>
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

  const projectJobs = Object.values(state.jobs).filter(({ receipt }) =>
    receipt.projectId === project.project_id && receipt.projectRevision === project.revision);
  const writerBusy = projectJobs.some(({ kind, progress }) =>
    (kind === 'build' || kind === 'verify' || kind === 'export' || kind === 'verify_https')
    && (!progress || progress.status === 'queued' || progress.status === 'running' || progress.status === 'cancelling'));

  const matchingJobs = (jobKind: 'export' | 'verify_https') => projectJobs.filter(entry =>
    entry.kind === jobKind
    && (!selectedRelease || !entry.progress?.releaseId || entry.progress.releaseId === selectedRelease.releaseId));
  const exportJobs = matchingJobs('export');
  const httpsJobs = matchingJobs('verify_https');
  const activeExportJob = (state.selectedJobId ? exportJobs.find(entry => entry.receipt.jobId === state.selectedJobId) : undefined)
    ?? exportJobs[exportJobs.length - 1]
    ?? null;
  const activeHttpsJob = (state.selectedJobId ? httpsJobs.find(entry => entry.receipt.jobId === state.selectedJobId) : undefined)
    ?? httpsJobs[httpsJobs.length - 1]
    ?? null;

  const exportReadiness = isReleaseExportReady(project, selectedRelease, state.reports);
  const exportReq = selectedRelease && exportReadiness.ready
    ? exportReleaseRequest(project, selectedRelease.releaseId, targetDirectory)
    : null;
  const httpsReq = selectedRelease && supportsCleanVerify(descriptor)
    ? verifyHttpsRequest(project, selectedRelease.releaseId, sourceUrl, buildId, controlsId)
    : null;
  const pairSummary = selectedRelease
    ? summarizePairReports(selectedRelease, state.reports, buildId, controlsId)
    : null;

  async function pickDirectory() {
    if (locked.current || writerBusy) return;
    locked.current = true;
    try {
      const selected = await open({ directory: true, multiple: false, title: text.targetDirectory });
      if (typeof selected === 'string' && useCreatorStore.getState().currentProject === opened) {
        setTargetDirectory(selected);
      }
    } finally {
      locked.current = false;
    }
  }

  async function startJob(kind: 'export' | 'verify_https') {
    const request = kind === 'export' ? exportReq : httpsReq;
    if (!request || locked.current || writerBusy || startingExport || startingHttps) return;
    locked.current = true;
    if (kind === 'export') setStartingExport(true);
    else setStartingHttps(true);
    try {
      await useCreatorStore.getState().startJob(kind, request);
    } catch {
      // Store retains jobError scoped to active project revision.
    } finally {
      locked.current = false;
      setStartingExport(false);
      setStartingHttps(false);
    }
  }

  const exportProgress = activeExportJob?.progress ?? null;
  const exportError = exportProgress?.error
    ?? activeExportJob?.error
    ?? (activeExportJob && state.selectedJobId === activeExportJob.receipt.jobId ? state.jobError : null);

  const httpsProgress = activeHttpsJob?.progress ?? null;
  const httpsError = httpsProgress?.error
    ?? activeHttpsJob?.error
    ?? (activeHttpsJob && state.selectedJobId === activeHttpsJob.receipt.jobId ? state.jobError : null);
  const httpsReport = pairSummary?.https_acquisition.report ?? null;
  const httpsStatus = pairSummary?.https_acquisition.status ?? 'not_tested';

  return <section className="border-t border-fusion-line py-4" aria-labelledby="creator-release-title" data-testid="creator-release-panel">
    <div className="flex flex-wrap items-center justify-between gap-2">
      <h2 id="creator-release-title" className="text-lg font-semibold">{text.title}</h2>
      <button type="button" className={actionClass} data-focus-id="creator:release-refresh" onClick={() => void state.loadReleases(project.project_id).catch(() => {})}>{t.creator.ui.refresh}</button>
    </div>
    <p className="mt-1 text-sm text-[var(--rh-muted)]">{text.note}</p>
    <p className="mt-1 text-xs text-[var(--rh-muted)]">{text.legacyNote}</p>

    {!selectedRelease ? (
      <div className="mt-3 space-y-2 text-sm text-[var(--rh-muted)]">
        <p>{text.noRelease}</p>
        <button type="button" data-focus-id="creator:release-go-verify" className={actionClass} onClick={() => onSelectStage('verification')}>{text.goToVerify}</button>
      </div>
    ) : <>
      <div className="mt-4 flex flex-wrap gap-3">
        <label className="grid min-w-0 gap-1 text-xs text-[var(--rh-muted)]">{text.release}
          <select name="releaseSelect" data-testid="creator-release-select" data-focus-id="creator:release-select" className={selectClass} disabled={writerBusy} value={selectedRelease.releaseId} onChange={event => state.selectRelease(event.target.value)}>
            {revReleases.map(release => <option key={release.releaseId} value={release.releaseId}>{release.releaseId}</option>)}
          </select>
        </label>
      </div>
      <p data-testid="creator-release-hash" className="mt-2 select-text break-all font-mono text-xs text-[var(--rh-muted)]">
        {t.creator.ui.verify.manifestHash}: {selectedRelease.manifestBlake3}
      </p>

      <div data-testid="creator-release-gate" className="mt-4 border-t border-fusion-line pt-3">
        <h3 className="text-sm font-semibold">{text.exportSection}</h3>
        <p className="mt-1 text-xs text-[var(--rh-muted)]" data-state={exportReadiness.ready ? 'ready' : 'blocked'}>
          {exportReadiness.ready
            ? text.gateReady
            : `${text.gateBlocked} ${exportReadiness.missingPairs.map(pair => `${pair.buildTitle} / ${pair.controlsTitle}`).join(', ')}`}
        </p>
        {!exportReadiness.ready && (
          <button type="button" data-focus-id="creator:release-go-verify" className={`${actionClass} mt-2`} onClick={() => onSelectStage('verification')}>{text.goToVerify}</button>
        )}

        <div className="mt-3 flex flex-wrap items-center gap-2">
          <button type="button" data-testid="creator-export-dir" data-focus-id="creator:export-dir" className={actionClass} disabled={writerBusy} onClick={() => void pickDirectory()}>{text.pickDirectory}</button>
          <input
            type="text"
            data-testid="creator-export-path"
            data-focus-id="creator:export-path"
            aria-label={text.targetDirectory}
            value={targetDirectory}
            placeholder={text.missingDirectory}
            disabled={writerBusy}
            onChange={event => setTargetDirectory(event.target.value)}
            className="rh-focusable h-9 min-w-0 flex-1 rounded-md border border-fusion-line bg-fusion-raised px-3 text-sm text-[var(--rh-text)]"
          />
          <span className="min-w-0 break-all text-xs text-[var(--rh-muted)]">
            {targetDirectory ? `${text.selected}: ${targetDirectory}` : text.missingDirectory}
          </span>
        </div>

        <div className="mt-3 flex flex-wrap items-center gap-2">
          <button type="button" data-testid="creator-export-start" data-focus-id="creator:export-start" className={actionClass} disabled={!exportReq || writerBusy || startingExport} onClick={() => void startJob('export')}>{text.exportAction}</button>
          {activeExportJob?.acceptingEvents && (exportProgress?.status === 'queued' || exportProgress?.status === 'running') && (
            <button type="button" data-testid="creator-export-cancel" data-focus-id="creator:export-cancel" className={actionClass} onClick={() => void state.cancelJob(activeExportJob.receipt.jobId).catch(() => {})}>{t.creator.ui.cancel}</button>
          )}
        </div>

        {exportError && (
          <p data-testid="creator-export-error" role="alert" className="mt-2 break-words font-mono text-xs text-[var(--rh-danger)]">
            {exportError.code} · {exportError.message}
          </p>
        )}

        {activeExportJob && exportProgress && (
          <div data-testid="creator-export-job" className="mt-3 text-xs">
            <span className="font-medium">{text.exportJob} · <span className="font-mono">{activeExportJob.receipt.jobId}</span></span>{' · '}
            <span data-testid="creator-export-job-status" data-status={exportProgress.status}>{t.creator.ui.statuses[exportProgress.status]}</span>
            <p className="mt-1 text-[var(--rh-muted)]">{t.creator.ui.phase}: {exportProgress.phase} · {t.creator.ui.sequence}: {exportProgress.sequence}</p>
          </div>
        )}
      </div>

      <div className="mt-4 border-t border-fusion-line pt-3">
        <h3 className="text-sm font-semibold">{text.httpsSection}</h3>
        <div className="mt-2 grid gap-3">
          <label className="grid min-w-0 gap-1 text-xs text-[var(--rh-muted)]">{text.httpsUrl}
            <input type="url" data-testid="creator-https-url" data-focus-id="creator:https-url" value={sourceUrl} placeholder={text.httpsPlaceholder} disabled={writerBusy || startingHttps} onChange={event => setSourceUrl(event.target.value)} className="rh-focusable h-9 rounded-md border border-fusion-line bg-fusion-raised px-3 text-sm text-[var(--rh-text)]" />
          </label>
          <div className="flex flex-wrap gap-3">
            <label className="grid min-w-0 gap-1 text-xs text-[var(--rh-muted)]">{t.creator.ui.verify.buildProfile}
              <select data-testid="creator-https-build-profile" data-focus-id="creator:https-build-profile" className={selectClass} disabled={writerBusy || startingHttps} value={buildId} onChange={event => setBuildId(event.target.value)}>
                {descriptor.build_profiles.map(profile => <option key={profile.id} value={profile.id}>{profile.title}</option>)}
              </select>
            </label>
            <label className="grid min-w-0 gap-1 text-xs text-[var(--rh-muted)]">{t.creator.ui.verify.controlsProfile}
              <select data-testid="creator-https-controls-profile" data-focus-id="creator:https-controls-profile" className={selectClass} disabled={writerBusy || startingHttps} value={controlsId} onChange={event => setControlsId(event.target.value)}>
                {applicableControls.map(profile => <option key={profile.id} value={profile.id}>{profile.title}</option>)}
              </select>
            </label>
          </div>
        </div>

        <div className="mt-3 flex flex-wrap items-center gap-2">
          <button type="button" data-testid="creator-https-start" data-focus-id="creator:https-start" className={actionClass} disabled={!httpsReq || writerBusy || startingHttps} onClick={() => void startJob('verify_https')}>
            {httpsProgress?.status === 'failed' || httpsProgress?.status === 'cancelled' ? text.httpsRetry : text.httpsAction}
          </button>
          {activeHttpsJob?.acceptingEvents && (httpsProgress?.status === 'queued' || httpsProgress?.status === 'running') && (
            <button type="button" data-testid="creator-https-cancel" data-focus-id="creator:https-cancel" className={actionClass} onClick={() => void state.cancelJob(activeHttpsJob.receipt.jobId).catch(() => {})}>{t.creator.ui.cancel}</button>
          )}
        </div>

        {httpsError && (
          <p data-testid="creator-https-error" role="alert" className="mt-2 break-words font-mono text-xs text-[var(--rh-danger)]">
            {httpsError.code} · {httpsError.message}
          </p>
        )}

        {activeHttpsJob && httpsProgress && (
          <div data-testid="creator-https-job" className="mt-3 text-xs">
            <span className="font-medium">{text.httpsJob} · <span className="font-mono">{activeHttpsJob.receipt.jobId}</span></span>{' · '}
            <span data-testid="creator-https-job-status" data-status={httpsProgress.status}>{t.creator.ui.statuses[httpsProgress.status]}</span>
            <p className="mt-1 text-[var(--rh-muted)]">{t.creator.ui.phase}: {httpsProgress.phase} · {t.creator.ui.sequence}: {httpsProgress.sequence}</p>
          </div>
        )}

        <div data-testid="creator-https-result" data-status={httpsStatus} className="mt-3 border-t border-fusion-line pt-2 text-sm">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span>{text.httpsResult}</span>
            <span className="text-xs text-[var(--rh-muted)]">{t.creator.ui.statuses[httpsStatus]}</span>
          </div>
          {httpsReport && (
            <p className="mt-1 break-all font-mono text-xs text-[var(--rh-muted)]">
              {t.creator.ui.verify.reportId}: {httpsReport.reportId}{typeof httpsReport.environment.source_url === 'string' ? ` · ${httpsReport.environment.source_url}` : ''}
            </p>
          )}
          {httpsReport && httpsReport.diagnostics.length > 0 && (
            <p className="mt-1 break-words text-xs text-[var(--rh-danger)]">
              {httpsReport.diagnostics.map(diag => `${diag.code}: ${diag.message}`).join(' · ')}
            </p>
          )}
        </div>
      </div>
    </>}
  </section>;
}
