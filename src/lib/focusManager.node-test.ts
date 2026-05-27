import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { actionForInput, nextFocusTarget } from './focusManager.ts';

const targets = [
  { id: 'a', zone: 'grid', rect: { left: 0, top: 0, width: 10, height: 10 } },
  { id: 'b', zone: 'grid', rect: { left: 30, top: 0, width: 10, height: 10 } },
  { id: 'c', zone: 'grid', rect: { left: 0, top: 30, width: 10, height: 10 } }
];

describe('focus manager', () => {
  it('moves to the closest target in the requested direction', () => {
    assert.equal(nextFocusTarget(targets, 'a', 'right')?.id, 'b');
    assert.equal(nextFocusTarget(targets, 'a', 'down')?.id, 'c');
  });

  it('wraps inside the current zone when no directional target exists', () => {
    assert.equal(nextFocusTarget(targets, 'b', 'right')?.id, 'a');
  });

  it('maps controller-like keyboard actions', () => {
    assert.equal(actionForInput('Enter'), 'activate');
    assert.equal(actionForInput('Escape'), 'back');
    assert.equal(actionForInput('KeyX'), 'search');
    assert.equal(actionForInput('KeyY'), 'menu');
    assert.equal(actionForInput('Tab'), null);
  });
});
