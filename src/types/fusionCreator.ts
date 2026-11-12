import { z } from 'zod';
import { creatorProgramProjectSchema, programDescriptorSchema, preservingJsonRecord } from './fusionProgram.ts';
export { programDescriptorSchema, creatorProgramProjectSchema, programCreatorManifestSchema } from './fusionProgram.ts';
export type { ProgramDescriptor, CreatorProgramProject, ProgramCreatorManifest } from './fusionProgram.ts';

/** Portable source/project data has snake_case keys; IPC envelopes use camelCase. */
const id = z.string().refine((value) => {
  const length = [...value].length;
  return length >= 1 && length <= 128;
}, 'Identifier must contain 1..128 Unicode code points');
const hash = z.string().regex(/^[a-f0-9]{64}$/);
const sequence = z.number().int().positive().max(Number.MAX_SAFE_INTEGER);
const revision = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const https = z.string().url().refine((value) => {
  const url = new URL(value);
  return url.protocol === 'https:' && !url.username && !url.password;
});
export const adapterReferenceSchema = z.object({ id, version: id }).strict();
export const componentOriginSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('managed'), artifact_ref: id, redistribution_allowed: z.boolean() }).strict(),
  z.object({ kind: z.literal('external'), url: https, transport_blake3: hash, inventory_blake3: hash }).strict(),
  z.object({ kind: z.literal('user_provided'), requirement_id: id, inventory_blake3: hash, instructions: z.string() }).strict()
]);
export const buildProfileSchema = z.object({
  id, title: z.string(), layer_ids: z.array(id), controls_profile_ids: z.array(id)
}).strict();
export const logicalBindingSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('keyboard'), code: id }).strict(),
  z.object({ kind: z.literal('mouse_button'), button: z.number().int().nonnegative() }).strict(),
  z.object({ kind: z.literal('mouse_axis'), axis: z.enum(['x', 'y']), direction: z.enum(['positive', 'negative']) }).strict(),
  z.object({ kind: z.literal('controller_button'), button: id }).strict(),
  z.object({ kind: z.literal('controller_axis'), axis: id, direction: z.enum(['positive', 'negative']) }).strict()
]);
export const controlsProfileSchema = z.object({
  id, title: z.string(), device: z.enum(['keyboard_mouse', 'sdl_gamecontroller']),
  bindings: preservingJsonRecord(logicalBindingSchema)
}).strict();
/** One finite profile pair; applying it belongs to installer B, not this scaffold. */
export const creatorProfileSelectionSchema = z.object({
  buildProfileId: id, controlsProfileId: id
}).strict();
export type CreatorProfileSelection = z.infer<typeof creatorProfileSelectionSchema>;
export const projectionRuleSchema = z.object({ path: z.string().min(1), winning_layer: id }).strict();
export type ProjectionRule = z.infer<typeof projectionRuleSchema>;
export const creatorManifestSchema = z.object({
  adapter: adapterReferenceSchema,
  components: z.array(z.object({ layer_id: id, origin: componentOriginSchema }).strict()),
  build_profiles: z.array(buildProfileSchema), controls_profiles: z.array(controlsProfileSchema),
  projection_rules: z.array(projectionRuleSchema)
}).strict();
/** Legacy 1.0 validation only; not an IPC/storage carrier or an implicit migration path. */
export const creatorProjectSchema = z.object({
  schema_version: z.literal('1.0'), project_id: id, revision, title: z.string(),
  source_id: id, game_id: id, game_version: id,
  adapter: adapterReferenceSchema,
  components: z.array(z.object({ id, layer_id: id, origin: componentOriginSchema }).strict()),
  build_profiles: z.array(buildProfileSchema), controls_profiles: z.array(controlsProfileSchema),
  projection_rules: z.array(projectionRuleSchema)
}).strict();
export type ComponentOrigin = z.infer<typeof componentOriginSchema>;
export type CreatorProject = z.infer<typeof creatorProjectSchema>;
export type CreatorManifest = z.infer<typeof creatorManifestSchema>;
export type BuildProfile = z.infer<typeof buildProfileSchema>;
export type ControlsProfile = z.infer<typeof controlsProfileSchema>;
export type LogicalBinding = z.infer<typeof logicalBindingSchema>;
export type AdapterReference = z.infer<typeof adapterReferenceSchema>;

