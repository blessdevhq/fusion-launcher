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
  event: { code: string; key: string; defaultPrevented?: boolean; isComposing?: boolean },
  { inputOwner, enabled = true }: InputOwnershipOptions = {}
) {
  if (!enabled || !isInputOwnerActive(inputOwner) || event.defaultPrevented || event.isComposing) return null;
  return DIRECTION_KEYS[event.code] ?? DIRECTION_KEYS[event.key] ?? actionForInput(event.code || event.key);
}
