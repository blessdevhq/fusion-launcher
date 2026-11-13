import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import * as creator from './fusionCreator.ts';
import {
  adapterReferenceSchema, buildProfileSchema, componentOriginSchema,
  controlsProfileSchema, creatorJobProgressSchema, creatorManifestSchema,
  creatorProjectSchema, inputStateSchema, logicalBindingSchema, projectionRuleSchema,
  creatorProfileSelectionSchema
} from './fusionCreator.ts';

interface SchemaObject {
  $ref?: string;
  $defs?: Record<string, Schema>;
  oneOf?: Schema[];
  anyOf?: Schema[];
  type?: string;
  const?: unknown;
  enum?: unknown[];
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
  pattern?: string;
  format?: string;
  properties?: Record<string, Schema>;
  required?: string[];
  additionalProperties?: Schema;
  items?: Schema;
  [keyword: string]: unknown;
}
type Schema = boolean | SchemaObject;
const schema = JSON.parse(readFileSync(new URL('../../docs/schemas/fusion-creator-contracts.schema.json', import.meta.url), 'utf8')) as SchemaObject;
const fixture = JSON.parse(readFileSync(new URL('../../fixtures/creator/contracts.json', import.meta.url), 'utf8'));
const definition = (name: string): Schema => {
  assert.ok(schema.$defs && Object.hasOwn(schema.$defs, name), `missing definition: ${name}`);
  return schema.$defs[name];
};

// Test-only interpreter, NOT a general JSON Schema implementation. Audit every
// keyword below so a future schema change cannot silently bypass validation.
const supported = new Set([
  '$schema', '$id', '$defs', '$ref', 'title', 'description', '$comment',
  'oneOf', 'anyOf', 'type', 'const', 'enum', 'minimum', 'maximum',
  'minLength', 'maxLength', 'pattern', 'format', 'properties', 'required',
  'additionalProperties', 'items', 'minItems', 'maxItems', 'uniqueItems'
]);
function audit(node: Schema): void {
  if (typeof node === 'boolean') return;
  for (const key of Object.keys(node)) assert.ok(supported.has(key), `unsupported schema keyword: ${key}`);
  if (node.$ref) {
    assert.match(node.$ref, /^#\/\$defs\/[^/]+$/);
    definition(node.$ref.slice('#/$defs/'.length));
  }
  if (node.type) assert.ok(['object', 'array', 'string', 'number', 'integer', 'boolean', 'null'].includes(node.type));
  if (node.format) assert.equal(node.format, 'uri');
  if (node.pattern) new RegExp(node.pattern, 'u');
  if (node.type === 'object') {
    assert.notEqual(node.additionalProperties, undefined, 'every object must declare its additional-property policy');
    if (node.properties) {
      assert.equal(node.additionalProperties, false, 'DTO objects must reject unknown properties');
      assert.deepEqual([...(node.required ?? [])].sort(), Object.keys(node.properties).sort(), 'all DTO fields, including nullable fields, are required');
    }
  }
  for (const child of Object.values(node.$defs ?? {})) audit(child);
  for (const child of Object.values(node.properties ?? {})) audit(child);
  for (const child of [...(node.oneOf ?? []), ...(node.anyOf ?? [])]) audit(child);
  if (node.items !== undefined) audit(node.items);
  if (node.additionalProperties !== undefined) audit(node.additionalProperties);
}
function matches(node: Schema, value: unknown): boolean {
  if (typeof node === 'boolean') return node;
  if (node.$ref && !matches(definition(node.$ref.slice('#/$defs/'.length)), value)) return false;
  if (node.oneOf && node.oneOf.filter((branch) => matches(branch, value)).length !== 1) return false;
  if (node.anyOf && !node.anyOf.some((branch) => matches(branch, value))) return false;
  if (Object.hasOwn(node, 'const') && JSON.stringify(node.const) !== JSON.stringify(value)) return false;
  if (node.enum && !node.enum.some((entry) => JSON.stringify(entry) === JSON.stringify(value))) return false;
  if (node.type) {
    const correctType = node.type === 'null' ? value === null
      : node.type === 'array' ? Array.isArray(value)
        : node.type === 'object' ? value !== null && typeof value === 'object' && !Array.isArray(value)
          : node.type === 'integer' ? typeof value === 'number' && Number.isInteger(value)
            : node.type === 'number' ? typeof value === 'number' && Number.isFinite(value)
              : typeof value === node.type;
    if (!correctType) return false;
  }
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) return false;
    if (node.minimum !== undefined && value < node.minimum) return false;
    if (node.maximum !== undefined && value > node.maximum) return false;
  }
  if (typeof value === 'string') {
    const length = [...value].length; // JSON Schema counts Unicode code points.
    if (node.minLength !== undefined && length < node.minLength) return false;
    if (node.maxLength !== undefined && length > node.maxLength) return false;
    if (node.pattern && !new RegExp(node.pattern, 'u').test(value)) return false;
    if (node.format === 'uri') {
      try { new URL(value); } catch { return false; }
      if (/\s/u.test(value)) return false;
    }
  }
  if (Array.isArray(value)) {
    if (typeof node.minItems === 'number' && value.length < node.minItems) return false;
    if (typeof node.maxItems === 'number' && value.length > node.maxItems) return false;
    if (node.uniqueItems === true && value.some((entry, i) => value.slice(0, i).some((other) => JSON.stringify(entry) === JSON.stringify(other)))) return false;
    if (node.items !== undefined && !value.every((entry) => matches(node.items!, entry))) return false;
  }
  if (value !== null && typeof value === 'object' && !Array.isArray(value)) {
    const object = value as Record<string, unknown>;
    if (node.required?.some((key) => !Object.hasOwn(object, key))) return false;
    for (const [key, entry] of Object.entries(object)) {
      if (node.properties && Object.hasOwn(node.properties, key)) {
        if (!matches(node.properties[key], entry)) return false;
      } else if (node.additionalProperties !== undefined && !matches(node.additionalProperties, entry)) return false;
    }
  }
  return true;
}
const accepts = (name: string, value: unknown): boolean => matches(definition(name), value);
audit(schema);

