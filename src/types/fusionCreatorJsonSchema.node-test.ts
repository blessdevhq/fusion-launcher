import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
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
  'additionalProperties', 'items'
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
  if (Array.isArray(value) && node.items !== undefined && !value.every((entry) => matches(node.items!, entry))) return false;
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

test('Creator JSON Schema publishes draft 2020-12 and the three public wire roots', () => {
  assert.equal(schema.$schema, 'https://json-schema.org/draft/2020-12/schema');
  assert.deepEqual(schema.oneOf, [
    { $ref: '#/$defs/CreatorProject' },
    { $ref: '#/$defs/CreatorJobProgress' },
    { $ref: '#/$defs/InputState' }
  ]);
});

test('shared Rust/TypeScript fixtures validate against their definitions and public roots', () => {
  for (const [name, value] of [
    ['CreatorProject', fixture.project], ['CreatorJobProgress', fixture.job], ['InputState', fixture.input]
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
    ['CreatorProject', creatorProjectSchema, fixture.project],
    ['CreatorManifest', creatorManifestSchema, fixture.manifest.creator],
    ['CreatorJobProgress', creatorJobProgressSchema, fixture.job],
    ['InputState', inputStateSchema, fixture.input],
    ['AdapterReference', adapterReferenceSchema, fixture.project.adapter],
    ['BuildProfile', buildProfileSchema, fixture.project.build_profiles[0]],
    ['ControlsProfile', controlsProfileSchema, fixture.project.controls_profiles[0]],
    ['ProjectionRule', projectionRuleSchema, fixture.project.projection_rules[0]],
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
  project.components[0].localPath = 'C:/private/runtime';
  assert.equal(accepts('CreatorProject', project), false);
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
    assert.equal(accepts('CreatorProject', { ...fixture.project, revision }), false);
    assert.equal(accepts('CreatorJobProgress', { ...fixture.job, projectRevision: revision }), false);
    assert.equal(accepts('CreatorJobProgress', { ...fixture.job, completedUnits: revision }), false);
    assert.equal(accepts('CreatorJobProgress', { ...fixture.job, totalUnits: revision }), false);
  }
  assert.equal(accepts('CreatorProject', { ...fixture.project, revision: 0 }), true);
  assert.equal(accepts('CreatorProject', { ...fixture.project, revision: Number.MAX_SAFE_INTEGER }), true);
  assert.equal(accepts('CreatorProject', { ...fixture.project, schema_version: '1.1' }), false);
  assert.equal(accepts('CreatorProject', { ...fixture.project, project_id: '' }), false);
  assert.equal(accepts('CreatorProject', { ...fixture.project, project_id: 'a'.repeat(129) }), false);
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
    stamp: { projectRevision: 1, inputHashes: { runtime: hash }, adapter: fixture.project.adapter },
    resultBlake3: hash, artifactId: 'artifact-1'
  };
  const report = {
    reportId: 'report-1', releaseId: 'release-1', releaseBlake3: hash, projectRevision: 1,
    kind: 'structure_integrity', status: 'not_tested', adapter: fixture.project.adapter,
    buildProfileId: 'default', controlsProfileId: 'keyboard', environment: { os: 'windows' }, diagnostics: []
  };
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
