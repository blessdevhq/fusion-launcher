import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { createCreatorStore } from './creatorStore.ts';
import type { CreatorApi, FusionInputApi, CreatorJob, CreatorJobProgress, CreatorProjectOpenResponse, InputReleaseReceipt, InputState } from '../types/fusionCreator.ts';

const fixtures = JSON.parse(readFileSync(new URL('../../fixtures/creator/contracts.json', import.meta.url), 'utf8'));
const opened: CreatorProjectOpenResponse = { project: fixtures.project, privateBindings: fixtures.projectBindings, missingInputs: fixtures.projectMissingInputs };
const progress = (overrides: Partial<CreatorJobProgress> = {}): CreatorJobProgress => ({ ...fixtures.job, ...overrides });
const job = (overrides: Partial<CreatorJobProgress> = {}): CreatorJob => ({ progress: progress(overrides), checkpoints: [], reportIds: [] });

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

test('input devices expose unavailable rather than a successful fake empty list', async () => {
  const store = createCreatorStore({ fusionInputApi: inputApi });
  await assert.rejects(store.getState().loadInputDevices(), { code: 'action_unavailable' });
  assert.equal(store.getState().inputDevicesStatus, 'unavailable');
  const ready = createCreatorStore({ fusionInputApi: inputWith({ devices: async () => [] }) });
  await ready.getState().loadInputDevices();
  assert.equal(ready.getState().inputDevicesStatus, 'ready');
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

test('opening a project again gates cached jobs until a fresh jobGet completes', async () => {
  const store = createCreatorStore({ creatorApi: apiWith({ projectOpen: async () => opened, jobGet: async () => job() }) });
  await store.getState().openProject(opened.project.project_id);
  await store.getState().reopenJob('job-1');
  await store.getState().openProject(opened.project.project_id);
  assert.equal(store.getState().mergeJobProgress(progress({ sequence: 2 })), false);
  await store.getState().reopenJob('job-1');
  assert.equal(store.getState().mergeJobProgress(progress({ sequence: 2 })), true);
});

test('a started job rejects a different kind even for its first event', async () => {
  const store = createCreatorStore({ creatorApi: apiWith({ projectOpen: async () => opened,
    scan: async () => ({ jobId: 'job-1', projectId: opened.project.project_id, projectRevision: 1 }) }) });
  await store.getState().openProject(opened.project.project_id);
  await store.getState().startJob('scan', fixtures.jobStart);
  assert.equal(store.getState().mergeJobProgress(progress({ kind: 'build' })), false);
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