const openResponse = { project: fixture.project, privateBindings: fixture.projectBindings, missingInputs: [] };
for (const [name, field, zod, withId] of [
  ['CreatorProgramProject', 'project_id', creator.creatorProgramProjectSchema,
    (id: string) => ({ ...fixture.project, project_id: id })],
  ['ProgramDescriptor', 'identity.id', creator.programDescriptorSchema,
    (id: string) => ({ ...fixture.project.program_descriptor, identity: { ...fixture.project.program_descriptor.identity, id } })],
  ['CreatorProjectSaveRequest', 'project.project_id', creator.creatorProjectSaveRequestSchema,
    (id: string) => ({ project: { ...fixture.project, project_id: id }, expectedRevision: null })],
  ['CreatorProjectRequest', 'projectId', creator.creatorProjectRequestSchema,
    (id: string) => ({ projectId: id })],
  ['CreatorPrivateBindings', 'projectId', creator.creatorPrivateBindingsSchema,
    (id: string) => ({ ...fixture.projectBindings.bindings, projectId: id })],
  ['SavedPrivateBindings', 'bindings.projectId', creator.savedPrivateBindingsSchema,
    (id: string) => ({ ...fixture.projectBindings, bindings: { ...fixture.projectBindings.bindings, projectId: id } })],
  ['CreatorProjectOpenResponse', 'project.project_id', creator.creatorProjectOpenResponseSchema,
    (id: string) => ({ ...openResponse, project: { ...fixture.project, project_id: id } })],
  ['CreatorProjectOpenResponse', 'privateBindings.bindings.projectId', creator.creatorProjectOpenResponseSchema,
    (id: string) => ({ ...openResponse, privateBindings: { ...fixture.projectBindings, bindings: { ...fixture.projectBindings.bindings, projectId: id } } })],
  ['CreatorProjectOpenResponse', 'missingInputs[]', creator.creatorProjectOpenResponseSchema,
    (id: string) => ({ ...openResponse, missingInputs: [id] })],
  ['CreatorProjectSummary', 'projectId', creator.creatorProjectSummarySchema,
    (id: string) => ({ ...fixture.projectSummary, projectId: id })],
  ['CreatorProjectListResponse', '[].projectId', creator.creatorProjectListResponseSchema,
    (id: string) => [{ ...fixture.projectSummary, projectId: id }]],
  ['CreatorJobStartRequest', 'projectId', creator.creatorJobStartRequestSchema,
    (id: string) => ({ ...fixture.jobStart, projectId: id })],
  ['CreatorJobStartRequest', 'buildProfileIds[]', creator.creatorJobStartRequestSchema,
    (id: string) => ({ ...fixture.jobStart, buildProfileIds: [id] })],
  ['CreatorJobStartRequest', 'controlsProfileIds[]', creator.creatorJobStartRequestSchema,
    (id: string) => ({ ...fixture.jobStart, controlsProfileIds: [id] })],
  ['CreatorJobStartRequest', 'releaseId', creator.creatorJobStartRequestSchema,
    (id: string) => ({ ...fixture.jobStart, releaseId: id })],
  ['CreatorJobProgress', 'jobId', creatorJobProgressSchema,
    (id: string) => ({ ...fixture.job, jobId: id })],
  ['CreatorJobProgress', 'projectId', creatorJobProgressSchema,
    (id: string) => ({ ...fixture.job, projectId: id })],
  ['CreatorJobProgress', 'checkpointId', creatorJobProgressSchema,
    (id: string) => ({ ...fixture.job, checkpointId: id })],
  ['CreatorJobProgress', 'releaseId', creatorJobProgressSchema,
    (id: string) => ({ ...fixture.job, releaseId: id })],
  ['CreatorReport', 'reportId', creator.creatorReportSchema,
    (id: string) => ({ ...fixture.report, reportId: id })],
  ['CreatorReport', 'releaseId', creator.creatorReportSchema,
    (id: string) => ({ ...fixture.report, releaseId: id })],
  ['CreatorReport', 'buildProfileId', creator.creatorReportSchema,
    (id: string) => ({ ...fixture.report, buildProfileId: id })],
  ['CreatorReport', 'controlsProfileId', creator.creatorReportSchema,
    (id: string) => ({ ...fixture.report, controlsProfileId: id })],
  ['CreatorProfileSelection', 'buildProfileId', creatorProfileSelectionSchema,
    (id: string) => ({ ...fixture.profileSelection, buildProfileId: id })],
  ['CreatorProfileSelection', 'controlsProfileId', creatorProfileSelectionSchema,
    (id: string) => ({ ...fixture.profileSelection, controlsProfileId: id })]
] as const) {
  test(`Unicode code-point identifier bounds: ${name}.${field}`, () => {
    for (const [length, expected] of [[0, false], [1, true], [65, true], [128, true], [129, false]] as const) {
      const value = withId('🎮'.repeat(length));
      const label = `${name}.${field}: ${length} code points`;
      assert.equal(accepts(name, value), expected, `JSON Schema ${label}`);
      assert.equal(zod.safeParse(value).success, expected, `Zod ${label}`);
    }
  });
}

