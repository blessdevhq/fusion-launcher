import { actionForInput, isInputOwnerActive, type FocusDirection, type InputOwner } from './focusManager.ts';

export interface InputOwnershipOptions {
  inputOwner?: InputOwner;
  enabled?: boolean;
}

const DIRECTION_KEYS: Record<string, FocusDirection> = {
  ArrowLeft: 'left', ArrowRight: 'right', ArrowUp: 'up', ArrowDown: 'down'
};

/** Shared, DOM-free routing gate. The owner is checked at event time, not render time. */
export function gamepadCommandForKey(
  event: { code: string; key: string; defaultPrevented?: boolean; isComposing?: boolean;
    nativeActivation?: 'link' | 'enter-space'; nativeControlIsShellTarget?: boolean },
  { inputOwner, enabled = true }: InputOwnershipOptions = {}
) {
  if (!enabled || !isInputOwnerActive(inputOwner) || event.defaultPrevented || event.isComposing) return null;
  if (event.nativeActivation && (event.code === 'Enter' || event.key === 'Enter')) return null;
  if ((event.code === 'Space' || event.key === ' ') && event.nativeActivation
    && !(event.nativeActivation === 'link' && event.nativeControlIsShellTarget)) return null;
  return DIRECTION_KEYS[event.code] ?? DIRECTION_KEYS[event.key] ?? actionForInput(event.code || event.key);
}

export interface GamepadButtonSnapshot {
  pressed: boolean;
  value?: number;
}

export interface GamepadSnapshot {
  connected: boolean;
  buttons: readonly GamepadButtonSnapshot[];
  axes: readonly number[];
}

export type GamepadActionCode = 'GamepadA' | 'GamepadB' | 'GamepadX' | 'GamepadY';

export type GamepadInputEvent =
  | { kind: 'direction'; direction: FocusDirection }
  | { kind: 'action'; code: GamepadActionCode };

export interface GamepadPollOptions {
  deadzone?: number;
  initialRepeatDelayMs?: number;
  repeatIntervalMs?: number;
  paused?: boolean;
}

export interface GamepadInputState {
  pressedActions: Record<GamepadActionCode, boolean>;
  heldDirection: FocusDirection | null;
  nextRepeatAtMs: number;
}

const ACTION_BUTTONS: ReadonlyArray<{ index: number; code: GamepadActionCode }> = [
  { index: 0, code: 'GamepadA' },
  { index: 1, code: 'GamepadB' },
  { index: 2, code: 'GamepadX' },
  { index: 3, code: 'GamepadY' }
];

const DPAD_BUTTONS: ReadonlyArray<{ index: number; direction: FocusDirection }> = [
  { index: 12, direction: 'up' },
  { index: 13, direction: 'down' },
  { index: 14, direction: 'left' },
  { index: 15, direction: 'right' }
];

export function createGamepadInputState(): GamepadInputState {
  return {
    pressedActions: {
      GamepadA: false,
      GamepadB: false,
      GamepadX: false,
      GamepadY: false
    },
    heldDirection: null,
    nextRepeatAtMs: 0
  };
}

export function pollGamepadEvents(
  state: GamepadInputState,
  gamepads: readonly (GamepadSnapshot | null | undefined)[],
  nowMs: number,
  options: GamepadPollOptions = {}
): GamepadInputEvent[] {
  const deadzone = options.deadzone ?? 0.55;
  const initialRepeatDelayMs = options.initialRepeatDelayMs ?? 360;
  const repeatIntervalMs = options.repeatIntervalMs ?? 120;

  if (options.paused) {
    resetState(state);
    return [];
  }

  const activeGamepad = gamepads.find((pad): pad is GamepadSnapshot => Boolean(pad && pad.connected));
  if (!activeGamepad) {
    resetState(state);
    return [];
  }

  const events: GamepadInputEvent[] = [];

  for (const { index, code } of ACTION_BUTTONS) {
    const isPressed = isButtonPressed(activeGamepad.buttons[index]);
    if (isPressed && !state.pressedActions[code]) {
      events.push({ kind: 'action', code });
    }
    state.pressedActions[code] = isPressed;
  }

  const direction = readDirection(activeGamepad, deadzone);
  if (!direction) {
    state.heldDirection = null;
    state.nextRepeatAtMs = 0;
  } else if (direction !== state.heldDirection) {
    state.heldDirection = direction;
    state.nextRepeatAtMs = nowMs + initialRepeatDelayMs;
    events.push({ kind: 'direction', direction });
  } else if (nowMs >= state.nextRepeatAtMs) {
    state.nextRepeatAtMs = nowMs + repeatIntervalMs;
    events.push({ kind: 'direction', direction });
  }

  return events;
}

function resetState(state: GamepadInputState) {
  state.pressedActions.GamepadA = false;
  state.pressedActions.GamepadB = false;
  state.pressedActions.GamepadX = false;
  state.pressedActions.GamepadY = false;
  state.heldDirection = null;
  state.nextRepeatAtMs = 0;
}

function isButtonPressed(button: GamepadButtonSnapshot | undefined): boolean {
  if (!button) return false;
  return Boolean(button.pressed || (typeof button.value === 'number' && button.value > 0.5));
}

function readDirection(gamepad: GamepadSnapshot, deadzone: number): FocusDirection | null {
  for (const { index, direction } of DPAD_BUTTONS) {
    if (isButtonPressed(gamepad.buttons[index])) {
      return direction;
    }
  }

  const axisX = gamepad.axes[0] ?? 0;
  const axisY = gamepad.axes[1] ?? 0;
  const absX = Math.abs(axisX);
  const absY = Math.abs(axisY);

  if (absX < deadzone && absY < deadzone) {
    return null;
  }

  if (absX >= absY) {
    return axisX < 0 ? 'left' : 'right';
  }
  return axisY < 0 ? 'up' : 'down';
}
