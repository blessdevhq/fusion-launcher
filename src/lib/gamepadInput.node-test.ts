import assert from 'node:assert/strict';
import { it } from 'node:test';
import { acquireInputOwner } from './focusManager.ts';
import * as input from './gamepadInput.ts';

it('routes shell keys only while no nested input owner exists', () => {
  const event = { code: 'Enter', key: 'Enter' };
  assert.equal(input.gamepadCommandForKey(event), 'activate');
  const dialog = Symbol('dialog');
  const release = acquireInputOwner(dialog);
  try {
    for (const code of ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Enter', 'Escape', 'Tab', 'KeyY']) {
      assert.equal(input.gamepadCommandForKey({ code, key: code }), null);
    }
    assert.equal(input.gamepadCommandForKey(event, { inputOwner: dialog }), 'activate');
    const releaseCapture = acquireInputOwner();
    try {
      assert.equal(input.gamepadCommandForKey(event, { inputOwner: dialog }), null);
    } finally { releaseCapture(); }
    assert.equal(input.gamepadCommandForKey(event, { inputOwner: dialog, enabled: false }), null);
    assert.equal(input.gamepadCommandForKey({ ...event, defaultPrevented: true }, { inputOwner: dialog }), null);
    assert.equal(input.gamepadCommandForKey({ ...event, isComposing: true }, { inputOwner: dialog }), null);
  } finally { release(); }
  assert.equal(input.gamepadCommandForKey({ code: '', key: 'ArrowDown' }), 'down');
  assert.equal(input.gamepadCommandForKey({ code: '', key: 'Escape' }), 'back');
});