test('project save JSON definitions accept 1.1 and reject legacy carriers and duplicate fields', () => {
  const program = JSON.parse(readFileSync(new URL('../../fixtures/creator/programs/supertuxkart-1.5.json', import.meta.url), 'utf8'));
  const project = {
    schema_version: '1.1', project_id: 'fixture-project', revision: 1, title: program.identity.title,
    source_id: 'fixture-source', game_id: program.identity.id, game_version: program.identity.version,
    program_descriptor: program
  };
  const request = { project, expectedRevision: null };
  assert.equal(accepts('CreatorProjectSaveRequest', request), true);
  assert.equal(accepts('CreatorProjectSaveResponse', { revision: 1 }), true);
  assert.equal(accepts('CreatorProjectSaveRequest', { project }), false);
  assert.equal(accepts('CreatorProjectSaveRequest', { ...request, expectedRevision: 0 }), true);
  assert.equal(accepts('CreatorProjectSaveRequest', { ...request, project: { ...project, schema_version: '1.0' } }), false);
  for (const field of ['adapter', 'components', 'build_profiles', 'controls_profiles', 'projection_rules']) {
    assert.equal(accepts('CreatorProjectSaveRequest', { ...request, project: { ...project, [field]: [] } }), false, field);
  }
});

test('local open, private binding and summary definitions match strict Zod contracts', () => {
  const program = JSON.parse(readFileSync(new URL('../../fixtures/creator/programs/supertuxkart-1.5.json', import.meta.url), 'utf8'));
  const project = {
    schema_version: '1.1', project_id: 'fixture-project', revision: 1, title: program.identity.title,
    source_id: 'fixture-source', game_id: program.identity.id, game_version: program.identity.version,
    program_descriptor: program
  };
  const bindings = { projectId: project.project_id, componentPaths: JSON.parse('{"__proto__":"C:/private/stk"}'), signingKeyRef: null };
  const saved = { revision: 2, bindings };
  const cases = [
    ['CreatorPrivateBindings', creator.creatorPrivateBindingsSchema, bindings],
    ['SavedPrivateBindings', creator.savedPrivateBindingsSchema, saved],
    ['CreatorProjectOpenResponse', creator.creatorProjectOpenResponseSchema, { project, privateBindings: saved, missingInputs: [] }],
    ['CreatorProjectOpenResponse', creator.creatorProjectOpenResponseSchema, { project, privateBindings: null, missingInputs: [program.components[0].id] }],
    ['CreatorProjectSummary', creator.creatorProjectSummarySchema, { projectId: project.project_id, title: project.title, revision: 1, verificationStatus: 'not_tested', modifiedAt: 0 }]
  ] as const;
  for (const [name, zod, value] of cases) {
    assert.equal(accepts(name, value), true, name);
    assert.deepEqual(zod.parse(value), value);
    assert.equal(matches(schema, value), false, `${name}: local envelopes are not portable roots`);
    assert.equal(accepts(name, { ...value, unknown: true }), false);
    assert.equal(zod.safeParse({ ...value, unknown: true }).success, false);
    for (const key of Object.keys(value)) {
      const missing: Record<string, unknown> = { ...value };
      delete missing[key];
      assert.equal(accepts(name, missing), false, `${name}: missing ${key}`);
      assert.equal(zod.safeParse(missing).success, false, `${name}: missing ${key}`);
    }
  }
  const summary = cases[4][2];
  for (const modifiedAt of [-1, 0.5, Number.MAX_SAFE_INTEGER + 1, Infinity, NaN, '0']) {
    assert.equal(accepts('CreatorProjectSummary', { ...summary, modifiedAt }), false);
    assert.equal(creator.creatorProjectSummarySchema.safeParse({ ...summary, modifiedAt }).success, false);
  }
  for (const verificationStatus of ['passed', 'failed', 'not_tested']) {
    const value = { ...summary, verificationStatus, modifiedAt: Number.MAX_SAFE_INTEGER };
    assert.equal(accepts('CreatorProjectSummary', value), true);
    assert.equal(creator.creatorProjectSummarySchema.safeParse(value).success, true);
  }
  assert.equal(accepts('CreatorProjectSummary', { ...summary, verificationStatus: 'completed' }), false);
});

