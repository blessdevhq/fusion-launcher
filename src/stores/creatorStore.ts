import { create } from 'zustand';
import { creatorApi, fusionInputApi } from '../lib/creatorApi.ts';
import { creatorJobProgressSchema, creatorProjectOpenResponseSchema, creatorProjectListResponseSchema, inputStateSchema } from '../types/fusionCreator.ts';
import type { CreatorApi, FusionInputApi, CreatorProjectSummary, CreatorProjectOpenResponse,
  CreatorJob, CreatorJobProgress, CreatorJobReceipt, CreatorJobKind, CreatorJobStartRequest,
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
  jobs: Record<string, CreatorJobEntry>;
  selectedJobId: string | null;
  jobError: FusionError | null;
  startJob(kind: CreatorJobKind, request: CreatorJobStartRequest): Promise<CreatorJobReceipt>;
  reopenJob(jobId: string): Promise<CreatorJob>;
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

/** Each factory has independent state; use only explicit injected APIs for preview/test fixtures. */
export function createCreatorStore(dependencies: Partial<CreatorStoreDependencies> = {}) {
  const api = dependencies.creatorApi ?? creatorApi;
  const input = dependencies.fusionInputApi ?? fusionInputApi;
  let listGeneration = 0;
  let projectGeneration = 0;
  const jobGenerations = new Map<string, number>();
  let devicesGeneration = 0;
  let inputGeneration = 0;
  let inputOwnerGeneration = 0;
  return create<CreatorState>((set, get) => {
    const releaseInput = async (forLaunch: boolean): Promise<InputReleaseReceipt> => {
      const { inputSessionId: sessionId, inputOwnership } = get();
      if (!sessionId || inputOwnership === 'starting' || inputOwnership === 'releasing') throw invalid('Input release requires an owned session and no other release in flight.');
      ++inputGeneration;
      ++inputOwnerGeneration;
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
      for (const [id, previous] of jobGenerations) jobGenerations.set(id, previous + 1);
      set((state) => ({ currentProject: null, projectStatus: 'loading', projectError: null, selectedJobId: null,
        jobs: Object.fromEntries(Object.entries(state.jobs).map(([id, entry]) => [id, { ...entry, acceptingEvents: false }])) }));
      try {
        const response = creatorProjectOpenResponseSchema.parse(await api.projectOpen({ projectId }));
        if (response.project.project_id !== projectId) throw invalid('Opened project identity differs from the request.');
        if (response.privateBindings && response.privateBindings.bindings.projectId !== projectId) throw invalid('Private bindings belong to a different project.');
        const components = new Set(response.project.program_descriptor.components.map((component) => component.id));
        if (response.missingInputs.some((id) => !components.has(id))) throw invalid('Missing input is not a descriptor component.');
        if (generation === projectGeneration) set({ currentProject: response, projectStatus: 'ready' });
        return response;
      } catch (error) {
        if (generation === projectGeneration) {
          const failure = creatorError(error);
          set({ projectStatus: errorStatus(failure), projectError: failure });
        }
        throw error;
      }
    },
    async startJob(kind, request) {
      try {
        if (!currentOwns(get().currentProject, request)) throw invalid('Job must target the current project revision.');
        const method = kind === 'verify_https' ? 'verifyHttps' : kind;
        const receipt = await api[method](request);
        if (!receipt.jobId || receipt.projectId !== request.projectId || receipt.projectRevision !== request.projectRevision) throw invalid('Job receipt differs from the requested identity.');
        const existing = ownJob(get().jobs, receipt.jobId);
        if (existing && !sameIdentity(existing.receipt, receipt)) throw invalid('Job ID already belongs to another project revision.');
        set((state) => ({ jobs: { ...state.jobs, [receipt.jobId]: existing ?? { ...emptyJob(receipt), kind, status: 'ready', acceptingEvents: true } }, selectedJobId: receipt.jobId, jobError: null }));
        return receipt;
      } catch (error) {
        set({ jobError: creatorError(error) });
        throw error;
      }
    },
    async reopenJob(jobId) {
      const current = get().currentProject;
      if (!current) throw invalid('Open a project before reopening its job.');
      const receipt = { jobId, projectId: current.project.project_id, projectRevision: current.project.revision };
      const existing = ownJob(get().jobs, jobId);
      if (existing && !sameIdentity(existing.receipt, receipt)) throw invalid('Job is not owned by this project revision.');
      const generation = (jobGenerations.get(jobId) ?? 0) + 1;
      jobGenerations.set(jobId, generation);
      set((state) => ({ jobs: { ...state.jobs, [jobId]: { ...(existing ?? emptyJob(receipt)), status: 'loading', error: null, acceptingEvents: false } }, selectedJobId: jobId, jobError: null }));
      try {
        const snapshot = await api.jobGet({ jobId });
        const progress = creatorJobProgressSchema.parse(snapshot.progress);
        if (!sameIdentity(receipt, progress)) throw invalid('Reopened job differs from its requested identity.');
        if (jobGenerations.get(jobId) === generation) {
          set((state) => {
            const entry = state.jobs[jobId];
            const advanced = canAdvance(entry.progress, progress) ? withProgress(entry, progress) : entry;
            return { jobs: { ...state.jobs, [jobId]: { ...advanced, job: advanced.progress === progress ? { ...snapshot, progress } : entry.job, status: 'ready', acceptingEvents: true } } };
          });
        }
        return snapshot;
      } catch (error) {
        if (jobGenerations.get(jobId) === generation) {
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
      if (!entry?.acceptingEvents || (entry.kind !== null && entry.kind !== progress.kind) || !sameIdentity(entry.receipt, progress)
        || !currentOwns(get().currentProject, progress) || !canAdvance(entry.progress, progress)) return false;
      set((state) => ({ jobs: { ...state.jobs, [progress.jobId]: withProgress(entry, progress) } }));
      return true;
    },
    async loadInputDevices() {
      const generation = ++devicesGeneration;
      set({ inputDevicesStatus: 'loading', inputDevicesError: null });
      try {
        const devices = await input.devices();
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
      const ownerGeneration = ++inputOwnerGeneration;
      set({ inputOwnership: 'starting', inputHandlesClosed: false, inputStatus: 'loading', inputError: null, inputState: null, inputHistory: [], inputAcceptingEvents: false, inputDeviceId: deviceId });
      try {
        const receipt = await input.sessionStart({ deviceId });
        if (typeof receipt.sessionId !== 'string' || !receipt.sessionId) throw invalid('Input start returned no session identity.');
        set({ inputSessionId: receipt.sessionId, inputOwnership: 'owned' });
        await get().refreshInputSession();
        return receipt;
      } catch (error) {
        const failure = creatorError(error);
        // The frozen exact start denial guarantees no session was created. No other
        // error (including a failed sessionGet after receipt) proves non-acquisition.
        const deniedBeforeAcquisition = !get().inputSessionId && failure.code === 'action_unavailable'
          && failure.details.command === 'fusion_input_session_start';
        if (ownerGeneration === inputOwnerGeneration) set({ inputOwnership: deniedBeforeAcquisition ? 'none' : 'uncertain', inputDeviceId: deniedBeforeAcquisition ? null : get().inputDeviceId, inputStatus: errorStatus(failure), inputError: failure, inputHandlesClosed: false });
        throw error;
      }
    },
    async refreshInputSession() {
      const { inputSessionId: sessionId, inputDeviceId: deviceId, inputOwnership } = get();
      if (!sessionId || inputOwnership === 'releasing') throw invalid('No input session is available to query.');
      const generation = ++inputGeneration;
      set({ inputAcceptingEvents: false, inputStatus: 'loading', inputError: null });
      try {
        const snapshot = inputStateSchema.parse(await input.sessionGet({ sessionId }));
        if (snapshot.sessionId !== sessionId || (snapshot.deviceId !== null && snapshot.deviceId !== deviceId)) throw invalid('Input snapshot identity differs from its owner.');
        if (generation === inputGeneration && get().inputSessionId === sessionId) {
          const previous = get().inputState;
          const advanced = canAdvanceInput(previous, snapshot);
          set({ inputState: advanced ? snapshot : previous, inputHistory: advanced ? [...get().inputHistory, snapshot] : get().inputHistory, inputStatus: 'ready', inputAcceptingEvents: true });
        }
        return snapshot;
      } catch (error) {
        if (generation === inputGeneration && get().inputSessionId === sessionId) {
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
      if (!state.inputAcceptingEvents || incoming.sessionId !== state.inputSessionId
        || (incoming.deviceId !== null && incoming.deviceId !== state.inputDeviceId)
        || !canAdvanceInput(state.inputState, incoming)) return false;
      set({ inputState: incoming, inputHistory: [...state.inputHistory, incoming] });
      return true;
    },
    stopInputSession: () => releaseInput(false),
    releaseInputForLaunch: () => releaseInput(true)
    };
  });
}

export const useCreatorStore = createCreatorStore();
