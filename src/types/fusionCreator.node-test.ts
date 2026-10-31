import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  creatorProjectSchema, creatorJobProgressSchema, inputStateSchema,
  CREATOR_COMMANDS, INPUT_COMMANDS, CREATOR_JOB_EVENT, INPUT_STATE_EVENT
} from './fusionCreator.ts';

const fixture = JSON.parse(readFileSync(new URL('../../fixtures/creator/contracts.json', import.meta.url), 'utf8'));

test('Creator portable project and job/input wire fixtures satisfy strict contracts', () => {
  assert.deepEqual(creatorProjectSchema.parse(fixture.project), fixture.project);
  assert.deepEqual(creatorJobProgressSchema.parse(fixture.job), fixture.job);
  assert.deepEqual(inputStateSchema.parse(fixture.input), fixture.input);
});

test('portable component origins exclude local bindings and require pinned external inputs', () => {
  const project = structuredClone(fixture.project);
  project.components[0].localPath = 'C:/private/runtime';
  assert.equal(creatorProjectSchema.safeParse(project).success, false);
  const external = structuredClone(fixture.project);
  external.components[0].origin = { kind: 'external', url: 'https://example.com/runtime.zip' };
  assert.equal(creatorProjectSchema.safeParse(external).success, false);
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
