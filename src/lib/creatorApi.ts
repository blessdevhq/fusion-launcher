import { CREATOR_JOB_EVENT, INPUT_STATE_EVENT, creatorJobProgressSchema, inputStateSchema } from '../types/fusionCreator.ts';
import type { CreatorApi, FusionInputApi, CreatorJobProgress, InputState } from '../types/fusionCreator.ts';
import type { FusionError } from '../types/fusionSource.ts';
import { isTauriRuntime } from './runtime.ts';

export type CreatorUnlisten = () => void;
/** Injection is explicit: fixtures must never replace this production transport by runtime guessing. */
export interface CreatorTransport {
  invoke<T>(command: string, args?: { request: unknown }): Promise<T>;
  listen<T>(event: string, handler: (payload: T) => void): Promise<CreatorUnlisten>;
}

function unavailable(command: string): FusionError {
  return { code: 'action_unavailable', message: 'This Creator action requires the desktop backend.', details: { command } };
}

const desktopTransport: CreatorTransport = {
  async invoke<T>(command: string, args?: { request: unknown }): Promise<T> {
    if (!isTauriRuntime()) throw unavailable(command);
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<T>(command, args);
  },
  async listen<T>(event: string, handler: (payload: T) => void) {
    // An inert listener does not claim an action or input-handle release succeeded.
    if (!isTauriRuntime()) return () => undefined;
    const { listen } = await import('@tauri-apps/api/event');
    return listen<T>(event, ({ payload }) => handler(payload));
  }
};

export function createCreatorBridge(transport: CreatorTransport = desktopTransport) {
  const request = <T>(command: string, value: unknown): Promise<T> => transport.invoke<T>(command, { request: value });
  const creatorApi: CreatorApi = {
    projectList: () => transport.invoke('fusion_creator_project_list'),
    projectOpen: (value) => request('fusion_creator_project_open', value),
    projectSave: (value) => request('fusion_creator_project_save', value),
    scan: (value) => request('fusion_creator_scan', value),
    classify: (value) => request('fusion_creator_classify', value),
    build: (value) => request('fusion_creator_build', value),
    verify: (value) => request('fusion_creator_verify', value),
    export: (value) => request('fusion_creator_export', value),
    verifyHttps: (value) => request('fusion_creator_verify_https', value),
    jobGet: (value) => request('fusion_creator_job_get', value),
    jobCancel: (value) => request('fusion_creator_job_cancel', value),
    jobResume: (value) => request('fusion_creator_job_resume', value),
    releaseList: (value) => request('fusion_creator_release_list', value),
    reportGet: (value) => request('fusion_creator_report_get', value)
  };
  const fusionInputApi: FusionInputApi = {
    devices: () => transport.invoke('fusion_input_devices'),
    sessionStart: (value) => request('fusion_input_session_start', value),
    sessionStop: (value) => request('fusion_input_session_stop', value),
    sessionGet: (value) => request('fusion_input_session_get', value),
    bindingsSave: (value) => request('fusion_input_bindings_save', value),
    releaseForLaunch: (value) => request('fusion_input_release_for_launch', value)
  };
  return {
    creatorApi,
    fusionInputApi,
    subscribeCreatorJobProgress: (handler: (progress: CreatorJobProgress) => void) =>
      transport.listen<unknown>(CREATOR_JOB_EVENT, (payload) => {
        const result = creatorJobProgressSchema.safeParse(payload);
        if (result.success) handler(result.data);
      }),
    subscribeInputState: (handler: (state: InputState) => void) =>
      transport.listen<unknown>(INPUT_STATE_EVENT, (payload) => {
        const result = inputStateSchema.safeParse(payload);
        if (result.success) handler(result.data);
      })
  };
}

export const { creatorApi, fusionInputApi, subscribeCreatorJobProgress, subscribeInputState } = createCreatorBridge();
