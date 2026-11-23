import { create } from 'zustand';
import { z } from 'zod';
import { creatorApi, fusionInputApi } from '../lib/creatorApi.ts';
import { creatorJobProgressSchema, creatorPrivateBindingsSaveResponseSchema, creatorProjectOpenResponseSchema, creatorProjectListResponseSchema, inputStateSchema } from '../types/fusionCreator.ts';
import type { CreatorApi, FusionInputApi, CreatorProjectSummary, CreatorProjectOpenResponse,
  CreatorJob, CreatorJobProgress, CreatorJobReceipt, CreatorJobKind, CreatorJobStartRequest, CreatorPrivateBindings,
  InputDevice, InputState, InputSessionReceipt, InputReleaseReceipt } from '../types/fusionCreator.ts';
import type { FusionError } from '../types/fusionSource.ts';

export type CreatorLoadStatus = 'idle' | 'loading' | 'ready' | 'unavailable' | 'error';
export type InputOwnership = 'none' | 'starting' | 'owned' | 'releasing' | 'uncertain' | 'released';
export interface CreatorStoreDependencies { creatorApi: CreatorApi; fusionInputApi: FusionInputApi }
export interface CreatorJobEntry {
  receipt: CreatorJobReceipt;
  kind: CreatorJobKind | null;
  job: CreatorJob | null;
  progress: CreatorJobProgress | null;
  history: CreatorJobProgress[];
  status: CreatorLoadStatus;
  error: FusionError | null;
  acceptingEvents: boolean;
}
export interface CreatorState {
  projects: CreatorProjectSummary[];
  projectsStatus: CreatorLoadStatus;
  projectsError: FusionError | null;
  loadProjects(): Promise<void>;
  currentProject: CreatorProjectOpenResponse | null;
  projectStatus: CreatorLoadStatus;
  projectError: FusionError | null;
  openProject(projectId: string): Promise<CreatorProjectOpenResponse>;
  savePrivateBindings(opened: CreatorProjectOpenResponse, bindings: CreatorPrivateBindings): Promise<void>;
  jobs: Record<string, CreatorJobEntry>;
  selectedJobId: string | null;
  jobError: FusionError | null;
  startJob(kind: CreatorJobKind, request: CreatorJobStartRequest): Promise<CreatorJobReceipt>;
  cancelJob(jobId: string): Promise<void>;
  reopenJob(jobId: string, select?: boolean): Promise<CreatorJob>;
  refreshKnownJobs(): void;
  mergeJobProgress(progress: CreatorJobProgress): boolean;
  inputDevices: InputDevice[];
  inputDevicesStatus: CreatorLoadStatus;
  inputDevicesError: FusionError | null;
  inputSessionId: string | null;
  inputDeviceId: string | null;
  inputState: InputState | null;
  inputHistory: InputState[];
  inputOwnership: InputOwnership;
  inputHandlesClosed: boolean;
  inputStatus: CreatorLoadStatus;
  inputError: FusionError | null;
  inputAcceptingEvents: boolean;
  loadInputDevices(): Promise<void>;
  startInputSession(deviceId: string): Promise<InputSessionReceipt>;
  refreshInputSession(): Promise<InputState>;
  mergeInputState(state: InputState): boolean;
  stopInputSession(): Promise<InputReleaseReceipt>;
  releaseInputForLaunch(): Promise<InputReleaseReceipt>;
}

export function creatorError(error: unknown): FusionError {
  if (typeof error === 'object' && error !== null && 'code' in error && 'message' in error
    && typeof error.code === 'string' && typeof error.message === 'string') {
    return { code: error.code, message: error.message, details: 'details' in error && typeof error.details === 'object' && error.details !== null ? error.details as Record<string, unknown> : {} };
  }
  return { code: 'creator_error', message: error instanceof Error ? error.message : String(error), details: {} };
}
const errorStatus = (error: FusionError): CreatorLoadStatus => error.code === 'action_unavailable' ? 'unavailable' : 'error';
const invalid = (message: string): FusionError => ({ code: 'invalid_response', message, details: {} });
const inputDevicesSchema: z.ZodType<InputDevice[]> = z.array(z.object({
  deviceId: inputStateSchema.shape.sessionId, name: z.string(), connected: z.boolean(), mappingKnown: z.boolean()
}).strict()).refine((devices) => new Set(devices.map((device) => device.deviceId)).size === devices.length,
  'Input device identities must be unique.');
