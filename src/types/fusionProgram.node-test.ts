import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import * as creator from './fusionCreator.ts';

const fixtures = ['eden-0.2.1', 'supertuxkart-1.5'].map((name) =>
  JSON.parse(readFileSync(new URL(`../../fixtures/creator/programs/${name}.json`, import.meta.url), 'utf8')));

test('public program descriptor schema roundtrips both frozen program fixtures', () => {
  assert.ok('programDescriptorSchema' in creator, 'missing strict ProgramDescriptor schema');
  const schema = creator.programDescriptorSchema as { parse(value: unknown): unknown };
  for (const fixture of fixtures) {
    assert.deepEqual(schema.parse(fixture), fixture);
    assert.deepEqual(schema.parse(JSON.parse(JSON.stringify(schema.parse(fixture)))), fixture);
  }
});

test('versioned program project and manifest carriers reject legacy copies and private fields', () => {
  const project = { schema_version: '1.1', project_id: 'project', revision: 0, title: 'Project',
    source_id: 'source', game_id: 'game', game_version: '1', program_descriptor: fixtures[0] };
  assert.deepEqual(creator.creatorProgramProjectSchema.parse(project), project);
  assert.equal(creator.creatorProjectSchema.safeParse(project).success, false);
  const legacy = JSON.parse(readFileSync(new URL('../../fixtures/creator/contracts.json', import.meta.url), 'utf8'));
  assert.equal(creator.creatorProgramProjectSchema.safeParse(legacy.project).success, false);
  assert.equal(creator.programCreatorManifestSchema.safeParse(legacy.manifest.creator).success, false);
  assert.deepEqual(creator.programCreatorManifestSchema.parse({ program_descriptor: fixtures[0] }),
    { program_descriptor: fixtures[0] });
  for (const extra of [{ components: [] }, { adapter: { id: 'native', version: '1' } }, { localPath: 'C:/private' }]) {
    assert.equal(creator.creatorProgramProjectSchema.safeParse({ ...project, ...extra }).success, false);
    assert.equal(creator.programCreatorManifestSchema.safeParse({ program_descriptor: fixtures[0], ...extra }).success, false);
  }
  for (const revision of [-1, 0.5, Number.MAX_SAFE_INTEGER + 1]) {
    assert.equal(creator.creatorProgramProjectSchema.safeParse({ ...project, revision }).success, false);
  }
  assert.equal(creator.creatorProgramProjectSchema.safeParse({ ...project, revision: Number.MAX_SAFE_INTEGER }).success, true);
});

test('program carriers enforce constants, nested strictness and Unicode identifier bounds', () => {
  for (const mutate of [
    (v: any) => { v.schema_version = '1.1'; },
    (v: any) => { v.executor.id = 'native'; },
    (v: any) => { v.executor.version = '2.0'; },
    (v: any) => { v.identity.os = 'linux'; },
    (v: any) => { v.identity.architecture = 'unknown'; },
    (v: any) => { v.components[0].private_path = 'C:/private'; },
    (v: any) => { v.configs[0].seed.secret = 'must-not-export'; },
    (v: any) => { v.configs[0].preserve_unmanaged = false; },
    (v: any) => { v.persistence[0].export = 'always'; }
  ]) {
    const value = structuredClone(fixtures[0]);
    mutate(value);
    assert.equal(creator.programDescriptorSchema.safeParse(value).success, false);
  }
  const unicode = structuredClone(fixtures[0]);
  unicode.identity.id = '🎮'.repeat(128);
  assert.equal(creator.programDescriptorSchema.safeParse(unicode).success, true);
  unicode.identity.id += '🎮';
  assert.equal(creator.programDescriptorSchema.safeParse(unicode).success, false);
});

test('program path guards reject Windows aliases and traversal without interpreting glob syntax', () => {
  for (const path of ['', '/absolute', 'C:/host', '../escape', 'a/../b', 'a//b', 'a/./b', 'a\\b',
    'CON', 'aux.txt', 'dir/LPT9.ini', 'trailing.', 'trailing ', 'wild*', 'wild?', 'a\0b', 'a\u007fb', 'a\u009fb']) {
    const value = structuredClone(fixtures[0]);
    value.entrypoint.executable = path;
    assert.equal(creator.programDescriptorSchema.safeParse(value).success, false, path);
  }
  for (const path of ['bin/app.exe', 'literal[1].exe', '🎮'.repeat(512)]) {
    const value = structuredClone(fixtures[0]);
    value.entrypoint.executable = path;
    assert.equal(creator.programDescriptorSchema.safeParse(value).success, true, path);
  }
});

