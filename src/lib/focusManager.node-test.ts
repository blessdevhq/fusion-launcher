import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { actionForInput, nextFocusTarget } from './focusManager.ts';
import * as focus from './focusManager.ts';

const targets = [
  { id: 'a', zone: 'grid', rect: { left: 0, top: 0, width: 10, height: 10 } },
  { id: 'b', zone: 'grid', rect: { left: 30, top: 0, width: 10, height: 10 } },
  { id: 'c', zone: 'grid', rect: { left: 0, top: 30, width: 10, height: 10 } }
];

describe('focus manager', () => {
  it('suspends the shell until the top nested input owner releases', () => {
    const dialog = Symbol('dialog');
    const assignment = Symbol('assignment');
    assert.equal(focus.isInputOwnerActive(), true);
    const releaseDialog = focus.acquireInputOwner(dialog);
    try {
      assert.equal(focus.isInputOwnerActive(), false);
      assert.equal(focus.isInputOwnerActive(dialog), true);
      const releaseAssignment = focus.acquireInputOwner(assignment);
      try {
        assert.equal(focus.isInputOwnerActive(dialog), false);
        assert.equal(focus.isInputOwnerActive(assignment), true);
      } finally {
        releaseAssignment();
      }
      assert.equal(focus.isInputOwnerActive(dialog), true);
    } finally {
      releaseDialog();
    }
    assert.equal(focus.isInputOwnerActive(), true);
  });

  it('moves to the closest target in the requested direction', () => {
    assert.equal(nextFocusTarget(targets, 'a', 'right')?.id, 'b');
    assert.equal(nextFocusTarget(targets, 'a', 'down')?.id, 'c');
  });

  it('wraps inside the current zone when no directional target exists', () => {
    assert.equal(nextFocusTarget(targets, 'b', 'right')?.id, 'a');
  });

  it('keeps navigation and wrapping within an explicit scope, not just a visual zone', () => {
    const dialog = Symbol('dialog');
    const assignment = Symbol('assignment');
    const scopedTargets = [
      ...targets,
      { ...targets[0], id: 'dialog-a', scope: dialog },
      { ...targets[1], id: 'dialog-b', scope: dialog },
      { ...targets[2], id: 'assignment', scope: assignment }
    ];
    assert.equal(nextFocusTarget(scopedTargets, 'dialog-a', 'right', dialog)?.id, 'dialog-b');
    assert.equal(nextFocusTarget(scopedTargets, 'dialog-b', 'right', dialog)?.id, 'dialog-a');
    assert.equal(nextFocusTarget(scopedTargets, 'dialog-b', 'down', assignment)?.id, 'assignment');
    assert.equal(nextFocusTarget(targets, 'a', 'right', dialog), null);
  });

  it('does not reactivate an unmounted parent on out-of-order or repeated cleanup', () => {
    const parent = Symbol('parent');
    const child = Symbol('child');
    const releaseParent = focus.acquireInputOwner(parent);
    const releaseChild = focus.acquireInputOwner(child);
    releaseParent();
    releaseParent();
    assert.equal(focus.isInputOwnerActive(child), true);
    assert.equal(focus.isInputOwnerActive(), false);
    releaseChild();
    releaseChild();
    assert.equal(focus.isInputOwnerActive(), true);
  });

  it('maps controller-like keyboard actions', () => {
    assert.equal(actionForInput('Enter'), 'activate');
    assert.equal(actionForInput('Escape'), 'back');
    assert.equal(actionForInput('Backspace'), null);
    assert.equal(actionForInput('KeyX'), null);
    assert.equal(actionForInput('KeyY'), 'menu');
    assert.equal(actionForInput('Tab'), null);
  });
});