export const CREATOR_JOB_EVENT = 'fusion-creator:job-progress' as const;
export const INPUT_STATE_EVENT = 'fusion-input:state' as const;
export const CREATOR_COMMANDS = [
  'fusion_creator_project_list', 'fusion_creator_project_open', 'fusion_creator_project_save',
  'fusion_creator_scan', 'fusion_creator_classify', 'fusion_creator_build', 'fusion_creator_verify',
  'fusion_creator_export', 'fusion_creator_verify_https', 'fusion_creator_job_get',
  'fusion_creator_job_cancel', 'fusion_creator_job_resume', 'fusion_creator_release_list',
  'fusion_creator_report_get'
] as const;
export const INPUT_COMMANDS = [
  'fusion_input_devices', 'fusion_input_session_start', 'fusion_input_session_stop',
  'fusion_input_session_get', 'fusion_input_bindings_save', 'fusion_input_release_for_launch'
] as const;
export type CreatorJobKind = 'scan' | 'classify' | 'build' | 'verify' | 'export' | 'verify_https';
export type CreatorJobStatus = 'queued' | 'running' | 'cancelling' | 'cancelled' | 'interrupted' | 'completed' | 'failed';
const fusionErrorSchema = z.object({ code: z.string(), message: z.string(), details: z.record(z.unknown()) }).strict();
export const creatorJobProgressSchema = z.object({
  jobId: id, projectId: id, projectRevision: revision, sequence,
  kind: z.enum(['scan', 'classify', 'build', 'verify', 'export', 'verify_https']),
  status: z.enum(['queued', 'running', 'cancelling', 'cancelled', 'interrupted', 'completed', 'failed']),
  phase: z.string(), completedUnits: revision, totalUnits: revision.nullable(),
  checkpointId: id.nullable(), releaseId: id.nullable(), error: fusionErrorSchema.nullable()
}).strict();
export type CreatorJobProgress = z.infer<typeof creatorJobProgressSchema>;
export interface CreatorJobReceipt { jobId: string; projectId: string; projectRevision: number }
export type CreatorCheckpointBoundary = 'scan_classification_saved' | 'artifact_committed' | 'release_built' | 'verification_completed' | 'export_completed';
export const creatorResumeStampSchema = z.object({
  projectRevision: revision, inputHashes: preservingJsonRecord(hash), executor: programDescriptorSchema.shape.executor
}).strict();
export type CreatorResumeStamp = z.infer<typeof creatorResumeStampSchema>;
/** A checkpoint references an already verified, atomic result, never an unfinished file. */
export interface CreatorCheckpoint {
  checkpointId: string; jobId: string; boundary: CreatorCheckpointBoundary;
  stamp: CreatorResumeStamp; resultBlake3: string; artifactId: string | null;
}
export interface CreatorJob {
  progress: CreatorJobProgress; checkpoints: CreatorCheckpoint[]; reportIds: string[];
}
export type CreatorReportKind = 'structure_integrity' | 'clean_install' | 'observed_launch' | 'manual_scenario' | 'https_acquisition';
export const creatorReportStatusSchema = z.enum(['passed', 'failed', 'not_tested']);
export const creatorReportSchema = z.object({
  reportId: id, releaseId: id, releaseBlake3: hash, projectRevision: revision,
  kind: z.enum(['structure_integrity', 'clean_install', 'observed_launch', 'manual_scenario', 'https_acquisition']),
  status: creatorReportStatusSchema, executor: programDescriptorSchema.shape.executor,
  buildProfileId: id, controlsProfileId: id, environment: preservingJsonRecord(z.string()),
  diagnostics: z.array(fusionErrorSchema)
}).strict();
export type CreatorReport = z.infer<typeof creatorReportSchema>;
export interface CreatorRelease {
  releaseId: string; projectId: string; projectRevision: number; manifestBlake3: string;
  artifactHashes: Record<string, string>; reportIds: string[];
}
/** Local IPC/storage only. Never part of a portable project, release or Source export. */
export const creatorPrivateBindingsSchema = z.object({
  projectId: id, componentPaths: preservingJsonRecord(z.string()), signingKeyRef: z.string().nullable()
}).strict();
/** Same revision/bindings envelope as project_storage::SavedPrivateBindings. */
export const savedPrivateBindingsSchema = z.object({ revision, bindings: creatorPrivateBindingsSchema }).strict();
export const creatorProjectOpenResponseSchema = z.object({
  project: creatorProgramProjectSchema, privateBindings: savedPrivateBindingsSchema.nullable(), missingInputs: z.array(id)
}).strict();
export const creatorProjectSummarySchema = z.object({
  projectId: id, title: z.string(), revision, verificationStatus: creatorReportStatusSchema,
  modifiedAt: revision // UTC Unix epoch milliseconds, not a formatted local date.
}).strict();
export const creatorProjectListResponseSchema = z.array(creatorProjectSummarySchema);
export const creatorProjectRequestSchema = z.object({ projectId: id }).strict();
export type CreatorPrivateBindings = z.infer<typeof creatorPrivateBindingsSchema>;
export type SavedPrivateBindings = z.infer<typeof savedPrivateBindingsSchema>;
export type CreatorProjectOpenResponse = z.infer<typeof creatorProjectOpenResponseSchema>;
export type CreatorProjectSummary = z.infer<typeof creatorProjectSummarySchema>;
export type CreatorProjectListResponse = z.infer<typeof creatorProjectListResponseSchema>;
export type CreatorProjectRequest = z.infer<typeof creatorProjectRequestSchema>;
/** null is insert-only; n updates n to the supplied project.revision = n + 1. */
export const creatorProjectSaveRequestSchema = z.object({
  project: creatorProgramProjectSchema, expectedRevision: revision.nullable()
}).strict();
export const creatorProjectSaveResponseSchema = z.object({ revision }).strict();
export type CreatorProjectSaveRequest = z.infer<typeof creatorProjectSaveRequestSchema>;
export type CreatorProjectSaveResponse = z.infer<typeof creatorProjectSaveResponseSchema>;
/** IDs reference program_descriptor; export never embeds private project data. */
export const creatorJobStartRequestSchema = z.object({
  projectId: id, projectRevision: revision, buildProfileIds: z.array(id), controlsProfileIds: z.array(id),
  releaseId: id.nullable(), targetDirectory: z.string().nullable(), sourceUrl: z.string().nullable()
}).strict();
export type CreatorJobStartRequest = z.infer<typeof creatorJobStartRequestSchema>;
export interface CreatorJobRequest { jobId: string }
export interface CreatorReportRequest { reportId: string }
export interface InputDevice {
  deviceId: string; name: string; connected: boolean; mappingKnown: boolean;
}
export interface InputPrivateIdentity {
  guid: string; path: string | null; serial: string | null; port: number | null;
}
export type RawInputBinding =
  | { kind: 'button'; index: number }
  | { kind: 'hat'; index: number; mask: number }
  | { kind: 'axis'; index: number; direction: 'positive' | 'negative' };