test('structured argv, environment and scalar guards do not accept shell or loader overrides', () => {
  for (const argument of [{ kind: 'shell', value: 'cmd.exe' }, { kind: 'literal', value: 'line\nbreak' },
    { kind: 'workspace_path', path: '../host', prefix: '' }, { kind: 'workspace_path', path: '', prefix: '--config ' }]) {
    const value = structuredClone(fixtures[0]);
    value.entrypoint.arguments = [argument];
    assert.equal(creator.programDescriptorSchema.safeParse(value).success, false);
  }
  for (const name of ['PATH', 'COMSPEC', 'APPDATA', 'PYTHONPATH', 'NODE_OPTIONS', 'RUSTC_WRAPPER', 'lowercase']) {
    const value = structuredClone(fixtures[0]);
    value.entrypoint.environment_paths = [{ name, path: '' }];
    assert.equal(creator.programDescriptorSchema.safeParse(value).success, false, name);
  }
  for (const allowed_values of [[0, 0.0], [Number.MAX_SAFE_INTEGER + 1], ['bad\u0085scalar'], []]) {
    const value = structuredClone(fixtures[0]);
    value.configs[0].keys[0].allowed_values = allowed_values;
    assert.equal(creator.programDescriptorSchema.safeParse(value).success, false);
  }
});

test('portable binding maps preserve own special-name keys without prototype mutation', () => {
  const value = structuredClone(fixtures[0]);
  const bindings = JSON.parse('{"__proto__":{"kind":"keyboard","code":"KeyA"},"constructor":{"kind":"keyboard","code":"KeyB"}}');
  value.controls_profiles[0] = { id: 'controls', title: 'Controls', kind: 'eden_param_package_v1',
    device: 'keyboard_mouse', config_id: 'settings', bindings };
  const parsed = creator.programDescriptorSchema.parse(value);
  assert.deepEqual(parsed, value);
  assert.equal(Object.getPrototypeOf((parsed.controls_profiles[0] as any).bindings), Object.prototype);
  const legacy = { id: 'controls', title: 'Controls', device: 'keyboard_mouse', bindings };
  assert.deepEqual(creator.controlsProfileSchema.parse(legacy), legacy);
});

test('external program origins reject raw whitespace and even empty userinfo', () => {
  for (const url of [' https://example.com/a.zip', 'https://example.com/a.zip ', 'https://@example.com/a.zip',
    'https://user:password@example.com/a.zip', 'http://example.com/a.zip',
    'https://example.com/a\ufeff.zip', 'https://example.com/a\u0085.zip',
    'https://example.com/a|b', 'https://example.com/a%XX', 'https://example.com/a\0b',
    'https://[bad]/a.zip', 'not-a-url']) {
    const value = structuredClone(fixtures[0]);
    value.components[0].origin = { kind: 'external', url, transport_blake3: '0'.repeat(64), inventory_blake3: '1'.repeat(64) };
    assert.equal(creator.programDescriptorSchema.safeParse(value).success, false, url);
  }
  for (const url of ['https://example.com/%E2%9C%93.zip',
    'HTTPS://example.com:443/a.zip?download=1#asset', 'https://[::1]/a.zip']) {
    const value = structuredClone(fixtures[0]);
    value.components[0].origin = { kind: 'external', url, transport_blake3: '0'.repeat(64), inventory_blake3: '1'.repeat(64) };
    assert.equal(creator.programDescriptorSchema.safeParse(value).success, true, url);
  }
});

test('program collections enforce all frozen cardinality and unique-item limits', () => {
  for (const [field, max] of [['components', 64], ['required_files', 256], ['projection_rules', 4096],
    ['directories', 64], ['configs', 16], ['build_profiles', 32], ['controls_profiles', 16], ['persistence', 64]] as const) {
    const value = structuredClone(fixtures[0]);
    const item = value[field][0] ?? (field === 'projection_rules' ? { path: 'data.bin', winning_layer: 'base' } : 'dir');
    value[field] = Array.from({ length: max + 1 }, () => structuredClone(item));
    assert.equal(creator.programDescriptorSchema.safeParse(value).success, false, field);
  }
  for (const field of ['layer_ids', 'controls_profile_ids']) {
    const value = structuredClone(fixtures[0]);
    value.build_profiles[0][field].push(value.build_profiles[0][field][0]);
    assert.equal(creator.programDescriptorSchema.safeParse(value).success, false, field);
  }
});