test('program descriptor v1 is a separate versioned public contract, not an adapter name', () => {
  const descriptor = definition('ProgramDescriptor') as SchemaObject;
  assert.equal(descriptor.type, 'object');
  assert.deepEqual(descriptor.properties?.document_type, { const: 'program_descriptor' });
  assert.deepEqual(descriptor.properties?.schema_version, { const: '1.0' });
  assert.ok(schema.oneOf?.some((root) => typeof root !== 'boolean' && root.$ref === '#/$defs/ProgramDescriptor'));
});

test('program authoring and signed manifest carriers are explicit, not extra legacy DTO fields', () => {
  const project = definition('CreatorProgramProject') as SchemaObject;
  assert.deepEqual(project.properties?.schema_version, { const: '1.1' });
  assert.deepEqual(project.properties?.program_descriptor, { $ref: '#/$defs/ProgramDescriptor' });
  const extension = definition('ProgramCreatorManifest') as SchemaObject;
  assert.deepEqual(extension.required, ['program_descriptor']);
});

test('Creator JSON Schema publishes draft 2020-12 and the five public roots', () => {
  assert.equal(schema.$schema, 'https://json-schema.org/draft/2020-12/schema');
  assert.deepEqual(schema.oneOf, [
    { $ref: '#/$defs/CreatorProject' },
    { $ref: '#/$defs/CreatorJobProgress' },
    { $ref: '#/$defs/InputState' },
    { $ref: '#/$defs/ProgramDescriptor' },
    { $ref: '#/$defs/CreatorProgramProject' }
  ]);
});

test('shared Rust/TypeScript fixtures validate against their definitions and public roots', () => {
  for (const [name, value] of [
    ['CreatorProgramProject', fixture.project], ['CreatorProject', fixture.legacyProject],
    ['CreatorJobProgress', fixture.job], ['InputState', fixture.input]
  ] as const) {
    assert.equal(accepts(name, value), true, name);
    assert.equal(matches(schema, value), true, name);
  }
  assert.equal(accepts('CreatorManifest', fixture.manifest.creator), true);
  assert.equal(accepts('CreatorManifest', fixture.manifest), false);
  assert.equal(matches(schema, fixture.manifest), false, 'not a complete GameManifest schema');
  assert.equal(matches(schema, fixture.manifest.creator), false, 'extension is selected explicitly through $defs');
});

