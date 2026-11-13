import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import * as creator from './fusionCreator.ts';
import {
  creatorProjectSchema, creatorJobProgressSchema, inputStateSchema,
  CREATOR_COMMANDS, INPUT_COMMANDS, CREATOR_JOB_EVENT, INPUT_STATE_EVENT
} from './fusionCreator.ts';

const fixture = JSON.parse(readFileSync(new URL('../../fixtures/creator/contracts.json', import.meta.url), 'utf8'));

const descriptor = JSON.parse(readFileSync(new URL('../../fixtures/creator/programs/supertuxkart-1.5.json', import.meta.url), 'utf8'));
const programProject = {
  schema_version: '1.1', project_id: 'fixture-project', revision: 1, title: descriptor.identity.title,
  source_id: 'fixture-source', game_id: descriptor.identity.id, game_version: descriptor.identity.version,
  program_descriptor: descriptor
};

test('project save uses the strict 1.1 carrier and a required nullable expected revision', () => {
  const request = { project: programProject, expectedRevision: null };
  assert.deepEqual(creator.creatorProjectSaveRequestSchema.parse(request), request);
  assert.deepEqual(creator.creatorProjectSaveRequestSchema.parse({ ...request, expectedRevision: 0 }), { ...request, expectedRevision: 0 });
  assert.deepEqual(creator.creatorProjectSaveResponseSchema.parse({ revision: 1 }), { revision: 1 });
  assert.equal(creator.creatorProjectSaveRequestSchema.safeParse({ project: programProject }).success, false);
  assert.equal(creator.creatorProjectSaveRequestSchema.safeParse({ ...request, project: { ...programProject, schema_version: '1.0' } }).success, false);
  for (const field of ['adapter', 'components', 'build_profiles', 'controls_profiles', 'projection_rules']) {
    assert.equal(creator.creatorProjectSaveRequestSchema.safeParse({ ...request, project: { ...programProject, [field]: [] } }).success, false, field);
  }
  for (const expectedRevision of [-1, 0.5, Number.MAX_SAFE_INTEGER + 1]) {
    assert.equal(creator.creatorProjectSaveRequestSchema.safeParse({ ...request, expectedRevision }).success, false);
  }
});

test('local project open envelopes retain private bindings separately from portable projects', () => {
  const bindings = { projectId: programProject.project_id, componentPaths: { [descriptor.components[0].id]: 'C:/private/stk' }, signingKeyRef: null };
  const saved = { revision: 2, bindings };
  const bound = { project: programProject, privateBindings: saved, missingInputs: [] };
  const unbound = { project: programProject, privateBindings: null, missingInputs: [descriptor.components[0].id] };
  assert.deepEqual(creator.creatorPrivateBindingsSchema.parse(bindings), bindings);
  assert.deepEqual(creator.savedPrivateBindingsSchema.parse(saved), saved);
  assert.deepEqual(creator.creatorProjectOpenResponseSchema.parse(bound), bound);
  assert.deepEqual(creator.creatorProjectOpenResponseSchema.parse(unbound), unbound);
  assert.equal(creator.creatorProjectOpenResponseSchema.safeParse({ ...unbound, missingInputs: [{ componentId: descriptor.components[0].id }] }).success, false);
  assert.equal(creator.creatorProgramProjectSchema.safeParse({ ...programProject, privateBindings: saved }).success, false);
  assert.equal(creator.creatorProjectSaveRequestSchema.safeParse({ project: programProject, expectedRevision: null, privateBindings: saved }).success, false);
  assert.equal(creator.programCreatorManifestSchema.safeParse({ program_descriptor: descriptor, privateBindings: saved }).success, false);
  const paths = JSON.parse('{"__proto__":"C:/private/prototype","constructor":"C:/private/constructor"}');
  assert.deepEqual(creator.creatorPrivateBindingsSchema.parse({ ...bindings, componentPaths: paths }).componentPaths, paths);
});

test('project summaries use bounded epoch milliseconds and a finite verification status', () => {
  const summary = { projectId: programProject.project_id, title: programProject.title, revision: 1, verificationStatus: 'not_tested', modifiedAt: 0 };
  assert.deepEqual(creator.creatorProjectSummarySchema.parse(summary), summary);
  for (const modifiedAt of [-1, 0.5, Number.MAX_SAFE_INTEGER + 1, Infinity, NaN, '0']) {
    assert.equal(creator.creatorProjectSummarySchema.safeParse({ ...summary, modifiedAt }).success, false);
  }
  for (const verificationStatus of ['passed', 'failed', 'not_tested']) {
    assert.equal(creator.creatorProjectSummarySchema.safeParse({ ...summary, verificationStatus, modifiedAt: Number.MAX_SAFE_INTEGER }).success, true);
  }
  assert.equal(creator.creatorProjectSummarySchema.safeParse({ ...summary, verificationStatus: 'completed' }).success, false);
});

test('Creator portable project and job/input wire fixtures satisfy strict contracts', () => {
  assert.deepEqual(creator.creatorProgramProjectSchema.parse(fixture.project), fixture.project);
  assert.deepEqual(creatorProjectSchema.parse(fixture.legacyProject), fixture.legacyProject);
  assert.equal(creator.creatorProjectSaveRequestSchema.safeParse({ project: fixture.legacyProject, expectedRevision: 1 }).success, false);
  assert.deepEqual(creatorJobProgressSchema.parse(fixture.job), fixture.job);
  assert.deepEqual(inputStateSchema.parse(fixture.input), fixture.input);
});

