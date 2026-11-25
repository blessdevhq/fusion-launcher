import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { acquireInputOwner } from './focusManager.ts';
import * as input from './gamepadInput.ts';
import { createGamepadInputState, pollGamepadEvents, type GamepadSnapshot } from './gamepadInput.ts';

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

function makePad(overrides: {
  connected?: boolean;
  pressedButtons?: number[];
  axes?: number[];
} = {}): GamepadSnapshot {
  const pressedSet = new Set(overrides.pressedButtons ?? []);
  const buttons = Array.from({ length: 16 }, (_, index) => ({
    pressed: pressedSet.has(index),
    value: pressedSet.has(index) ? 1 : 0
  }));

  return {
    connected: overrides.connected ?? true,
    buttons,
    axes: overrides.axes ?? [0, 0]
  };
}

describe('gamepad input polling', () => {
  it('emits edge-triggered A/B/X/Y actions without spamming while held', () => {
    const state = createGamepadInputState();

    assert.deepEqual(pollGamepadEvents(state, [makePad({ pressedButtons: [0, 2] })], 0), [
      { kind: 'action', code: 'GamepadA' },
      { kind: 'action', code: 'GamepadX' }
    ]);
    assert.deepEqual(pollGamepadEvents(state, [makePad({ pressedButtons: [0, 2] })], 100), []);
    assert.deepEqual(pollGamepadEvents(state, [makePad({ pressedButtons: [] })], 150), []);
    assert.deepEqual(pollGamepadEvents(state, [makePad({ pressedButtons: [1, 3] })], 200), [
      { kind: 'action', code: 'GamepadB' },
      { kind: 'action', code: 'GamepadY' }
    ]);
    assert.deepEqual(pollGamepadEvents(state, [makePad({ pressedButtons: [0] })], 250), [
      { kind: 'action', code: 'GamepadA' }
    ]);
  });

  it('supports D-pad directions and stick deadzone with dominant axis selection', () => {
    const state = createGamepadInputState();

    assert.deepEqual(pollGamepadEvents(state, [makePad({ axes: [0.4, -0.3] })], 0), []);
    assert.deepEqual(pollGamepadEvents(state, [makePad({ axes: [0.7, -0.58] })], 10), [
      { kind: 'direction', direction: 'right' }
    ]);
    assert.deepEqual(pollGamepadEvents(state, [makePad({ pressedButtons: [12], axes: [0.9, 0] })], 20), [
      { kind: 'direction', direction: 'up' }
    ]);
  });

  it('repeats held direction according to initial delay and repeat interval', () => {
    const state = createGamepadInputState();
    const downPad = [makePad({ pressedButtons: [13] })];

    assert.deepEqual(pollGamepadEvents(state, downPad, 0), [{ kind: 'direction', direction: 'down' }]);
    assert.deepEqual(pollGamepadEvents(state, downPad, 200), []);
    assert.deepEqual(pollGamepadEvents(state, downPad, 360), [{ kind: 'direction', direction: 'down' }]);
    assert.deepEqual(pollGamepadEvents(state, downPad, 400), []);
    assert.deepEqual(pollGamepadEvents(state, downPad, 480), [{ kind: 'direction', direction: 'down' }]);

    assert.deepEqual(pollGamepadEvents(state, [makePad()], 500), []);
    assert.deepEqual(pollGamepadEvents(state, downPad, 520), [{ kind: 'direction', direction: 'down' }]);
  });

  it('resets held state when paused or disconnected to prevent phantom events', () => {
    const state = createGamepadInputState();

    assert.deepEqual(pollGamepadEvents(state, [makePad({ pressedButtons: [0, 14] })], 0), [
      { kind: 'action', code: 'GamepadA' },
      { kind: 'direction', direction: 'left' }
    ]);

    assert.deepEqual(pollGamepadEvents(state, [makePad({ pressedButtons: [0, 14] })], 400, { paused: true }), []);
    assert.deepEqual(pollGamepadEvents(state, [null], 420), []);

    assert.deepEqual(pollGamepadEvents(state, [makePad({ pressedButtons: [14] })], 450), [
      { kind: 'direction', direction: 'left' }
    ]);
  });
});
