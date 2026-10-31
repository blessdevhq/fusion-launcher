import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { creatorProfileSelectionSchema, projectionRuleSchema } from './fusionCreator.ts';

const fixture = JSON.parse(readFileSync(new URL('../../fixtures/creator/contracts.json', import.meta.url), 'utf8'));
const document = JSON.parse(readFileSync(new URL('../../docs/schemas/fusion-creator-contracts.schema.json', import.meta.url), 'utf8'));

test('a selected profile pair has explicit buildProfileId and controlsProfileId', () => {
  assert.deepEqual(creatorProfileSelectionSchema.parse(fixture.profileSelection), fixture.profileSelection);
  for (const invalid of [
    { buildProfileId: 'default' }, { controlsProfileId: 'keyboard' },
    { buildProfileId: '', controlsProfileId: 'keyboard' },
    { buildProfileId: 'default', controlsProfileId: '' },
    { ...fixture.profileSelection, enabledAddons: ['implicit-mix'] }
  ]) assert.equal(creatorProfileSelectionSchema.safeParse(invalid).success, false);
  assert.deepEqual(document.$defs.CreatorProfileSelection.required, ['buildProfileId', 'controlsProfileId']);
  assert.equal(document.$defs.CreatorProfileSelection.additionalProperties, false);
});

test('a portable path decision names its explicit winning layer, never dependency priority', () => {
  const decision = fixture.project.projection_rules[0];
  assert.deepEqual(projectionRuleSchema.parse(decision), decision);
  assert.equal(projectionRuleSchema.safeParse({ path: decision.path }).success, false);
  assert.equal(projectionRuleSchema.safeParse({ path: decision.path, priority: 1 }).success, false);
  assert.equal(projectionRuleSchema.safeParse({ ...decision, winning_layer: '' }).success, false);
});