test('portable component origins exclude local bindings and require pinned external inputs', () => {
  const project = structuredClone(fixture.project);
  project.program_descriptor.components[0].localPath = 'C:/private/runtime';
  assert.equal(creator.creatorProgramProjectSchema.safeParse(project).success, false);
  const external = structuredClone(fixture.project);
  external.program_descriptor.components[0].origin = { kind: 'external', url: 'https://example.com/runtime.zip' };
  assert.equal(creator.creatorProgramProjectSchema.safeParse(external).success, false);
});

test('jobs, reports and export use descriptor executor and profile references, not legacy project fields', () => {
  assert.deepEqual(creator.creatorResumeStampSchema.parse(fixture.resumeStamp), fixture.resumeStamp);
  assert.deepEqual(creator.creatorReportSchema.parse(fixture.report), fixture.report);
  assert.deepEqual(creator.creatorJobStartRequestSchema.parse(fixture.jobStart), fixture.jobStart);
  for (const [schema, value] of [
    [creator.creatorResumeStampSchema, fixture.resumeStamp],
    [creator.creatorReportSchema, fixture.report]
  ] as const) {
    const { executor: _executor, ...legacy } = value;
    assert.equal(schema.safeParse({ ...legacy, adapter: fixture.legacyProject.adapter }).success, false);
    assert.equal(schema.safeParse({ ...value, adapter: fixture.legacyProject.adapter }).success, false);
    assert.equal(schema.safeParse({ ...value, executor: { id: 'supertuxkart', version: '1.5' } }).success, false);
  }
  for (const field of ['adapter', 'components', 'build_profiles', 'controls_profiles', 'projection_rules', 'privateBindings']) {
    assert.equal(creator.creatorJobStartRequestSchema.safeParse({ ...fixture.jobStart, [field]: [] }).success, false, field);
  }
  assert.ok(fixture.project.program_descriptor.build_profiles.some((profile: { id: string }) => profile.id === fixture.report.buildProfileId));
  assert.ok(fixture.project.program_descriptor.controls_profiles.some((profile: { id: string }) => profile.id === fixture.report.controlsProfileId));
});

test('progress always identifies its job or session and a positive safe sequence', () => {
  for (const sequence of [0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
    assert.equal(creatorJobProgressSchema.safeParse({ ...fixture.job, sequence }).success, false);
    assert.equal(inputStateSchema.safeParse({ ...fixture.input, sequence }).success, false);
  }
  const { jobId: _jobId, ...withoutJob } = fixture.job;
  const { sessionId: _sessionId, ...withoutSession } = fixture.input;
  assert.equal(creatorJobProgressSchema.safeParse(withoutJob).success, false);
  assert.equal(inputStateSchema.safeParse(withoutSession).success, false);
});

test('Creator and input IPC namespaces and event names are disjoint and stable', () => {
  assert.equal(CREATOR_JOB_EVENT, 'fusion-creator:job-progress');
  assert.equal(INPUT_STATE_EVENT, 'fusion-input:state');
  assert.ok(CREATOR_COMMANDS.every((name) => name.startsWith('fusion_creator_')));
  assert.ok(INPUT_COMMANDS.every((name) => name.startsWith('fusion_input_')));
  assert.equal(new Set([...CREATOR_COMMANDS, ...INPUT_COMMANDS]).size, CREATOR_COMMANDS.length + INPUT_COMMANDS.length);
});

test('every frozen Creator/input command is declared and registered in Rust', () => {
  const handlers = readFileSync(new URL('../../src-tauri/src/commands/fusion_creator.rs', import.meta.url), 'utf8');
  const registration = readFileSync(new URL('../../src-tauri/src/lib.rs', import.meta.url), 'utf8');
  const declared = [...handlers.matchAll(/unavailable_command!\((fusion_(?:creator|input)_\w+)/g)].map((match) => match[1]);
  const registered = [...registration.matchAll(/commands::fusion_creator::(fusion_(?:creator|input)_\w+)/g)].map((match) => match[1]);
  const expected = [...CREATOR_COMMANDS, ...INPUT_COMMANDS].sort();
  assert.deepEqual(declared.sort(), expected);
  assert.deepEqual(registered.sort(), expected);
  const rustCreator = readFileSync(new URL('../../src-tauri/src/fusion_creator/mod.rs', import.meta.url), 'utf8');
  const rustInput = readFileSync(new URL('../../src-tauri/src/fusion_creator/input.rs', import.meta.url), 'utf8');
  assert.deepEqual([...rustCreator.matchAll(/"(fusion_creator_\w+)"/g)].map((match) => match[1]).sort(), [...CREATOR_COMMANDS].sort());
  assert.deepEqual([...rustInput.matchAll(/"(fusion_input_\w+)"/g)].map((match) => match[1]).sort(), [...INPUT_COMMANDS].sort());
  assert.ok(rustCreator.includes(`"${CREATOR_JOB_EVENT}"`));
  assert.ok(rustInput.includes(`"${INPUT_STATE_EVENT}"`));
});
