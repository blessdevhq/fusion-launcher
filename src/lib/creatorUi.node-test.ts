import assert from 'node:assert/strict';
import test from 'node:test';
import { creatorStages, creatorStageHasContent } from './creatorUi.ts';

test('Creator stages preserve the authoring order and do not expose descriptor forms', () => {
  assert.deepEqual(creatorStages, ['base', 'composition', 'profiles', 'verification', 'release']);
  assert.equal(creatorStageHasContent('base'), true);
  for (const stage of ['composition', 'profiles', 'verification', 'release'] as const) {
    assert.equal(creatorStageHasContent(stage), false);
  }
});