test('JSON Schema definitions agree with Zod on fixtures, required fields and unknown fields', () => {
  const cases = [
    ['CreatorProgramProject', creator.creatorProgramProjectSchema, fixture.project],
    ['CreatorProject', creatorProjectSchema, fixture.legacyProject],
    ['CreatorManifest', creatorManifestSchema, fixture.manifest.creator],
    ['CreatorJobProgress', creatorJobProgressSchema, fixture.job],
    ['InputState', inputStateSchema, fixture.input],
    ['AdapterReference', adapterReferenceSchema, fixture.legacyProject.adapter],
    ['BuildProfile', buildProfileSchema, fixture.legacyProject.build_profiles[0]],
    ['ControlsProfile', controlsProfileSchema, fixture.legacyProject.controls_profiles[0]],
    ['ProjectionRule', projectionRuleSchema, fixture.legacyProject.projection_rules[0]],
    ['CreatorProjectRequest', creator.creatorProjectRequestSchema, { projectId: fixture.project.project_id }],
    ['CreatorProjectSaveRequest', creator.creatorProjectSaveRequestSchema, { project: fixture.project, expectedRevision: null }],
    ['CreatorProjectSaveResponse', creator.creatorProjectSaveResponseSchema, fixture.projectSaved],
    ['CreatorPrivateBindings', creator.creatorPrivateBindingsSchema, fixture.projectBindings.bindings],
    ['SavedPrivateBindings', creator.savedPrivateBindingsSchema, fixture.projectBindings],
    ['CreatorProjectOpenResponse', creator.creatorProjectOpenResponseSchema, { project: fixture.project, privateBindings: fixture.projectBindings, missingInputs: [] }],
    ['CreatorProjectSummary', creator.creatorProjectSummarySchema, fixture.projectSummary],
    ['CreatorResumeStamp', creator.creatorResumeStampSchema, fixture.resumeStamp],
    ['CreatorReport', creator.creatorReportSchema, fixture.report],
    ['CreatorJobStartRequest', creator.creatorJobStartRequestSchema, fixture.jobStart],
    ['CreatorProfileSelection', creatorProfileSelectionSchema, fixture.profileSelection]
  ] as const;
  for (const [name, zod, value] of cases) {
    assert.equal(accepts(name, value), zod.safeParse(value).success, name);
    const extra = { ...value, privateLocalPath: 'C:/private/not-exported' };
    assert.equal(accepts(name, extra), false, `${name}: unknown field`);
    assert.equal(zod.safeParse(extra).success, false);
    for (const key of Object.keys(value)) {
      const missing = { ...value };
      delete missing[key];
      assert.equal(accepts(name, missing), false, `${name}: missing ${key}`);
      assert.equal(zod.safeParse(missing).success, false);
    }
  }
  const project = structuredClone(fixture.project);
  project.program_descriptor.components[0].localPath = 'C:/private/runtime';
  assert.equal(accepts('CreatorProgramProject', project), false);
  const manifest = structuredClone(fixture.manifest.creator);
  manifest.components[0].id = 'project-component-id-is-not-a-manifest-field';
  assert.equal(accepts('CreatorManifest', manifest), false);
});

test('component origin discriminator, HTTPS pins and logical binding unions agree with Zod', () => {
  const hash = 'a'.repeat(64);
  const origins = [
    { kind: 'managed', artifact_ref: 'runtime', redistribution_allowed: true },
    { kind: 'external', url: 'https://example.com/runtime.zip', transport_blake3: hash, inventory_blake3: hash },
    { kind: 'user_provided', requirement_id: 'own-runtime', inventory_blake3: hash, instructions: '' }
  ];
  const bindings = [
    { kind: 'keyboard', code: 'KeyX' }, { kind: 'mouse_button', button: 0 },
    { kind: 'mouse_axis', axis: 'x', direction: 'positive' },
    { kind: 'controller_button', button: 'a' },
    { kind: 'controller_axis', axis: 'left_x', direction: 'negative' }
  ];
  for (const [name, zod, values] of [
    ['ComponentOrigin', componentOriginSchema, origins], ['LogicalBinding', logicalBindingSchema, bindings]
  ] as const) {
    for (const value of values) {
      assert.equal(accepts(name, value), true);
      assert.equal(zod.safeParse(value).success, true);
      for (const invalid of [{ ...value, kind: 'unknown' }, { ...value, privateGuid: 'not-portable' }]) {
        assert.equal(accepts(name, invalid), false);
        assert.equal(zod.safeParse(invalid).success, false);
      }
      for (const key of Object.keys(value)) {
        const invalid: Record<string, unknown> = { ...value };
        delete invalid[key];
        assert.equal(accepts(name, invalid), false, `${name}: missing ${key}`);
      }
    }
  }
  for (const url of ['https://example.com/a', 'HTTPS://example.com/a', 'https://example.com/a@b?x=1']) {
    const value = { ...origins[1], url };
    assert.equal(accepts('ComponentOrigin', value), true, url);
    assert.equal(componentOriginSchema.safeParse(value).success, true);
  }
  for (const url of ['http://example.com/a', 'file:///private/a', 'https://user:secret@example.com/a', 'https://user@example.com/a']) {
    const value = { ...origins[1], url };
    assert.equal(accepts('ComponentOrigin', value), false);
    assert.equal(componentOriginSchema.safeParse(value).success, false);
  }
  for (const value of [
    { ...origins[1], transport_blake3: 'A'.repeat(64) },
    { ...origins[1], inventory_blake3: 'short' },
    { ...bindings[1], button: -1 }, { ...bindings[1], button: 0.5 },
    { ...bindings[2], axis: 'z' }, { ...bindings[4], direction: 'up' }
  ]) {
    const name = value.kind === 'external' ? 'ComponentOrigin' : 'LogicalBinding';
    const zod = name === 'ComponentOrigin' ? componentOriginSchema : logicalBindingSchema;
    assert.equal(accepts(name, value), false);
    assert.equal(zod.safeParse(value).success, false);
  }
});

test('managed redistribution flag is data, not a substitute for publication policy', () => {
  assert.equal(accepts('ComponentOrigin', { kind: 'managed', artifact_ref: 'runtime', redistribution_allowed: false }), true);
});

