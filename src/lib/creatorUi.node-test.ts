import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { creatorStages, creatorStageHasContent, supportsSignedBuild, signedBuildRequest } from './creatorUi.ts';
import type { CreatorProjectOpenResponse } from '../types/fusionCreator.ts';

const { project } = JSON.parse(readFileSync(new URL('../../fixtures/creator/contracts.json', import.meta.url), 'utf8')) as CreatorProjectOpenResponse;

test('signed build accepts only redistributable managed directories rooted at the input root', () => {
  const component = { ...project.program_descriptor.components[0], source_root: '',
    origin: { kind: 'managed', artifact_ref: 'program', redistribution_allowed: true } as const };
  assert.equal(supportsSignedBuild([component]), true);
  assert.equal(supportsSignedBuild([]), false);
  assert.equal(supportsSignedBuild([component, { ...component, source_root: 'nested' }]), false);
  assert.equal(supportsSignedBuild([{ ...component, origin: { ...component.origin, redistribution_allowed: false } }]), false);
  assert.equal(supportsSignedBuild([{ ...component, origin: { kind: 'user_provided', requirement_id: 'local', inventory_blake3: 'a'.repeat(64), instructions: 'Local' } }]), false);
});

test('signed build sends a compatible descriptor pair and no private bindings', () => {
  assert.deepEqual(signedBuildRequest(project, 'windowed', 'native'), {
    projectId: project.project_id, projectRevision: project.revision,
    buildProfileIds: ['windowed'], controlsProfileIds: ['native'], targetDirectory: null, releaseId: null, sourceUrl: null
  });
  assert.equal(signedBuildRequest(project, 'windowed', 'missing'), null);
});

test('Creator stages preserve the authoring order and do not expose descriptor forms', () => {
  assert.deepEqual(creatorStages, ['base', 'composition', 'profiles', 'verification', 'release']);
  assert.equal(creatorStageHasContent('base'), true);
  for (const stage of ['composition', 'profiles', 'verification', 'release'] as const) {
    assert.equal(creatorStageHasContent(stage), false);
  }
});
