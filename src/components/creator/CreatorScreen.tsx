'use client';

import { useEffect, useState } from 'react';
import { useI18n } from '@/components/I18nProvider';
import { ScreenHeader } from '@/components/dashboard/shared';
import { CreatorDeviceDialog } from './CreatorDeviceDialog';
import { creatorStages, creatorStageHasContent, type CreatorStage } from '@/lib/creatorUi';
import { useCreatorStore } from '@/stores/creatorStore';
import { CreatorStatus } from './CreatorStatus';

export function CreatorScreen({ contextTitle, onFocus }: { contextTitle: string | null; onFocus: (id: string) => void }) {
  const { t, locale } = useI18n();
  const state = useCreatorStore();
  const [stage, setStage] = useState<CreatorStage>('base');
  const [devicesOpen, setDevicesOpen] = useState(false);
  const [jobId, setJobId] = useState('');
  useEffect(() => { void useCreatorStore.getState().loadProjects().catch(() => {}); }, []);
  // Reopen known jobs from durable state before accepting subsequent event updates.
  useEffect(() => {
    const store = useCreatorStore.getState();
    for (const entry of Object.values(store.jobs)) {
      if (store.currentProject?.project.project_id === entry.receipt.projectId && store.currentProject.project.revision === entry.receipt.projectRevision) {
        void store.reopenJob(entry.receipt.jobId).catch(() => {});
      }
    }
  }, []);
  const jobs = Object.values(state.jobs);
  return <section className="rh-screen rh-panel rh-screen-scroll" data-testid="creator-screen" onFocusCapture={(event) => {
    const id = (event.target as HTMLElement).dataset.focusId;
    if (id) onFocus(id);
  }}>
    <ScreenHeader eyebrow={t.creator.ui.title} title={t.creator.ui.title} description={t.creator.ui.description} />
    {contextTitle && <div className="mb-4 border-l-2 border-fusion-line pl-3">
      <p className="text-xs text-[var(--rh-muted)]">{t.creator.ui.context}</p>
      <p data-testid="creator-context-title" className="break-words font-semibold">{contextTitle}</p>
      <p className="mt-1 text-sm text-[var(--rh-muted)]">{t.creator.ui.contextNote}</p>
    </div>}
    <nav aria-label={t.creator.ui.title} className="mb-5 flex shrink-0 flex-wrap gap-2">
      {creatorStages.map((item, index) => <button key={item} type="button" data-testid={`creator-step-${item}`} data-focus-id={`creator:step-${item}`} data-focus-zone="creator-stages" aria-current={stage === item ? 'step' : undefined} onClick={() => setStage(item)} className={`rh-filter-chip rh-focusable ${stage === item ? 'rh-filter-chip-active' : ''}`}>
        <span aria-hidden="true">{index + 1}</span><span>{t.creator.ui.stages[item]}</span>
      </button>)}
    </nav>
    {!creatorStageHasContent(stage) ? <div data-testid="creator-stage-unavailable" data-state="unavailable" role="status" className="border-t border-fusion-line py-5 text-sm text-[var(--rh-muted)]">
      <h2 className="mb-2 text-lg font-semibold text-[var(--rh-text)]">{t.creator.ui.stages[stage]} · {t.creator.common.unavailable}</h2>
      <p>{stage === 'release' ? t.creator.ui.releaseUnavailable : t.creator.ui.descriptorUnavailable}</p>
    </div> : <>
      <section className="border-t border-fusion-line py-4" aria-labelledby="creator-projects-title">
        <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
          <h2 id="creator-projects-title" className="text-lg font-semibold">{t.creator.ui.projects}</h2>
          <div className="flex flex-wrap gap-2">
            <button type="button" data-focus-id="creator:refresh" className="rh-filter-chip rh-focusable" disabled={state.projectsStatus === 'loading'} onClick={() => void state.loadProjects().catch(() => {})}>{t.creator.ui.refresh}</button>
            <button type="button" data-testid="creator-device-open" data-focus-id="creator:device-open" className="rh-filter-chip rh-focusable" onClick={() => setDevicesOpen(true)}>{t.input.open}</button>
          </div>
        </div>
        <CreatorStatus status={state.projectsStatus} error={state.projectsError} testId="creator-projects-state" />
        <div data-testid="creator-project-list" role="list">
          {state.projectsStatus === 'ready' && state.projects.length === 0 && <p className="text-sm text-[var(--rh-muted)]">{t.creator.ui.emptyProjects}</p>}
          {state.projectsStatus === 'ready' && state.projects.map((project) => <div key={project.projectId} role="listitem" className="border-b border-fusion-line py-3">
            <button type="button" data-focus-id={`creator:project-${project.projectId}`} className="rh-focusable w-full rounded-md p-2 text-left" onClick={() => void state.openProject(project.projectId).catch(() => {})}>
              <span className="block break-words font-semibold">{project.title}</span>
              <span className="mt-1 block text-xs text-[var(--rh-muted)]">{t.creator.ui.revision} {project.revision} · {t.creator.ui.statuses[project.verificationStatus]} · {t.creator.ui.modified}: {new Date(project.modifiedAt).toLocaleString(locale)}</span>
            </button>
          </div>)}
        </div>
        <CreatorStatus status={state.projectStatus} error={state.projectError} />
        {state.currentProject && <p className="mt-3 break-words text-sm">{state.currentProject.project.title} · {t.creator.ui.revision} {state.currentProject.project.revision}</p>}
      </section>
      <section className="border-t border-fusion-line py-4" aria-labelledby="creator-jobs-title">
        <h2 id="creator-jobs-title" className="text-lg font-semibold">{t.creator.ui.jobs}</h2>
        <p className="mt-1 text-xs text-[var(--rh-muted)]">{t.creator.ui.historyNote}</p>
        <form className="my-3 flex flex-wrap items-end gap-2" onSubmit={(event) => { event.preventDefault(); if (state.currentProject && jobId.trim()) void state.reopenJob(jobId.trim()).catch(() => {}); }}>
          <label className="grid gap-1 text-xs text-[var(--rh-muted)]">{t.creator.ui.jobId}
            <input data-testid="creator-job-id" value={jobId} onChange={(event) => setJobId(event.target.value)} className="h-9 rounded-md border border-fusion-line bg-fusion-raised px-3 text-sm text-[var(--rh-text)]" />
          </label>
          <button type="submit" data-testid="creator-job-load" data-focus-id="creator:job-load" disabled={!state.currentProject || !jobId.trim()} className="rh-filter-chip rh-focusable disabled:opacity-40">{t.creator.ui.loadJob}</button>
        </form>
        {state.jobError && <CreatorStatus status={state.jobError.code === 'action_unavailable' ? 'unavailable' : 'error'} error={state.jobError} />}
        {jobs.length === 0 && <p data-testid="creator-jobs-state" data-state="idle" className="text-sm text-[var(--rh-muted)]">{t.creator.ui.noJobs}</p>}
        {jobs.map((entry) => <article key={entry.receipt.jobId} className="border-b border-fusion-line py-3" data-testid="creator-job-row">
          <p className="break-all font-mono text-xs">{entry.receipt.jobId} · {t.creator.ui.revision} {entry.receipt.projectRevision}</p>
          <CreatorStatus status={entry.status} error={entry.error} />
          {entry.progress && <>
            <p className="mt-2 text-sm">{t.creator.ui.kinds[entry.progress.kind]} · {t.creator.ui.statuses[entry.progress.status]}</p>
            <p className="mt-1 break-words text-xs text-[var(--rh-muted)]">{t.creator.ui.phase}: {entry.progress.phase} · {t.creator.ui.sequence}: {entry.progress.sequence} · {entry.progress.completedUnits}{entry.progress.totalUnits === null ? '' : ` / ${entry.progress.totalUnits}`}</p>
            {entry.progress.totalUnits !== null && entry.progress.totalUnits > 0 && <progress className="mt-2 w-full accent-fusion-accent" max={entry.progress.totalUnits} value={Math.min(entry.progress.completedUnits, entry.progress.totalUnits)} />}
            {entry.progress.error && <CreatorStatus status={entry.progress.error.code === 'action_unavailable' ? 'unavailable' : 'error'} error={entry.progress.error} />}
            <details className="mt-2 text-xs text-[var(--rh-muted)]"><summary className="rh-focusable cursor-pointer">{t.creator.ui.jobs} ({entry.history.length})</summary>
              <ol className="mt-2 space-y-2">{entry.history.map((event) => <li key={event.sequence} className="break-words">#{event.sequence} · {t.creator.ui.statuses[event.status]} · {event.phase} · {event.completedUnits}{event.totalUnits === null ? '' : ` / ${event.totalUnits}`}</li>)}</ol>
            </details>
          </>}
        </article>)}
      </section>
    </>}
    {devicesOpen && <CreatorDeviceDialog onClose={() => setDevicesOpen(false)} />}
  </section>;
}