test('job/input sequences and revisions are bounded safe integers; null fields remain required', () => {
  for (const sequence of [0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
    for (const [name, value, zod] of [
      ['CreatorJobProgress', fixture.job, creatorJobProgressSchema], ['InputState', fixture.input, inputStateSchema]
    ] as const) {
      assert.equal(accepts(name, { ...value, sequence }), false);
      assert.equal(zod.safeParse({ ...value, sequence }).success, false);
    }
  }
  for (const sequence of [1, Number.MAX_SAFE_INTEGER]) {
    assert.equal(accepts('CreatorJobProgress', { ...fixture.job, sequence }), true);
    assert.equal(accepts('InputState', { ...fixture.input, sequence }), true);
  }
  for (const revision of [-1, 0.5, Number.MAX_SAFE_INTEGER + 1]) {
    assert.equal(accepts('CreatorProgramProject', { ...fixture.project, revision }), false);
    assert.equal(accepts('CreatorJobProgress', { ...fixture.job, projectRevision: revision }), false);
    assert.equal(accepts('CreatorJobProgress', { ...fixture.job, completedUnits: revision }), false);
    assert.equal(accepts('CreatorJobProgress', { ...fixture.job, totalUnits: revision }), false);
  }
  assert.equal(accepts('CreatorProgramProject', { ...fixture.project, revision: 0 }), true);
  assert.equal(accepts('CreatorProgramProject', { ...fixture.project, revision: Number.MAX_SAFE_INTEGER }), true);
  assert.equal(accepts('CreatorProgramProject', { ...fixture.project, schema_version: '1.0' }), false);
  assert.equal(accepts('CreatorProgramProject', { ...fixture.project, project_id: '' }), false);
  assert.equal(accepts('CreatorProgramProject', { ...fixture.project, project_id: 'a'.repeat(129) }), false);
  const error = { code: 'unavailable', message: 'Not available', details: { nested: [null, true, 1] } };
  assert.equal(accepts('CreatorJobProgress', { ...fixture.job, totalUnits: 10, checkpointId: 'cp-1', releaseId: 'release-1', error }), true);
  assert.equal(accepts('InputState', { ...fixture.input, deviceId: null, error }), true);
  assert.equal(accepts('InputState', { ...fixture.input, error: { ...error, localPath: 'private' } }), false);
  for (const axis of [-1.01, 1.01, '0']) assert.equal(accepts('InputState', { ...fixture.input, axes: { left_x: axis } }), false);
  for (const axis of [-1, 0, 1]) assert.equal(accepts('InputState', { ...fixture.input, axes: { left_x: axis } }), true);
  assert.equal(accepts('InputState', { ...fixture.input, buttons: { a: 1 } }), false);
  assert.equal(accepts('InputState', { ...fixture.input, status: 'running' }), false);
  assert.equal(accepts('CreatorJobProgress', { ...fixture.job, status: 'active' }), false);
  assert.equal(accepts('CreatorJobProgress', { ...fixture.job, kind: 'install' }), false);
});

test('checkpoint, report, release and job DTO definitions validate pinned camelCase records', () => {
  const hash = 'b'.repeat(64);
  const checkpoint = {
    checkpointId: 'cp-1', jobId: 'job-1', boundary: 'artifact_committed',
    stamp: fixture.resumeStamp,
    resultBlake3: hash, artifactId: 'artifact-1'
  };
  const report = fixture.report;
  const release = {
    releaseId: 'release-1', projectId: 'fixture-project', projectRevision: 1,
    manifestBlake3: hash, artifactHashes: { runtime: hash }, reportIds: ['report-1']
  };
  for (const [name, value] of [
    ['CreatorCheckpoint', checkpoint], ['CreatorReport', report], ['CreatorRelease', release],
    ['CreatorResumeStamp', checkpoint.stamp],
    ['CreatorJob', { progress: fixture.job, checkpoints: [checkpoint], reportIds: ['report-1'] }],
    ['CreatorJobReceipt', { jobId: 'job-1', projectId: 'fixture-project', projectRevision: 1 }],
    ['InputBindingsReceipt', { sessionId: 'session-1', controlsProfileId: 'keyboard' }]
  ] as const) {
    assert.equal(accepts(name, value), true, name);
    assert.equal(accepts(name, { ...value, unexpected: true }), false, name);
    for (const key of Object.keys(value)) {
      const missing: Record<string, unknown> = { ...value };
      delete missing[key];
      assert.equal(accepts(name, missing), false, `${name}: missing ${key}`);
    }
  }
  assert.equal(accepts('CreatorCheckpoint', { ...checkpoint, boundary: 'partial_file_written' }), false);
  assert.equal(accepts('CreatorCheckpoint', { ...checkpoint, resultBlake3: 'not-a-hash' }), false);
  assert.equal(accepts('CreatorCheckpoint', { ...checkpoint, stamp: { ...checkpoint.stamp, inputHashes: { runtime: 'short' } } }), false);
  assert.equal(accepts('CreatorReport', { ...report, kind: 'verified' }), false);
  assert.equal(accepts('CreatorReport', { ...report, status: 'completed' }), false);
  for (const [name, value] of [['CreatorResumeStamp', fixture.resumeStamp], ['CreatorReport', report]] as const) {
    const { executor: _executor, ...legacy } = value;
    assert.equal(accepts(name, { ...legacy, adapter: fixture.legacyProject.adapter }), false);
    assert.equal(accepts(name, { ...value, adapter: fixture.legacyProject.adapter }), false);
    assert.equal(accepts(name, { ...value, executor: { id: 'supertuxkart', version: '1.5' } }), false);
  }
  assert.equal(accepts('CreatorProjectListResponse', [fixture.projectSummary]), true);
  assert.equal(accepts('CreatorProjectListResponse', [fixture.project]), false);
  assert.equal(creator.creatorProjectListResponseSchema.safeParse([fixture.project]).success, false);
  assert.equal(accepts('CreatorProjectSaveRequest', { project: fixture.legacyProject, expectedRevision: 1 }), false);
  assert.equal(accepts('CreatorRelease', { ...release, artifactHashes: { runtime: 'short' } }), false);
});

test('raw input variants stay separate from portable logical bindings', () => {
  const raw = [
    { kind: 'button', index: 0 }, { kind: 'hat', index: 1, mask: 255 },
    { kind: 'axis', index: 2, direction: 'negative' }
  ];
  for (const value of raw) {
    assert.equal(accepts('RawInputBinding', value), true);
    assert.equal(accepts('LogicalBinding', value), false);
    assert.equal(accepts('RawInputBinding', { ...value, index: -1 }), false);
    assert.equal(accepts('RawInputBinding', { ...value, index: 4294967296 }), false);
    assert.equal(accepts('RawInputBinding', { ...value, guid: 'private' }), false);
  }
  assert.equal(accepts('RawInputBinding', { kind: 'hat', index: 1, mask: 256 }), false);
  const request = {
    sessionId: 'session-1', controlsProfileId: 'keyboard',
    bindings: { a: { kind: 'keyboard', code: 'KeyX' } }, deadzones: { left_x: 0.15 },
    rawBindings: { a: raw[0] }
  };
  assert.equal(accepts('InputBindingsSaveRequest', request), true);
  assert.equal(accepts('InputBindingsSaveRequest', { ...request, bindings: { a: raw[0] } }), false);
});

const programFixture = (name: string) => JSON.parse(readFileSync(new URL(`../../fixtures/creator/programs/${name}.json`, import.meta.url), 'utf8'));

test('Eden and non-emulator SuperTuxKart descriptors validate without changing legacy DTOs', () => {
  for (const name of ['eden-0.2.1', 'supertuxkart-1.5']) {
    const program = programFixture(name);
    assert.equal(accepts('ProgramDescriptor', program), true, name);
    assert.equal(matches(schema, program), true);
    assert.equal(accepts('CreatorProject', program), false);
    const project = {
      schema_version: '1.1', project_id: `program-${name}`, revision: 1,
      title: program.identity.title, source_id: 'descriptor-schema-fixture',
      game_id: program.identity.id, game_version: program.identity.version, program_descriptor: program
    };
    assert.equal(accepts('CreatorProgramProject', project), true);
    assert.equal(matches(schema, project), true);
    assert.equal(creatorProjectSchema.safeParse(project).success, false, 'legacy validator must not accept the project IPC 1.1 carrier');
    const extension = { program_descriptor: program };
    assert.equal(accepts('ProgramCreatorManifest', extension), true);
    assert.equal(matches(schema, extension), false, 'select extension, not a complete GameManifest');
    assert.equal(creatorManifestSchema.safeParse(extension).success, false);
    for (const key of Object.keys(program)) {
      const missing = { ...program };
      delete missing[key];
      assert.equal(accepts('ProgramDescriptor', missing), false, `${name}: missing ${key}`);
    }
    for (const field of ['shell', 'script', 'command', 'post_setup_steps', 'wasm', 'plugin_url', 'local_path']) {
      assert.equal(accepts('ProgramDescriptor', { ...program, [field]: 'untrusted' }), false, field);
    }
  }
});

test('program path schema blocks Windows lexical escapes but is not a filesystem authorization', () => {
  for (const path of ['app/eden.exe', 'app/stk-code/data/stk_config.xml', 'userdata/config-0.10', 'app/игра.exe']) {
    assert.equal(accepts('ProgramPath', path), true, path);
  }
  for (const path of ['', '.', '..', '../x', 'x/../y', 'x/./y', '/x', 'C:/x', 'C:x', '\\\\server\\x', 'x\\y',
    'x:y', 'x//y', 'x/', 'con', 'x/CON.txt', 'nul', 'aux.exe', 'COM1', 'Lpt9.txt', 'x./y', 'x /y', 'x.',
    'x ', 'x\n', 'x\u0085', 'x*', 'x?', 'x|y', 'x<y', 'x>y', 'x"y', 'a'.repeat(513)]) {
    assert.equal(accepts('ProgramPath', path), false, JSON.stringify(path));
  }
  assert.equal(accepts('ProgramDirectory', ''), true);
});

test('program operations are bounded argv, workspace paths and scalar config selectors, not scripts', () => {
  for (const arg of [
    { kind: 'literal', value: '--windowed' },
    { kind: 'workspace_path', path: 'userdata', prefix: '--data=' },
    { kind: 'workspace_path', path: 'game/file.nsp', prefix: '' }
  ]) assert.equal(accepts('ProgramArgument', arg), true);
  for (const arg of [
    { kind: 'shell', value: 'cmd /c something' },
    { kind: 'workspace_path', path: '../private', prefix: '' },
    { kind: 'workspace_path', path: 'userdata', prefix: 'cmd /c ' },
    { kind: 'literal', value: 'bad\u0000argument' },
    { kind: 'template', value: '{{WORKSPACE_DIR}}' }
  ]) assert.equal(accepts('ProgramArgument', arg), false);
  for (const name of ['SUPERTUXKART_SAVEDIR', 'SUPERTUXKART_DATADIR', 'SUPERTUXKART_HIGHSCOREDIR']) {
    assert.equal(accepts('ProgramEnvironmentPath', { name, path: 'userdata' }), true);
  }
  for (const name of ['PATH', 'APPDATA', 'LOCALAPPDATA', 'COMSPEC', 'QT_PLUGIN_PATH', 'NODE_OPTIONS', 'PYTHONPATH', 'lowercase']) {
    assert.equal(accepts('ProgramEnvironmentPath', { name, path: 'userdata' }), false, name);
  }
  assert.equal(accepts('ProgramConfigAddress', { kind: 'xpath', expression: '//Video[@foo]' }), false);
  assert.equal(accepts('ProgramConfigSeed', { kind: 'xml_root', root: '!DOCTYPE', attributes: [] }), false);
  assert.equal(accepts('ProgramPersistence', { path: 'userdata/players.xml', kind: 'file', policy: 'preserve_latest', export: 'always' }), false);
});

test('program finite profiles and operation budgets reject unbounded or duplicate selections', () => {
  const program = programFixture('supertuxkart-1.5');
  for (const field of ['components', 'required_files', 'build_profiles', 'controls_profiles']) {
    assert.equal(accepts('ProgramDescriptor', { ...program, [field]: [] }), false, field);
  }
  const profile = program.build_profiles[0];
  assert.equal(accepts('ProgramBuildProfile', { ...profile, layer_ids: ['stk-runtime', 'stk-runtime'] }), false);
  assert.equal(accepts('ProgramBuildProfile', { ...profile, controls_profile_ids: [] }), false);
  assert.equal(accepts('ProgramDescriptor', { ...program, directories: Array.from({ length: 65 }, (_, i) => `d${i}`) }), false);
  assert.equal(accepts('ProgramEntrypoint', { ...program.entrypoint, arguments: Array.from({ length: 65 }, () => ({ kind: 'literal', value: 'x' })) }), false);
  const key = program.configs[0].keys[0];
  for (const allowed_values of [[], [true, true], Array.from({ length: 65 }, (_, i) => i)]) {
    assert.equal(accepts('ProgramConfigKey', { ...key, allowed_values }), false);
  }
});

test('native defaults are distinct from Eden writer capability and private bindings never become portable', () => {
  const native = programFixture('supertuxkart-1.5').controls_profiles[0];
  const eden = programFixture('eden-0.2.1').controls_profiles[0];
  assert.equal(accepts('ProgramControlsProfile', native), true);
  assert.equal(accepts('ProgramControlsProfile', { ...native, bindings: {} }), false);
  assert.equal(accepts('ProgramControlsProfile', eden), true);
  assert.equal(accepts('ProgramControlsProfile', { ...eden, kind: 'download_my_adapter' }), false);
  assert.equal(accepts('ProgramControlsProfile', { ...eden, guid: 'private-guid' }), false);
  assert.equal(accepts('ProgramControlsProfile', { ...eden, bindings: { a: { kind: 'button', index: 0 } } }), false);
  assert.equal(accepts('ProgramDescriptor', { ...programFixture('eden-0.2.1'), executor: { id: 'eden', version: '0.2.1' } }), false);
});
