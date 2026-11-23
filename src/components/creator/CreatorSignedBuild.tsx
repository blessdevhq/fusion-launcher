'use client';

import { open } from '@tauri-apps/plugin-dialog';
import { useRef, useState } from 'react';
import { useI18n } from '@/components/I18nProvider';
import { signedBuildRequest, supportsSignedBuild } from '@/lib/creatorUi';
import { useCreatorStore } from '@/stores/creatorStore';
import type { CreatorProjectOpenResponse } from '@/types/fusionCreator';

const actionClass = 'rh-filter-chip rh-focusable disabled:opacity-40';
const selectClass = 'rh-focusable h-9 min-w-0 max-w-full rounded-md border border-fusion-line bg-fusion-raised px-2 text-sm text-[var(--rh-text)]';

export function CreatorSignedBuild({ opened }: { opened: CreatorProjectOpenResponse }) {
  const { t } = useI18n();
  const text = t.creator.ui.signedBuild;
  const { project } = opened;
  const descriptor = project.program_descriptor;
  const [paths, setPaths] = useState<Record<string, string>>({});
  const [keyDraft, setKeyDraft] = useState<string | null>(null);
  const [buildId, setBuildId] = useState(descriptor.build_profiles[0]?.id ?? '');
  const [controlsDraft, setControls] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<'invalid' | 'failed' | null>(null);
  const locked = useRef(false);
  const stored = opened.privateBindings?.bindings;
  const applicableControls = descriptor.controls_profiles.filter(profile =>
    descriptor.build_profiles.find(build => build.id === buildId)?.controls_profile_ids.includes(profile.id));
  const controlsId = applicableControls.some(profile => profile.id === controlsDraft) ? controlsDraft : applicableControls[0]?.id ?? '';
  const key = keyDraft ?? stored?.signingKeyRef ?? null;
  const effectivePaths = Object.fromEntries(descriptor.components.flatMap(({ id }) => {
    const selected = Object.hasOwn(paths, id) ? paths[id]
      : !opened.missingInputs.includes(id) && stored && Object.hasOwn(stored.componentPaths, id) ? stored.componentPaths[id] : undefined;
    return selected ? [[id, selected]] : [];
  }));
  const building = useCreatorStore(state => Object.values(state.jobs).some(({ receipt, kind, progress }) =>
    kind === 'build' && receipt.projectId === project.project_id && receipt.projectRevision === project.revision
    && (!progress || progress.status === 'queued' || progress.status === 'running' || progress.status === 'cancelling')));

  async function exclusive(work: () => Promise<void>) {
    if (locked.current || building) return;
    locked.current = true;
    setBusy(true);
    setError(null);
    try { await work(); }
    catch { setError('failed'); }
    finally { locked.current = false; setBusy(false); }
  }

  async function pick(componentId: string | null) {
    await exclusive(async () => {
      const selected = await open({ directory: componentId !== null, multiple: false, title: componentId === null ? text.key : text.directory });
      if (typeof selected !== 'string' || useCreatorStore.getState().currentProject !== opened) return;
      if (componentId === null) setKeyDraft(selected);
      else setPaths(previous => ({ ...previous, [componentId]: selected }));
    });
  }

  async function build() {
    const request = signedBuildRequest(project, buildId, controlsId);
    if (!request || !key || descriptor.components.some(({ id }) => !Object.hasOwn(effectivePaths, id))) {
      setError('invalid');
      return;
    }
    await exclusive(async () => {
      const store = useCreatorStore.getState();
      if (store.currentProject !== opened) return;
      const changed = !stored || key !== stored.signingKeyRef || Object.entries(effectivePaths).some(([id, path]) => stored.componentPaths[id] !== path);
      if (changed) await store.savePrivateBindings(opened, { projectId: project.project_id, componentPaths: effectivePaths, signingKeyRef: key });
      const current = useCreatorStore.getState().currentProject;
      if (current?.project !== project || (changed && current.privateBindings?.revision !== (opened.privateBindings ? opened.privateBindings.revision + 1 : 0))) return;
      await store.startJob('build', request);
    });
  }

  return <section className="border-t border-fusion-line py-4" aria-labelledby="creator-signed-build-title" data-testid="creator-signed-build">
    <h2 id="creator-signed-build-title" className="text-lg font-semibold">{text.title}</h2>
    {!supportsSignedBuild(descriptor.components) ? <p className="mt-1 text-sm text-[var(--rh-muted)]">{text.unsupported}</p> : <>
      <p className="mt-1 text-sm text-[var(--rh-muted)]" id="creator-signed-build-note">{text.note}</p>
      <form className="mt-3 space-y-3" aria-labelledby="creator-signed-build-title" aria-describedby="creator-signed-build-note" onSubmit={event => { event.preventDefault(); void build(); }}>
        <ul className="divide-y divide-fusion-line">
          {descriptor.components.map(({ id }) => <li key={id} className="flex flex-wrap items-center justify-between gap-2 py-2">
            <span className="min-w-0 break-all text-sm">{id} <span className="text-xs text-[var(--rh-muted)]">· {Object.hasOwn(effectivePaths, id) ? text.selected : text.missing}</span></span>
            <button type="button" aria-label={`${text.directory}: ${id}`} data-testid={`creator-build-component-${id}`} data-focus-id={`creator:build-component-${id}`} disabled={busy || building} className={actionClass} onClick={() => void pick(id)}>{text.directory}</button>
          </li>)}
          <li className="flex flex-wrap items-center justify-between gap-2 py-2">
            <span className="text-sm text-[var(--rh-muted)]">{key ? text.selected : text.missing}</span>
            <button type="button" data-testid="creator-build-signing-key" data-focus-id="creator:build-signing-key" disabled={busy || building} className={actionClass} onClick={() => void pick(null)}>{text.key}</button>
          </li>
        </ul>
        <div className="flex flex-wrap gap-3">
          <label className="grid min-w-0 gap-1 text-xs text-[var(--rh-muted)]">{text.buildProfile}
            <select name="buildProfile" value={buildId} data-focus-id="creator:build-profile" className={selectClass} disabled={busy || building} onChange={event => { setBuildId(event.target.value); setError(null); }}>
              {descriptor.build_profiles.map(profile => <option key={profile.id} value={profile.id}>{profile.title}</option>)}
            </select>
          </label>
          <label className="grid min-w-0 gap-1 text-xs text-[var(--rh-muted)]">{text.controlsProfile}
            <select name="controlsProfile" value={controlsId} data-focus-id="creator:build-controls" className={selectClass} disabled={busy || building} onChange={event => { setControls(event.target.value); setError(null); }}>
              {applicableControls.map(profile => <option key={profile.id} value={profile.id}>{profile.title}</option>)}
            </select>
          </label>
        </div>
        <button type="submit" data-testid="creator-build-start" data-focus-id="creator:build-start" className={actionClass} disabled={busy || building}>{text.action}</button>
        {busy && <p role="status" className="text-sm text-[var(--rh-muted)]">{t.common.loading}</p>}
        {error && <p role="alert" className="text-sm text-[var(--rh-danger)]">{text[error]}</p>}
      </form>
    </>}
  </section>;
}