const terminalJob = (progress: CreatorJobProgress) => ['cancelled', 'interrupted', 'completed', 'failed'].includes(progress.status);
const sameIdentity = (receipt: CreatorJobReceipt, value: CreatorJobReceipt) => receipt.jobId === value.jobId
  && receipt.projectId === value.projectId && receipt.projectRevision === value.projectRevision;
const currentOwns = (current: CreatorProjectOpenResponse | null, value: { projectId: string; projectRevision: number }) =>
  current?.project.project_id === value.projectId && current.project.revision === value.projectRevision;
const emptyJob = (receipt: CreatorJobReceipt): CreatorJobEntry => ({ receipt, kind: null, job: null, progress: null, history: [], status: 'loading', error: null, acceptingEvents: false });
const ownJob = (jobs: Record<string, CreatorJobEntry>, id: string) => Object.hasOwn(jobs, id) ? jobs[id] : undefined;
function withProgress(entry: CreatorJobEntry, progress: CreatorJobProgress): CreatorJobEntry {
  return { ...entry, kind: progress.kind, progress, job: entry.job ? { ...entry.job, progress } : null, history: [...entry.history, progress] };
}
function canAdvance(previous: CreatorJobProgress | null, incoming: CreatorJobProgress) {
  return !previous || (incoming.sequence > previous.sequence && incoming.kind === previous.kind
    && !(terminalJob(previous) && !terminalJob(incoming)));
}
// Disconnected is not a closure ACK or terminal teardown. Reconnection may be observed
// in the same owned session; only stopping/terminal states prevent reactivation.
const inputRank: Record<InputState['status'], number> = { active: 0, disconnected: 0, stopping: 1, stopped: 2, failed: 2 };
const canAdvanceInput = (previous: InputState | null, incoming: InputState) => !previous
  || (incoming.sequence > previous.sequence && inputRank[incoming.status] >= inputRank[previous.status]);
const INPUT_HISTORY_LIMIT = 120;
const rememberInput = (history: InputState[], incoming: InputState) => [...history.slice(1 - INPUT_HISTORY_LIMIT), incoming];