export const inputStateSchema = z.object({
  sessionId: id, sequence, deviceId: id.nullable(),
  status: z.enum(['active', 'disconnected', 'stopping', 'stopped', 'failed']),
  buttons: z.record(z.boolean()), axes: z.record(z.number().min(-1).max(1)),
  error: fusionErrorSchema.nullable()
}).strict();
export type InputState = z.infer<typeof inputStateSchema>;
export interface InputSessionRequest { sessionId: string }
export interface InputSessionStartRequest { deviceId: string }
export interface InputSessionReceipt { sessionId: string }
export interface InputReleaseReceipt { sessionId: string; handlesClosed: true }
export interface InputBindingsReceipt { sessionId: string; controlsProfileId: string }
export interface InputBindingsSaveRequest {
  sessionId: string; controlsProfileId: string; bindings: Record<string, LogicalBinding>;
  deadzones: Record<string, number>; rawBindings: Record<string, RawInputBinding>;
}

/** Signatures only: production bridge may reject with action_unavailable during wave zero. */
export interface CreatorApi {
  projectList(): Promise<CreatorProjectListResponse>;
  projectOpen(request: CreatorProjectRequest): Promise<CreatorProjectOpenResponse>;
  projectSave(request: CreatorProjectSaveRequest): Promise<CreatorProjectSaveResponse>;
  scan(request: CreatorJobStartRequest): Promise<CreatorJobReceipt>;
  classify(request: CreatorJobStartRequest): Promise<CreatorJobReceipt>;
  build(request: CreatorJobStartRequest): Promise<CreatorJobReceipt>;
  verify(request: CreatorJobStartRequest): Promise<CreatorJobReceipt>;
  export(request: CreatorJobStartRequest): Promise<CreatorJobReceipt>;
  verifyHttps(request: CreatorJobStartRequest): Promise<CreatorJobReceipt>;
  jobGet(request: CreatorJobRequest): Promise<CreatorJob>;
  jobCancel(request: CreatorJobRequest): Promise<CreatorJob>;
  jobResume(request: CreatorJobRequest): Promise<CreatorJobReceipt>;
  releaseList(request: CreatorProjectRequest): Promise<CreatorRelease[]>;
  reportGet(request: CreatorReportRequest): Promise<CreatorReport>;
}
export interface FusionInputApi {
  devices(): Promise<InputDevice[]>;
  sessionStart(request: InputSessionStartRequest): Promise<InputSessionReceipt>;
  sessionStop(request: InputSessionRequest): Promise<InputReleaseReceipt>;
  sessionGet(request: InputSessionRequest): Promise<InputState>;
  bindingsSave(request: InputBindingsSaveRequest): Promise<InputBindingsReceipt>;
  releaseForLaunch(request: InputSessionRequest): Promise<InputReleaseReceipt>;
}
