import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { createCreatorStore } from './creatorStore.ts';
import type { CreatorApi, FusionInputApi, CreatorJob, CreatorJobProgress, CreatorProjectOpenResponse, CreatorRelease, CreatorReport, InputReleaseReceipt, InputState } from '../types/fusionCreator.ts';

const fixtures = JSON.parse(readFileSync(new URL('../../fixtures/creator/contracts.json', import.meta.url), 'utf8'));
const opened: CreatorProjectOpenResponse = { project: fixtures.project, privateBindings: fixtures.projectBindings, missingInputs: fixtures.projectMissingInputs };
const progress = (overrides: Partial<CreatorJobProgress> = {}): CreatorJobProgress => ({ ...fixtures.job, ...overrides });
const job = (overrides: Partial<CreatorJobProgress> = {}): CreatorJob => ({ progress: progress(overrides), checkpoints: [], reportIds: [] });
const releaseFixture = (overrides: Partial<CreatorRelease> = {}): CreatorRelease => ({
  releaseId: 'release-1', projectId: opened.project.project_id, projectRevision: 1,
  manifestBlake3: 'b'.repeat(64), artifactHashes: { program: 'c'.repeat(64) }, reportIds: [], ...overrides
});
const reportFixture = (overrides: Partial<CreatorReport> = {}): CreatorReport => ({ ...fixtures.report, ...overrides });