/** Each factory has independent state; use only explicit injected APIs for preview/test fixtures. */
export function createCreatorStore(dependencies: Partial<CreatorStoreDependencies> = {}) {
  const api = dependencies.creatorApi ?? creatorApi;
  const input = dependencies.fusionInputApi ?? fusionInputApi;
  let listGeneration = 0;
  let projectGeneration = 0;
  let savingBindings = false;
  const jobGenerations = new Map<string, number>();
  const pendingJobs = new Map<string, CreatorJobProgress[]>();
  let devicesGeneration = 0;
  let inputGeneration = 0;

  let pendingInput: { sessionId: string; events: InputState[] } | null = null;
  async function readProject(projectId: string): Promise<CreatorProjectOpenResponse> {
    const response = creatorProjectOpenResponseSchema.parse(await api.projectOpen({ projectId }));
    if (response.project.project_id !== projectId) throw invalid('Opened project identity differs from the request.');
    if (response.privateBindings && response.privateBindings.bindings.projectId !== projectId) throw invalid('Private bindings belong to a different project.');
    const components = new Set(response.project.program_descriptor.components.map(component => component.id));
    if (response.missingInputs.some(id => !components.has(id))) throw invalid('Missing input is not a descriptor component.');
    return response;
  }
  return create<CreatorState>((set, get) => {
    const releaseInput = async (forLaunch: boolean): Promise<InputReleaseReceipt> => {
      const { inputSessionId: sessionId, inputOwnership } = get();
      if (!sessionId || inputOwnership === 'starting' || inputOwnership === 'releasing') throw invalid('Input release requires an owned session and no other release in flight.');
      ++inputGeneration;

      pendingInput = null;
      set({ inputOwnership: 'releasing', inputHandlesClosed: false, inputAcceptingEvents: false, inputError: null, inputStatus: 'loading' });
      try {
        const receipt = await (forLaunch ? input.releaseForLaunch({ sessionId }) : input.sessionStop({ sessionId }));
        if (receipt.sessionId !== sessionId || receipt.handlesClosed !== true) throw invalid('Native worker has not acknowledged closing this session’s handles.');
        set({ inputSessionId: null, inputDeviceId: null, inputOwnership: 'released', inputHandlesClosed: true, inputStatus: 'ready' });
        return receipt;
      } catch (error) {
        const failure = creatorError(error);
        set({ inputOwnership: 'uncertain', inputHandlesClosed: false, inputError: failure, inputStatus: errorStatus(failure) });
        throw error;
      }
    };
    return {
    projects: [], projectsStatus: 'idle', projectsError: null,
    currentProject: null, projectStatus: 'idle', projectError: null,
    jobs: {}, selectedJobId: null, jobError: null,
    inputDevices: [], inputDevicesStatus: 'idle', inputDevicesError: null,
    inputSessionId: null, inputDeviceId: null, inputState: null, inputHistory: [],
    inputOwnership: 'none', inputHandlesClosed: false, inputStatus: 'idle', inputError: null, inputAcceptingEvents: false,
    async loadProjects() {
      const generation = ++listGeneration;
      set({ projectsStatus: 'loading', projectsError: null });
      try {
        const projects = creatorProjectListResponseSchema.parse(await api.projectList());
        if (generation === listGeneration) set({ projects, projectsStatus: 'ready' });
      } catch (error) {
        if (generation === listGeneration) {
          const failure = creatorError(error);
          set({ projectsStatus: errorStatus(failure), projectsError: failure });
        }
        throw error;
      }
    },
    async openProject(projectId) {
      const generation = ++projectGeneration;
      pendingJobs.clear();
      for (const [id, previous] of jobGenerations) jobGenerations.set(id, previous + 1);
      set((state) => ({ currentProject: null, projectStatus: 'loading', projectError: null, selectedJobId: null,
        jobs: Object.fromEntries(Object.entries(state.jobs).map(([id, entry]) => [id, { ...entry, acceptingEvents: false }])) }));
      try {
        const response = await readProject(projectId);
        if (generation === projectGeneration) {
          set({ currentProject: response, projectStatus: 'ready' });
          get().refreshKnownJobs();
        }
        return response;
      } catch (error) {
        if (generation === projectGeneration) {
          const failure = creatorError(error);
          set({ projectStatus: errorStatus(failure), projectError: failure });
        }
        throw error;
      }
    },
    async savePrivateBindings(opened, bindings) {
      if (savingBindings || get().currentProject !== opened || bindings.projectId !== opened.project.project_id) {
        throw invalid('Saving requires the current private bindings revision and no other save in flight.');
      }
      const expectedBindingsRevision = opened.privateBindings?.revision ?? null;
      savingBindings = true;
      try {
        const receipt = creatorPrivateBindingsSaveResponseSchema.safeParse(await api.privateBindingsSave({
          projectRevision: opened.project.revision, expectedBindingsRevision, bindings
        }));
        if (!receipt.success || receipt.data.revision !== (expectedBindingsRevision === null ? 0 : expectedBindingsRevision + 1)) {
          throw invalid('Unexpected private bindings revision.');
        }
        if (get().currentProject !== opened) return;
        const updated = { ...opened, privateBindings: { revision: receipt.data.revision, bindings } };
        set({ currentProject: updated });
        // Saving a path does not prove availability. Re-read the authoritative
        // missing-input list without invalidating ownership of jobs on this revision.
        const fresh = await readProject(bindings.projectId).catch(() => null);
        if (get().currentProject === updated && fresh?.project.revision === opened.project.revision
          && fresh.privateBindings?.revision === receipt.data.revision) {
          set({ currentProject: { ...updated, missingInputs: fresh.missingInputs } });
        }
      } finally {
        savingBindings = false;
      }
    },
    async startJob(kind, request) {
      const generation = projectGeneration;
      try {
        if (!currentOwns(get().currentProject, request)) throw invalid('Job must target the current project revision.');
        const method = kind === 'verify_https' ? 'verifyHttps' : kind;
        const receipt = await api[method](request);
        if (!inputStateSchema.shape.sessionId.safeParse(receipt?.jobId).success || receipt.projectId !== request.projectId || receipt.projectRevision !== request.projectRevision) throw invalid('Job receipt differs from the requested identity.');
        const existing = ownJob(get().jobs, receipt.jobId);
        if (existing && (!sameIdentity(existing.receipt, receipt) || (existing.kind !== null && existing.kind !== kind))) throw invalid('Job ID already belongs to another job.');
        const current = currentOwns(get().currentProject, receipt);
        const ownsSelection = current && generation === projectGeneration;
        // Keep a valid receipt even if the project was changed while awaiting IPC.
        set(state => ({
          jobs: { ...state.jobs, [receipt.jobId]: existing ?? { ...emptyJob(receipt), kind, status: 'ready', acceptingEvents: ownsSelection && kind !== 'build' } },
          ...(ownsSelection ? { selectedJobId: receipt.jobId, jobError: null } : {})
        }));
        // A fast worker can finish before the receipt is delivered. Read durable state
        // instead of relying on a future event; a read failure isn't a failed start.
        if (current && (kind === 'build' || !ownsSelection)) await get().reopenJob(receipt.jobId, ownsSelection).catch(() => {});
        return receipt;
      } catch (error) {
        if (generation === projectGeneration) set({ jobError: creatorError(error) });
        throw error;
      }
    },
    async cancelJob(jobId) {
      const entry = ownJob(get().jobs, jobId);
      const previous = entry?.progress;
      if (!previous || !entry.acceptingEvents || !currentOwns(get().currentProject, previous)
        || (previous.status !== 'queued' && previous.status !== 'running')) throw invalid('No cancellable job for the current project.');
      const generation = projectGeneration;
      const snapshot = await api.jobCancel({ jobId });
      const parsed = creatorJobProgressSchema.safeParse(snapshot?.progress);
      if (!parsed.success || !sameIdentity(entry.receipt, parsed.data) || parsed.data.kind !== previous.kind) {
        throw invalid('Cancel response differs from the requested job.');
      }
      if (generation === projectGeneration) get().mergeJobProgress(parsed.data);
    },
    refreshKnownJobs() {
      const { currentProject, jobs } = get();
      for (const { receipt } of Object.values(jobs)) {
        if (currentOwns(currentProject, receipt)) void get().reopenJob(receipt.jobId, false).catch(() => {});
      }
    },
    async reopenJob(jobId, select = true) {
      const current = get().currentProject;
      if (!current) throw invalid('Open a project before reopening its job.');
      const receipt = { jobId, projectId: current.project.project_id, projectRevision: current.project.revision };
      const existing = ownJob(get().jobs, jobId);
      if (existing && !sameIdentity(existing.receipt, receipt)) throw invalid('Job is not owned by this project revision.');
      const generation = (jobGenerations.get(jobId) ?? 0) + 1;
      jobGenerations.set(jobId, generation);
      const pending = pendingJobs.get(jobId) ?? [];
      pendingJobs.set(jobId, pending);
      set((state) => ({ jobs: { ...state.jobs, [jobId]: { ...(existing ?? emptyJob(receipt)), status: 'loading', error: null, acceptingEvents: false } },
        ...(select ? { selectedJobId: jobId, jobError: null } : {}) }));
      try {
        const snapshot = await api.jobGet({ jobId });
        const progress = creatorJobProgressSchema.parse(snapshot.progress);
        if (!sameIdentity(receipt, progress) || (existing?.kind != null && existing.kind !== progress.kind)) {
          throw invalid('Reopened job differs from its requested identity or known kind.');
        }
        if (jobGenerations.get(jobId) === generation) {
          set((state) => {
            const entry = state.jobs[jobId];
            let advanced = canAdvance(entry.progress, progress) ? withProgress(entry, progress) : entry;
            if (advanced !== entry || !entry.job) {
              advanced = { ...advanced, job: { ...snapshot, progress: advanced.progress ?? progress } };
            }
            for (const event of pending) {
              if (canAdvance(advanced.progress, event)) advanced = withProgress(advanced, event);
            }
            pendingJobs.delete(jobId);
            return { jobs: { ...state.jobs, [jobId]: { ...advanced, status: 'ready', acceptingEvents: true } } };
          });
        }
        return snapshot;
      } catch (error) {
        if (jobGenerations.get(jobId) === generation) {
          pendingJobs.delete(jobId);
          const failure = creatorError(error);
          set((state) => ({ jobs: { ...state.jobs, [jobId]: { ...state.jobs[jobId], status: errorStatus(failure), error: failure, acceptingEvents: false } }, jobError: failure }));
        }
        throw error;
      }
    },
    mergeJobProgress(value) {
      const parsed = creatorJobProgressSchema.safeParse(value);
      if (!parsed.success) return false;
      const progress = parsed.data;
      const entry = ownJob(get().jobs, progress.jobId);
      if (!entry || (entry.kind !== null && entry.kind !== progress.kind) || !sameIdentity(entry.receipt, progress)
        || !currentOwns(get().currentProject, progress)) return false;
      const pending = pendingJobs.get(progress.jobId);
      if (pending) {
        pending.push(progress);
        return false; // Buffered, not applied until the snapshot has validated ownership.
      }
      if (!entry.acceptingEvents || !canAdvance(entry.progress, progress)) return false;
      set((state) => ({ jobs: { ...state.jobs, [progress.jobId]: withProgress(entry, progress) } }));
      return true;
    },
    async loadInputDevices() {
      const generation = ++devicesGeneration;
      set({ inputDevicesStatus: 'loading', inputDevicesError: null });
      try {
        const parsed = inputDevicesSchema.safeParse(await input.devices());
        if (!parsed.success) throw invalid('Input devices response is malformed or contains duplicate identities.');
        const devices = parsed.data;
        if (generation === devicesGeneration) set({ inputDevices: devices, inputDevicesStatus: 'ready' });
      } catch (error) {
        if (generation === devicesGeneration) {
          const failure = creatorError(error);
          set({ inputDevicesStatus: errorStatus(failure), inputDevicesError: failure });
        }
        throw error;
      }
    },
    async startInputSession(deviceId) {
      if (!['none', 'released'].includes(get().inputOwnership)) throw invalid('Prior input ownership has not been released by the native worker.');
      set({ inputOwnership: 'starting', inputHandlesClosed: false, inputStatus: 'loading', inputError: null, inputState: null, inputHistory: [], inputAcceptingEvents: false, inputDeviceId: deviceId });
      let receipt: InputSessionReceipt;
      try {
        receipt = await input.sessionStart({ deviceId });
        if (!inputStateSchema.shape.sessionId.safeParse(receipt?.sessionId).success) throw invalid('Input start returned no valid session identity.');
      } catch (error) {
        const failure = creatorError(error);
        // Only an exact start denial proves non-acquisition. Snapshot errors are
        // handled by refreshInputSession, which owns the read generation.
        const deniedBeforeAcquisition = failure.code === 'action_unavailable'
          && failure.details.command === 'fusion_input_session_start';
        set({ inputOwnership: deniedBeforeAcquisition ? 'none' : 'uncertain', inputDeviceId: deniedBeforeAcquisition ? null : deviceId, inputStatus: errorStatus(failure), inputError: failure, inputHandlesClosed: false });
        throw error;
      }
      set({ inputSessionId: receipt.sessionId, inputOwnership: 'owned' });
      await get().refreshInputSession();
      return receipt;
    },
    async refreshInputSession() {
      const { inputSessionId: sessionId, inputDeviceId: deviceId, inputOwnership } = get();
      if (!sessionId || inputOwnership === 'releasing') throw invalid('No input session is available to query.');
      const generation = ++inputGeneration;
      const pending = { sessionId, events: pendingInput?.sessionId === sessionId ? pendingInput.events : [] };
      pendingInput = pending;
      set({ inputAcceptingEvents: false, inputStatus: 'loading', inputError: null });
      try {
        const snapshot = inputStateSchema.parse(await input.sessionGet({ sessionId }));
        if (snapshot.sessionId !== sessionId || (snapshot.deviceId !== null && snapshot.deviceId !== deviceId)) throw invalid('Input snapshot identity differs from its owner.');
        if (generation === inputGeneration && get().inputSessionId === sessionId) {
          let previous = get().inputState;
          const history = [...get().inputHistory];
          for (const incoming of [snapshot, ...pending.events]) {
            if (canAdvanceInput(previous, incoming)) {
              previous = incoming;
              history.push(incoming);
            }
          }
          pendingInput = null;
          set({ inputState: previous, inputHistory: history.slice(-INPUT_HISTORY_LIMIT), inputStatus: 'ready', inputAcceptingEvents: true });
        }
        return snapshot;
      } catch (error) {
        if (generation === inputGeneration && get().inputSessionId === sessionId) {
          pendingInput = null;
          const failure = creatorError(error);
          set({ inputStatus: errorStatus(failure), inputError: failure, inputOwnership: 'uncertain', inputAcceptingEvents: false });
        }
        throw error;
      }
    },
    mergeInputState(value) {
      const parsed = inputStateSchema.safeParse(value);
      if (!parsed.success) return false;
      const incoming = parsed.data;
      const state = get();
      if (incoming.sessionId !== state.inputSessionId
        || (incoming.deviceId !== null && incoming.deviceId !== state.inputDeviceId)) return false;
      if (pendingInput?.sessionId === incoming.sessionId) {
        // Only the validated snapshot establishes the status baseline for replay.
        pendingInput.events = rememberInput(pendingInput.events, incoming);
        return false;
      }
      if (!state.inputAcceptingEvents || !canAdvanceInput(state.inputState, incoming)) return false;
      set({ inputState: incoming, inputHistory: rememberInput(state.inputHistory, incoming) });
      return true;
    },
    stopInputSession: () => releaseInput(false),
    releaseInputForLaunch: () => releaseInput(true)
    };
  });
}

export const useCreatorStore = createCreatorStore();
