import assert from 'node:assert/strict';
import { it } from 'node:test';
import { acquireInputOwner } from './focusManager.ts';
import * as input from './gamepadInput.ts';

it('leaves Enter and Space to the focused native control, never a stale shell target', () => {
  for (const [code, key] of [['Enter', 'Enter'], ['Space', ' '], ['', 'Enter'], ['', ' ']]) {
    assert.equal(input.gamepadCommandForKey({ code, key, nativeActivation: 'enter-space' }), null);
  }
  assert.equal(input.gamepadCommandForKey({ code: 'GamepadA', key: '', nativeActivation: 'enter-space' }), 'activate');
  assert.equal(input.gamepadCommandForKey({ code: 'ArrowDown', key: 'ArrowDown', nativeActivation: 'enter-space' }), 'down');
});

it('leaves Enter to a native link and routes Space only to the focused link itself', () => {
  assert.equal(input.gamepadCommandForKey({ code: 'Enter', key: 'Enter', nativeActivation: 'link', nativeControlIsShellTarget: true }), null);
  const space = { code: 'Space', key: ' ', nativeActivation: 'link' } as const;
  assert.equal(input.gamepadCommandForKey({ ...space, nativeControlIsShellTarget: true }), 'activate');
  assert.equal(input.gamepadCommandForKey({ ...space, nativeControlIsShellTarget: false }), null);
});

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