const unavailable = { code: 'action_unavailable', message: 'Not implemented', details: { command: 'fusion_creator_project_list' } };
function apiWith(overrides: Partial<CreatorApi> = {}): CreatorApi {
  return new Proxy(overrides as CreatorApi, { get: (target, key) => target[key as keyof CreatorApi] ?? (async () => { throw unavailable; }) });
}
const inputApi = new Proxy({} as FusionInputApi, { get: () => async () => { throw unavailable; } });
function inputWith(overrides: Partial<FusionInputApi>): FusionInputApi {
  return new Proxy(overrides as FusionInputApi, { get: (target, key) => target[key as keyof FusionInputApi] ?? (async () => { throw unavailable; }) });
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test('valid job identifiers are data, not inherited object properties', async () => {
  const store = createCreatorStore({ creatorApi: apiWith({
    projectOpen: async () => opened,
    jobGet: async ({ jobId }) => job({ jobId })
  }) });
  await store.getState().openProject(opened.project.project_id);
  for (const jobId of ['__proto__', 'constructor', 'toString']) {
    await store.getState().reopenJob(jobId);
    assert.equal(store.getState().mergeJobProgress(progress({ jobId, sequence: 2 })), true);
    assert.deepEqual(store.getState().jobs[jobId].history.map((event) => event.sequence), [1, 2]);
  }
  assert.equal(Object.getPrototypeOf(store.getState().jobs), Object.prototype);
});

test('projects distinguish loading, a genuine ready empty list, unavailable and error', async () => {
  const pending = deferred<[]>();
  let response = () => pending.promise;
  const store = createCreatorStore({ creatorApi: apiWith({ projectList: () => response() }), fusionInputApi: inputApi });
  assert.equal(store.getState().projectsStatus, 'idle');
  const loading = store.getState().loadProjects();
  assert.equal(store.getState().projectsStatus, 'loading');
  pending.resolve([]);
  await loading;
  assert.equal(store.getState().projectsStatus, 'ready');
  assert.deepEqual(store.getState().projects, []);
  response = async () => { throw unavailable; };
  await assert.rejects(store.getState().loadProjects(), { code: 'action_unavailable' });
  assert.equal(store.getState().projectsStatus, 'unavailable');
  assert.deepEqual(store.getState().projectsError, unavailable);
  response = async () => { throw new Error('offline'); };
  await assert.rejects(store.getState().loadProjects());
  assert.equal(store.getState().projectsStatus, 'error');
});

test('open uses the exact 1.1 envelope and older responses cannot replace a newer selection', async () => {
  const old = deferred<CreatorProjectOpenResponse>();
  const store = createCreatorStore({ creatorApi: apiWith({ projectOpen: ({ projectId }) => projectId === 'old' ? old.promise : Promise.resolve(opened) }) });
  const first = store.getState().openProject('old');
  await store.getState().openProject(opened.project.project_id);
  assert.deepEqual(store.getState().currentProject, opened);
  old.resolve({ ...opened, project: { ...opened.project, project_id: 'old' }, privateBindings: null });
  await first;
  assert.equal(store.getState().currentProject?.project.project_id, opened.project.project_id);
});

test('private binding saves use independent CAS and ignore a save for a superseded project selection', async () => {
  const pending = deferred<{ revision: number }>();
  const bindings = { projectId: opened.project.project_id, componentPaths: {}, signingKeyRef: 'C:/fixture/signing-key' };
  const store = createCreatorStore({ creatorApi: apiWith({
    projectOpen: async ({ projectId }) => projectId === opened.project.project_id ? opened : {
      ...opened, project: { ...opened.project, project_id: projectId }, privateBindings: null
    },
    privateBindingsSave: async (request) => {
      assert.deepEqual(request, { projectRevision: 1, expectedBindingsRevision: 3, bindings });
      return pending.promise;
    }
  }) });
  const project = await store.getState().openProject(bindings.projectId);
  const saving = store.getState().savePrivateBindings(project, bindings);
  await store.getState().openProject('other');
  pending.resolve({ revision: 4 });
  await saving;
  assert.equal(store.getState().currentProject?.privateBindings, null);
  const reopened = await store.getState().openProject(bindings.projectId);
  await store.getState().savePrivateBindings(reopened, bindings);
  assert.deepEqual(store.getState().currentProject?.project, opened.project);
  assert.deepEqual(store.getState().currentProject?.privateBindings, { revision: 4, bindings });
});

test('signed build reads its durable snapshot and cancel does not overwrite a newer terminal event', async () => {
  const cancellation = deferred<CreatorJob>();
  const receipt = { jobId: 'job-1', projectId: opened.project.project_id, projectRevision: 1 };
  const request = { ...fixtures.jobStart, releaseId: null, targetDirectory: 'C:/fixture/output' };
  const store = createCreatorStore({ creatorApi: apiWith({
    projectOpen: async () => opened,
    build: async (value) => { assert.deepEqual(value, request); return receipt; },
    jobGet: async ({ jobId }) => { assert.equal(jobId, receipt.jobId); return job({ kind: 'build', sequence: 2, status: 'running' }); },
    jobCancel: async ({ jobId }) => { assert.equal(jobId, receipt.jobId); return cancellation.promise; }
  }) });
  await store.getState().openProject(receipt.projectId);
  await store.getState().startJob('build', request);
  assert.equal(store.getState().jobs[receipt.jobId].progress?.sequence, 2);
  const cancelling = store.getState().cancelJob(receipt.jobId);
  assert.equal(store.getState().jobs[receipt.jobId].progress?.status, 'running');
  assert.equal(store.getState().mergeJobProgress(progress({ kind: 'build', sequence: 4, status: 'completed' })), true);
  cancellation.resolve(job({ kind: 'build', sequence: 3, status: 'cancelling' }));
  await cancelling;
  assert.equal(store.getState().jobs[receipt.jobId].progress?.status, 'completed');
});

test('job receipts establish ownership; events reject unknown IDs, revision mismatches and nonmonotonic sequences', async () => {
  const store = createCreatorStore({ creatorApi: apiWith({
    projectOpen: async () => opened,
    scan: async () => ({ jobId: 'job-1', projectId: opened.project.project_id, projectRevision: 1 })
  }) });
  await store.getState().openProject(opened.project.project_id);
  assert.equal(store.getState().mergeJobProgress(progress()), false);
  await store.getState().startJob('scan', fixtures.jobStart);
  assert.equal(store.getState().mergeJobProgress(progress({ jobId: 'unknown' })), false);
  assert.equal(store.getState().mergeJobProgress(progress({ projectRevision: 2 })), false);
  assert.equal(store.getState().mergeJobProgress(progress({ projectId: 'other' })), false);
  assert.equal(store.getState().mergeJobProgress(progress()), true);
  for (const sequence of [1, 0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
    assert.equal(store.getState().mergeJobProgress(progress({ sequence })), false);
  }
  assert.equal(store.getState().mergeJobProgress(progress({ sequence: 2, status: 'completed' })), true);
  assert.equal(store.getState().mergeJobProgress(progress({ sequence: 3, status: 'running' })), false);
  assert.deepEqual(store.getState().jobs['job-1'].history.map((item) => item.sequence), [1, 2]);
});

test('late start receipts cannot select jobs after any project selection generation change', async () => {
  for (const change of ['other', 'revision', 'same'] as const) {
    const pending = deferred<{ jobId: string; projectId: string; projectRevision: number }>();
    let response = opened;
    const store = createCreatorStore({ creatorApi: apiWith({ projectOpen: async () => response, scan: () => pending.promise }) });
    await store.getState().openProject(opened.project.project_id);
    const starting = store.getState().startJob('scan', fixtures.jobStart);
    response = { ...opened, privateBindings: null, project: { ...opened.project,
      project_id: change === 'other' ? 'other' : opened.project.project_id,
      revision: change === 'revision' ? 2 : opened.project.revision } };
    await store.getState().openProject(response.project.project_id);
    const receipt = { jobId: 'job-1', projectId: opened.project.project_id, projectRevision: 1 };
    pending.resolve(receipt);
    assert.deepEqual(await starting, receipt); // Backend creation is real; no invented cancellation.
    assert.equal(store.getState().selectedJobId, null, change);
    assert.equal(store.getState().jobs['job-1']?.acceptingEvents ?? false, false, change);
    assert.equal(store.getState().mergeJobProgress(progress()), false, change);
  }
});

test('reopening gates events until jobGet, preserving snapshot history and refusing identity spoofing', async () => {
  const pending = deferred<CreatorJob>();
  const store = createCreatorStore({ creatorApi: apiWith({ projectOpen: async () => opened, jobGet: () => pending.promise }) });
  await store.getState().openProject(opened.project.project_id);
  const loading = store.getState().reopenJob('job-1');
  assert.equal(store.getState().mergeJobProgress(progress({ sequence: 4 })), false);
  pending.resolve(job({ sequence: 5 }));
  await loading;
  assert.equal(store.getState().mergeJobProgress(progress({ sequence: 4 })), false);
  assert.equal(store.getState().mergeJobProgress(progress({ sequence: 6 })), true);
  assert.deepEqual(store.getState().jobs['job-1'].history.map((item) => item.sequence), [5, 6]);
  const invalid = createCreatorStore({ creatorApi: apiWith({ projectOpen: async () => opened, jobGet: async () => job({ jobId: 'wrong' }) }) });
  await invalid.getState().openProject(opened.project.project_id);
  await assert.rejects(invalid.getState().reopenJob('job-1'), { code: 'invalid_response' });
  assert.equal(invalid.getState().mergeJobProgress(progress()), false);
});

test('jobGet replays matching buffered events after its snapshot without terminal regression', async () => {
  const pending = deferred<CreatorJob>();
  const store = createCreatorStore({ creatorApi: apiWith({ projectOpen: async () => opened, jobGet: () => pending.promise }) });
  await store.getState().openProject(opened.project.project_id);
  const loading = store.getState().reopenJob('job-1');
  for (const event of [progress({ sequence: 4 }), progress({ sequence: 6 }),
    progress({ sequence: 7, kind: 'build' }), progress({ sequence: 7, status: 'completed' }), progress({ sequence: 8 }),
    progress({ sequence: 9, projectRevision: 2 })]) store.getState().mergeJobProgress(event);
  assert.equal(store.getState().jobs['job-1'].progress, null);
  pending.resolve(job({ sequence: 5 }));
  await loading;
  assert.deepEqual(store.getState().jobs['job-1'].history.map((event) => event.sequence), [5, 6, 7]);
  assert.equal(store.getState().jobs['job-1'].job?.progress.status, 'completed');
});

test('reopening with an older job snapshot preserves cached report metadata and progress', async () => {
  let response = { ...job({ sequence: 5 }), reportIds: ['report-1'] };
  const store = createCreatorStore({ creatorApi: apiWith({ projectOpen: async () => opened, jobGet: async () => response }) });
  await store.getState().openProject(opened.project.project_id);
  await store.getState().reopenJob('job-1');
  response = job({ sequence: 4 });
  await store.getState().reopenJob('job-1');
  assert.deepEqual(store.getState().jobs['job-1'].job?.reportIds, ['report-1']);
  assert.equal(store.getState().jobs['job-1'].job?.progress.sequence, 5);
});

test('input devices expose unavailable rather than a successful fake empty list', async () => {
  const store = createCreatorStore({ fusionInputApi: inputApi });
  await assert.rejects(store.getState().loadInputDevices(), { code: 'action_unavailable' });
  assert.equal(store.getState().inputDevicesStatus, 'unavailable');
  const ready = createCreatorStore({ fusionInputApi: inputWith({ devices: async () => [] }) });
  await ready.getState().loadInputDevices();
  assert.equal(ready.getState().inputDevicesStatus, 'ready');
});

test('input device responses require valid fields and unique device identities before ready', async () => {
  const device = { deviceId: 'device-1', name: 'Controller', connected: true, mappingKnown: false };
  for (const malformed of [null, {}, [null], [{ ...device, deviceId: '' }],
    [{ ...device, name: null }], [{ ...device, connected: 1 }],
    [{ ...device, mappingKnown: 'yes' }], [{ deviceId: 'device-1' }], [device, device]]) {
    const store = createCreatorStore({ fusionInputApi: inputWith({ devices: async () => malformed as never }) });
    await assert.rejects(store.getState().loadInputDevices(), { code: 'invalid_response' });
    assert.equal(store.getState().inputDevicesStatus, 'error');
    assert.deepEqual(store.getState().inputDevices, []);
  }
  const store = createCreatorStore({ fusionInputApi: inputWith({ devices: async () => [device] }) });
  await store.getState().loadInputDevices();
  assert.equal(store.getState().inputDevicesStatus, 'ready');
  assert.deepEqual(store.getState().inputDevices, [device]);
});

test('input ownership is exclusive, snapshots precede events, and terminal events never prove release', async () => {
  const snapshot = deferred<InputState>();
  let starts = 0;
  const store = createCreatorStore({ fusionInputApi: inputWith({
    sessionStart: async () => { starts++; return { sessionId: 'session-1' }; },
    sessionGet: () => snapshot.promise
  }) });
  const start = store.getState().startInputSession('device-1');
  await assert.rejects(store.getState().startInputSession('device-2'));
  await Promise.resolve();
  assert.equal(starts, 1);
  assert.equal(store.getState().mergeInputState(fixtures.input), false);
  snapshot.resolve(fixtures.input);
  await start;
  assert.equal(store.getState().inputOwnership, 'owned');
  for (const update of [
    { ...fixtures.input, sessionId: 'unknown', sequence: 2 },
    { ...fixtures.input, deviceId: 'device-2', sequence: 2 },
    { ...fixtures.input, sequence: 1 },
    { ...fixtures.input, sequence: 1.5 }
  ]) assert.equal(store.getState().mergeInputState(update), false);
  assert.equal(store.getState().mergeInputState({ ...fixtures.input, sequence: 2, status: 'stopping' }), true);
  assert.equal(store.getState().mergeInputState({ ...fixtures.input, sequence: 3, status: 'active' }), false);
  assert.equal(store.getState().mergeInputState({ ...fixtures.input, sequence: 4, status: 'stopped' }), true);
  assert.equal(store.getState().mergeInputState({ ...fixtures.input, sequence: 5, status: 'active' }), false);
  assert.equal(store.getState().inputSessionId, 'session-1');
  assert.equal(store.getState().inputHandlesClosed, false);
  await assert.rejects(store.getState().startInputSession('device-1'));
});

test('sessionGet replays buffered input only after validation and obeys terminal ordering', async () => {
  const pending = deferred<InputState>();
  const store = createCreatorStore({ fusionInputApi: inputWith({
    sessionStart: async () => ({ sessionId: 'session-1' }), sessionGet: () => pending.promise
  }) });
  const loading = store.getState().startInputSession('device-1');
  await Promise.resolve();
  for (const event of [
    { ...fixtures.input, sequence: 4 }, { ...fixtures.input, sequence: 6 },
    { ...fixtures.input, sequence: 7, status: 'stopped' },
    { ...fixtures.input, sequence: 8 },
    { ...fixtures.input, sequence: 9, sessionId: 'other' },
    { ...fixtures.input, sequence: 10, deviceId: 'other' }
  ]) store.getState().mergeInputState(event);
  assert.equal(store.getState().inputState, null);
  pending.resolve({ ...fixtures.input, sequence: 5 });
  await loading;
  assert.deepEqual(store.getState().inputHistory.map((event) => event.sequence), [5, 6, 7]);
  assert.equal(store.getState().inputState?.status, 'stopped');
  assert.equal(store.getState().inputHandlesClosed, false);
});

test('a buffered input status older than sessionGet cannot suppress a newer valid event', async () => {
  const pending = deferred<InputState>();
  const store = createCreatorStore({ fusionInputApi: inputWith({
    sessionStart: async () => ({ sessionId: 'session-1' }), sessionGet: () => pending.promise
  }) });
  const loading = store.getState().startInputSession('device-1');
  await Promise.resolve();
  store.getState().mergeInputState({ ...fixtures.input, sequence: 4, status: 'stopped' });
  store.getState().mergeInputState({ ...fixtures.input, sequence: 6 });
  pending.resolve({ ...fixtures.input, sequence: 5 });
  await loading;
  assert.deepEqual(store.getState().inputHistory.map(({ sequence }) => sequence), [5, 6]);
  assert.equal(store.getState().inputHandlesClosed, false);
});

test('stop never releases optimistically and requires matching handlesClosed true ACK', async () => {
  const ack = deferred<InputReleaseReceipt>();
  const store = createCreatorStore({ fusionInputApi: inputWith({
    sessionStart: async () => ({ sessionId: 'session-1' }), sessionGet: async () => fixtures.input,
    sessionStop: () => ack.promise
  }) });
  await store.getState().startInputSession('device-1');
  const stop = store.getState().stopInputSession();
  assert.equal(store.getState().inputOwnership, 'releasing');
  assert.equal(store.getState().inputSessionId, 'session-1');
  assert.equal(store.getState().inputHandlesClosed, false);
  ack.resolve({ sessionId: 'session-1', handlesClosed: false } as unknown as InputReleaseReceipt);
  await assert.rejects(stop, { code: 'invalid_response' });
  assert.equal(store.getState().inputOwnership, 'uncertain');
  assert.equal(store.getState().inputSessionId, 'session-1');
  assert.equal(store.getState().inputHandlesClosed, false);
});

test('release timeout or wrong-session ACK blocks ownership transfer; only a genuine ACK releases', async () => {
  let release: FusionInputApi['releaseForLaunch'] = async () => { throw new Error('ACK timeout'); };
  const store = createCreatorStore({ fusionInputApi: inputWith({
    sessionStart: async () => ({ sessionId: 'session-1' }), sessionGet: async () => fixtures.input,
    releaseForLaunch: (request) => release(request)
  }) });
  await store.getState().startInputSession('device-1');
  await assert.rejects(store.getState().releaseInputForLaunch());
  assert.equal(store.getState().inputHandlesClosed, false);
  assert.equal(store.getState().inputSessionId, 'session-1');
  release = async () => ({ sessionId: 'other', handlesClosed: true });
  await assert.rejects(store.getState().releaseInputForLaunch(), { code: 'invalid_response' });
  assert.equal(store.getState().inputOwnership, 'uncertain');
  release = async () => ({ sessionId: 'session-1', handlesClosed: true });
  assert.deepEqual(await store.getState().releaseInputForLaunch(), { sessionId: 'session-1', handlesClosed: true });
  assert.equal(store.getState().inputHandlesClosed, true);
  assert.equal(store.getState().inputSessionId, null);
  assert.equal(store.getState().inputOwnership, 'released');
  assert.equal(store.getState().mergeInputState({ ...fixtures.input, sequence: 20 }), false);
});

test('failed input initialization remains fail closed even without a session ID', async () => {
  const store = createCreatorStore({ fusionInputApi: inputApi });
  await assert.rejects(store.getState().startInputSession('device-1'));
  assert.equal(store.getState().inputOwnership, 'uncertain');
  assert.equal(store.getState().inputHandlesClosed, false);
  assert.equal(store.getState().inputStatus, 'unavailable');
  await assert.rejects(store.getState().releaseInputForLaunch());
  await assert.rejects(store.getState().startInputSession('device-1'));
});

test('opening a project again reloads cached jobs without losing the terminal state', async () => {
  const pending = deferred<CreatorJob>();
  let reads = 0;
  const store = createCreatorStore({ creatorApi: apiWith({ projectOpen: async () => opened,
    jobGet: async () => ++reads === 1 ? job() : pending.promise }) });
  await store.getState().openProject(opened.project.project_id);
  await store.getState().reopenJob('job-1');
  await store.getState().openProject(opened.project.project_id);
  assert.equal(reads, 2);
  assert.equal(store.getState().mergeJobProgress(progress({ sequence: 3, status: 'completed' })), false);
  pending.resolve(job({ sequence: 2 }));
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(store.getState().jobs['job-1'].progress?.status, 'completed');
  assert.equal(store.getState().selectedJobId, null);
});

test('late receipt survives leaving a project and is loaded when that revision is opened again', async () => {
  const pending = deferred<typeof fixtures.jobReceipt>();
  const projectId = opened.project.project_id;
  const store = createCreatorStore({ creatorApi: apiWith({
    projectOpen: async ({ projectId: id }) => id === projectId ? opened : {
      ...opened, privateBindings: null, project: { ...opened.project, project_id: id }
    },
    build: () => pending.promise,
    jobGet: async () => job({ kind: 'build', sequence: 2, status: 'completed' })
  }) });
  await store.getState().openProject(projectId);
  const starting = store.getState().startJob('build', fixtures.jobStart);
  await store.getState().openProject('other');
  pending.resolve({ jobId: 'job-1', projectId, projectRevision: 1 });
  await starting;
  assert.equal(store.getState().jobs['job-1']?.acceptingEvents, false);
  assert.equal(store.getState().selectedJobId, null);
  await store.getState().openProject(projectId);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(store.getState().jobs['job-1'].progress?.status, 'completed');
});

test('binding save refreshes missing inputs from an authoritative matching open response', async () => {
  const bindings = { projectId: opened.project.project_id, componentPaths: { [opened.missingInputs[0]]: 'C:/fixture/program' }, signingKeyRef: null };
  let saved = false;
  const store = createCreatorStore({ creatorApi: apiWith({
    projectOpen: async () => saved ? { ...opened, privateBindings: { revision: 4, bindings }, missingInputs: [] } : opened,
    privateBindingsSave: async () => { saved = true; return { revision: 4 }; }
  }) });
  const initial = await store.getState().openProject(bindings.projectId);
  assert.notEqual(initial.missingInputs.length, 0);
  await store.getState().savePrivateBindings(initial, bindings);
  assert.deepEqual(store.getState().currentProject?.missingInputs, []);
  assert.equal(store.getState().currentProject?.project, initial.project);
});

test('a started job rejects a different kind even for its first event', async () => {
  const store = createCreatorStore({ creatorApi: apiWith({ projectOpen: async () => opened,
    scan: async () => ({ jobId: 'job-1', projectId: opened.project.project_id, projectRevision: 1 }) }) });
  await store.getState().openProject(opened.project.project_id);
  await store.getState().startJob('scan', fixtures.jobStart);
  assert.equal(store.getState().mergeJobProgress(progress({ kind: 'build' })), false);
});

test('a job snapshot cannot replace the kind established by its start receipt', async () => {
  const store = createCreatorStore({ creatorApi: apiWith({ projectOpen: async () => opened,
    scan: async () => ({ jobId: 'job-1', projectId: opened.project.project_id, projectRevision: 1 }),
    jobGet: async () => job({ kind: 'build' }) }) });
  await store.getState().openProject(opened.project.project_id);
  await store.getState().startJob('scan', fixtures.jobStart);
  await assert.rejects(store.getState().reopenJob('job-1'), { code: 'invalid_response' });
  assert.equal(store.getState().jobs['job-1'].kind, 'scan');
  assert.equal(store.getState().jobs['job-1'].acceptingEvents, false);
});

test('input diagnostics retain only the last 120 snapshots across buffered and live events', async () => {
  const snapshot = deferred<InputState>();
  const store = createCreatorStore({ fusionInputApi: inputWith({ sessionStart: async () => ({ sessionId: 'session-1' }),
    sessionGet: () => snapshot.promise }) });
  const starting = store.getState().startInputSession('device-1');
  await Promise.resolve();
  for (let sequence = 2; sequence <= 140; sequence++) store.getState().mergeInputState({ ...fixtures.input, sequence });
  snapshot.resolve(fixtures.input);
  await starting;
  assert.deepEqual(store.getState().inputHistory.map(({ sequence }) => sequence), Array.from({ length: 120 }, (_, i) => i + 21));
  assert.equal(store.getState().mergeInputState({ ...fixtures.input, sequence: 141 }), true);
  assert.deepEqual(store.getState().inputHistory.map(({ sequence }) => sequence), Array.from({ length: 120 }, (_, i) => i + 22));
});

test('late initial sessionGet failure cannot overwrite a newer successful read', async () => {
  const snapshot = deferred<InputState>();
  let reads = 0;
  const store = createCreatorStore({ fusionInputApi: inputWith({ sessionStart: async () => ({ sessionId: 'session-1' }),
    sessionGet: async () => ++reads === 1 ? snapshot.promise : fixtures.input }) });
  const starting = store.getState().startInputSession('device-1');
  await Promise.resolve();
  await store.getState().refreshInputSession();
  snapshot.reject(new Error('late snapshot error'));
  await assert.rejects(starting);
  assert.equal(store.getState().inputOwnership, 'owned');
  assert.equal(store.getState().inputStatus, 'ready');
  assert.equal(store.getState().inputAcceptingEvents, true);
});

test('late initial sessionGet failure cannot undo a successful stop ACK', async () => {
  const snapshot = deferred<InputState>();
  const store = createCreatorStore({ fusionInputApi: inputWith({ sessionStart: async () => ({ sessionId: 'session-1' }),
    sessionGet: () => snapshot.promise, sessionStop: async () => ({ sessionId: 'session-1', handlesClosed: true }) }) });
  const starting = store.getState().startInputSession('device-1');
  await Promise.resolve();
  await store.getState().stopInputSession();
  snapshot.reject(new Error('late snapshot error'));
  await assert.rejects(starting);
  assert.equal(store.getState().inputOwnership, 'released');
  assert.equal(store.getState().inputHandlesClosed, true);
  assert.equal(store.getState().inputSessionId, null);
});

test('only an exact unavailable start denial proves no new input session was acquired', async () => {
  const denial = { code: 'action_unavailable', message: 'Unavailable', details: { command: 'fusion_input_session_start' } };
  let starts = 0;
  const store = createCreatorStore({ fusionInputApi: inputWith({ sessionStart: async () => { starts++; throw denial; } }) });
  await assert.rejects(store.getState().startInputSession('device-1'));
  assert.equal(store.getState().inputOwnership, 'none');
  assert.equal(store.getState().inputSessionId, null);
  assert.equal(store.getState().inputHandlesClosed, false);
  assert.equal(store.getState().inputStatus, 'unavailable');
  await assert.rejects(store.getState().startInputSession('device-1'));
  assert.equal(starts, 2);
});

test('disconnection is nonterminal, but stopping and stopped cannot reactivate', async () => {
  const store = createCreatorStore({ fusionInputApi: inputWith({ sessionStart: async () => ({ sessionId: 'session-1' }), sessionGet: async () => fixtures.input }) });
  await store.getState().startInputSession('device-1');
  assert.equal(store.getState().mergeInputState({ ...fixtures.input, sequence: 2, deviceId: null, status: 'disconnected' }), true);
  assert.equal(store.getState().mergeInputState({ ...fixtures.input, sequence: 3, status: 'active' }), true);
  assert.equal(store.getState().mergeInputState({ ...fixtures.input, sequence: 4, status: 'failed' }), true);
  assert.equal(store.getState().mergeInputState({ ...fixtures.input, sequence: 5, status: 'active' }), false);
});

const flushMicrotasks = () => new Promise((resolve) => setTimeout(resolve, 0));

test('openProject hydrates releases and reports while ignoring stale project responses after switch', async () => {
  const staleReleases = deferred<CreatorRelease[]>();
  const staleReport = deferred<CreatorReport>();
  const store = createCreatorStore({ creatorApi: apiWith({
    projectOpen: async ({ projectId }) => projectId === 'other'
      ? { ...opened, project: { ...opened.project, project_id: 'other', revision: 1 }, privateBindings: null }
      : opened,
    releaseList: async ({ projectId }) => projectId === opened.project.project_id
      ? staleReleases.promise
      : [releaseFixture({ releaseId: 'rel-other', projectId: 'other', reportIds: ['rep-other'] })],
    reportGet: async ({ reportId }) => reportId === 'rep-old'
      ? staleReport.promise
      : reportFixture({ reportId: 'rep-other', releaseId: 'rel-other', kind: 'clean_install', status: 'passed' })
  }) });
  await store.getState().openProject(opened.project.project_id);
  const strayReport = store.getState().loadReport('rep-old').catch(() => null);
  await store.getState().openProject('other');
  await flushMicrotasks();
  assert.equal(store.getState().selectedReleaseId, 'rel-other');
  assert.equal(store.getState().reports['rep-other']?.report?.status, 'passed');
  staleReleases.resolve([releaseFixture({ releaseId: 'rel-old', reportIds: ['rep-old'] })]);
  staleReport.resolve(reportFixture({ reportId: 'rep-old', releaseId: 'rel-old', status: 'passed' }));
  await strayReport;
  await flushMicrotasks();
  assert.equal(store.getState().releases.length, 1);
  assert.equal(store.getState().releases[0].releaseId, 'rel-other');
  assert.equal(store.getState().reports['rep-old'], undefined);
});

test('verify startJob reads snapshot, applies monotonic live progress, refreshes reports and cancels cleanly', async () => {
  const receipt = { jobId: 'job-v1', projectId: opened.project.project_id, projectRevision: 1 };
  const request = { ...fixtures.jobStart, releaseId: 'release-1', targetDirectory: null };
  let reportsForRelease: string[] = [];
  let projectsRefreshCount = 0;
  let jobReads = 0;
  const store = createCreatorStore({ creatorApi: apiWith({
    projectList: async () => { projectsRefreshCount++; return [fixtures.projectSummary]; },
    projectOpen: async () => opened,
    releaseList: async () => [releaseFixture({ releaseId: 'release-1', reportIds: reportsForRelease })],
    reportGet: async ({ reportId }) => reportFixture({ reportId, releaseId: 'release-1', kind: 'structure_integrity', status: 'passed' }),
    verify: async (value) => { assert.deepEqual(value, request); return receipt; },
    jobGet: async () => {
      jobReads++;
      if (jobReads === 1) return job({ jobId: receipt.jobId, kind: 'verify', sequence: 2, status: 'running', phase: 'structure_integrity', releaseId: 'release-1' });
      reportsForRelease = ['rep-struct'];
      return {
        progress: progress({ jobId: receipt.jobId, kind: 'verify', sequence: 4, status: 'running', phase: 'clean_install', completedUnits: 1, totalUnits: 2, checkpointId: 'cp-1', releaseId: 'release-1' }),
        checkpoints: [{ checkpointId: 'cp-1', jobId: receipt.jobId, boundary: 'verification_completed', stamp: fixtures.resumeStamp, resultBlake3: 'a'.repeat(64), artifactId: null }],
        reportIds: ['rep-struct']
      };
    },
    jobCancel: async ({ jobId }) => {
      assert.equal(jobId, receipt.jobId);
      return job({ jobId, kind: 'verify', sequence: 5, status: 'cancelling', phase: 'cancelling', releaseId: 'release-1' });
    }
  }) });
  await store.getState().openProject(opened.project.project_id);
  await store.getState().startJob('verify', request);
  assert.equal(store.getState().jobs[receipt.jobId].progress?.status, 'running');
  assert.equal(store.getState().mergeJobProgress(progress({
    jobId: receipt.jobId, kind: 'verify', sequence: 4, status: 'running',
    phase: 'clean_install', completedUnits: 1, totalUnits: 2, checkpointId: 'cp-1', releaseId: 'release-1'
  })), true);
  assert.equal(store.getState().mergeJobProgress(progress({ jobId: receipt.jobId, kind: 'verify', sequence: 3 })), false);
  await flushMicrotasks();
  assert.equal(store.getState().reports['rep-struct']?.report?.kind, 'structure_integrity');
  assert.equal(store.getState().releases[0].reportIds[0], 'rep-struct');
  assert.ok(projectsRefreshCount >= 1);
  await store.getState().cancelJob(receipt.jobId);
  assert.equal(store.getState().jobs[receipt.jobId].progress?.status, 'cancelling');
});

test('late verify receipt after project switch stays inactive and reopening back restores ownership', async () => {
  const pending = deferred<{ jobId: string; projectId: string; projectRevision: number }>();
  const store = createCreatorStore({ creatorApi: apiWith({
    projectOpen: async ({ projectId }) => projectId === 'other'
      ? { ...opened, project: { ...opened.project, project_id: 'other', revision: 1 }, privateBindings: null }
      : opened,
    verify: () => pending.promise,
    jobGet: async ({ jobId }) => job({ jobId, kind: 'verify', sequence: 2, status: 'completed', releaseId: 'release-1' }),
    releaseList: async () => []
  }) });
  await store.getState().openProject(opened.project.project_id);
  const starting = store.getState().startJob('verify', { ...fixtures.jobStart, releaseId: 'release-1', targetDirectory: null });
  await store.getState().openProject('other');
  pending.resolve({ jobId: 'job-vlate', projectId: opened.project.project_id, projectRevision: 1 });
  await starting;
  assert.equal(store.getState().selectedJobId, null);
  assert.equal(store.getState().jobs['job-vlate'].acceptingEvents, false);
  await store.getState().openProject(opened.project.project_id);
  await flushMicrotasks();
  assert.equal(store.getState().jobs['job-vlate'].acceptingEvents, true);
  assert.equal(store.getState().jobs['job-vlate'].progress?.status, 'completed');
});

test('resumeJob resumes interrupted verify with checkpoint and rejects zero-checkpoint jobs', async () => {
  const checkpoint = { checkpointId: 'cp-1', jobId: 'job-int', boundary: 'verification_completed', stamp: fixtures.resumeStamp, resultBlake3: 'a'.repeat(64), artifactId: null };
  let resumed = false;
  const store = createCreatorStore({ creatorApi: apiWith({
    projectOpen: async () => opened,
    releaseList: async () => [releaseFixture({ releaseId: 'release-1', reportIds: resumed ? ['rep-struct', 'rep-clean'] : ['rep-struct'] })],
    reportGet: async ({ reportId }) => reportFixture({ reportId, releaseId: 'release-1', kind: reportId === 'rep-struct' ? 'structure_integrity' : 'clean_install', status: 'passed' }),
    jobGet: async ({ jobId }) => jobId === 'job-zero'
      ? { progress: progress({ jobId, kind: 'verify', sequence: 1, status: 'interrupted', releaseId: 'release-1' }), checkpoints: [], reportIds: [] }
      : {
          progress: progress({ jobId, kind: 'verify', sequence: resumed ? 4 : 2, status: resumed ? 'completed' : 'interrupted', releaseId: 'release-1' }),
          checkpoints: [checkpoint],
          reportIds: resumed ? ['rep-struct', 'rep-clean'] : ['rep-struct']
        },
    jobResume: async ({ jobId }) => {
      assert.equal(jobId, 'job-int');
      resumed = true;
      return { jobId, projectId: opened.project.project_id, projectRevision: 1 };
    }
  }) });
  await store.getState().openProject(opened.project.project_id);
  await store.getState().reopenJob('job-zero');
  await assert.rejects(store.getState().resumeJob('job-zero'), { code: 'invalid_response' });
  await store.getState().reopenJob('job-int');
  await store.getState().resumeJob('job-int');
  await flushMicrotasks();
  assert.equal(store.getState().jobs['job-int'].progress?.status, 'completed');
  assert.equal(store.getState().reports['rep-clean']?.report?.status, 'passed');
});

test('failed verify retry creates distinct job and keeps separate failure and retry reports', async () => {
  let attempt = 0;
  const store = createCreatorStore({ creatorApi: apiWith({
    projectOpen: async () => opened,
    releaseList: async () => [releaseFixture({ releaseId: 'release-1', reportIds: attempt === 1 ? ['rep-fail'] : ['rep-fail', 'rep-struct-2', 'rep-clean-2'] })],
    reportGet: async ({ reportId }) => reportId === 'rep-fail'
      ? reportFixture({ reportId, releaseId: 'release-1', kind: 'clean_install', status: 'failed' })
      : reportFixture({ reportId, releaseId: 'release-1', kind: reportId === 'rep-struct-2' ? 'structure_integrity' : 'clean_install', status: 'passed' }),
    verify: async () => ({ jobId: `job-try-${++attempt}`, projectId: opened.project.project_id, projectRevision: 1 }),
    jobGet: async ({ jobId }) => jobId === 'job-try-1'
      ? {
          progress: progress({ jobId, kind: 'verify', sequence: 3, status: 'failed', releaseId: 'release-1', error: { code: 'integrity_violation', message: 'Tampered', details: { passedStages: ['structure_integrity'] } } }),
          checkpoints: [],
          reportIds: ['rep-fail']
        }
      : {
          progress: progress({ jobId, kind: 'verify', sequence: 3, status: 'completed', releaseId: 'release-1' }),
          checkpoints: [{ checkpointId: 'cp-2', jobId, boundary: 'verification_completed', stamp: fixtures.resumeStamp, resultBlake3: 'a'.repeat(64), artifactId: null }],
          reportIds: ['rep-struct-2', 'rep-clean-2']
        }
  }) });
  await store.getState().openProject(opened.project.project_id);
  const req = { ...fixtures.jobStart, releaseId: 'release-1', targetDirectory: null };
  await store.getState().startJob('verify', req);
  await store.getState().startJob('verify', req);
  await flushMicrotasks();
  assert.equal(store.getState().jobs['job-try-1'].progress?.status, 'failed');
  assert.equal(store.getState().jobs['job-try-2'].progress?.status, 'completed');
  assert.equal(store.getState().reports['rep-fail']?.report?.status, 'failed');
  assert.equal(store.getState().reports['rep-clean-2']?.report?.status, 'passed');
});

test('export and verifyHttps read snapshots eagerly and hydrate https_acquisition reports', async () => {
  const exportReq = { ...fixtures.jobStart, releaseId: 'release-1', targetDirectory: 'D:/exports/game', sourceUrl: null };
  const httpsReq = { ...fixtures.jobStart, releaseId: 'release-1', targetDirectory: null, sourceUrl: 'https://example.org/source-index.json' };
  let httpsFinished = false;
  const store = createCreatorStore({ creatorApi: apiWith({
    projectOpen: async () => opened,
    releaseList: async () => [releaseFixture({ releaseId: 'release-1', reportIds: httpsFinished ? ['rep-https'] : [] })],
    reportGet: async ({ reportId }) => reportFixture({
      reportId, releaseId: 'release-1', kind: 'https_acquisition', status: 'passed',
      environment: { os: 'windows', source_url: httpsReq.sourceUrl }
    }),
    export: async (req) => { assert.deepEqual(req, exportReq); return { jobId: 'job-exp', projectId: opened.project.project_id, projectRevision: 1 }; },
    verifyHttps: async (req) => { assert.deepEqual(req, httpsReq); httpsFinished = true; return { jobId: 'job-https', projectId: opened.project.project_id, projectRevision: 1 }; },
    jobGet: async ({ jobId }) => jobId === 'job-exp'
      ? {
          progress: progress({ jobId, kind: 'export', sequence: 2, status: 'completed', releaseId: 'release-1', checkpointId: 'cp-exp' }),
          checkpoints: [{ checkpointId: 'cp-exp', jobId, boundary: 'export_completed', stamp: fixtures.resumeStamp, resultBlake3: 'a'.repeat(64), artifactId: null }],
          reportIds: []
        }
      : {
          progress: progress({ jobId, kind: 'verify_https', sequence: 2, status: 'completed', releaseId: 'release-1', checkpointId: 'cp-https' }),
          checkpoints: [{ checkpointId: 'cp-https', jobId, boundary: 'verification_completed', stamp: fixtures.resumeStamp, resultBlake3: 'a'.repeat(64), artifactId: null }],
          reportIds: ['rep-https']
        }
  }) });
  await store.getState().openProject(opened.project.project_id);
  await store.getState().startJob('export', exportReq);
  assert.equal(store.getState().jobs['job-exp'].progress?.status, 'completed');
  await store.getState().startJob('verify_https', httpsReq);
  await flushMicrotasks();
  assert.equal(store.getState().jobs['job-https'].progress?.status, 'completed');
  assert.equal(store.getState().reports['rep-https']?.report?.kind, 'https_acquisition');
  assert.equal(store.getState().reports['rep-https']?.report?.status, 'passed');
});

test('saveInputBindings persists lastSavedBindings on match and records structured errors on rejection', async () => {
  let rejectSave = false;
  const store = createCreatorStore({
    fusionInputApi: inputWith({
      sessionStart: async () => ({ sessionId: 'session-1' }),
      sessionGet: async () => ({ sessionId: 'session-1', sequence: 1, deviceId: 'device-1', status: 'active', buttons: {}, axes: {}, error: null }),
      bindingsSave: async (req) => {
        if (rejectSave) throw { code: 'action_unavailable', message: 'Controller changed.', details: {} };
        return { sessionId: req.sessionId, controlsProfileId: req.controlsProfileId };
      }
    })
  });
  const req = {
    sessionId: 'session-1',
    controlsProfileId: 'sdl-p0',
    bindings: { a: { kind: 'controller_button' as const, button: 'b' } },
    deadzones: { leftx: 0.15 },
    rawBindings: {}
  };
  await assert.rejects(store.getState().saveInputBindings(req), { code: 'invalid_response' });
  await store.getState().startInputSession('device-1');
  const receipt = await store.getState().saveInputBindings(req);
  assert.deepEqual(receipt, { sessionId: 'session-1', controlsProfileId: 'sdl-p0' });
  assert.equal(store.getState().inputSaveStatus, 'ready');
  assert.deepEqual(store.getState().lastSavedBindings, receipt);

  rejectSave = true;
  await assert.rejects(store.getState().saveInputBindings(req));
  assert.equal(store.getState().inputSaveStatus, 'unavailable');
  assert.equal(store.getState().inputSaveError?.code, 'action_unavailable');
});
