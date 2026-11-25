'use client';

import { useEffect, useRef, type RefObject } from 'react';
import {
  actionForInput,
  isInputOwnerActive,
  nextFocusTarget,
  type FocusTarget
} from '../lib/focusManager.ts';
import {
  createGamepadInputState,
  gamepadCommandForKey,
  pollGamepadEvents,
  type GamepadSnapshot,
  type InputOwnershipOptions
} from '../lib/gamepadInput.ts';
import { getModalFocusableElements, getTopmostModal } from './useModalFocusTrap.ts';

interface UseGamepadOptions extends InputOwnershipOptions {
  focusedItemId: string | null;
  setFocusedItemId: (id: string | null) => void;
  onActivate: (id: string) => void;
  onBack: () => void;
  onMenu: (id: string | null) => void;
  onSecondary?: (id: string | null) => void;
  /** Limit collection and focus lookup to this root when using an explicit inputOwner. */
  focusRoot?: RefObject<HTMLElement | null>;
}

export function useGamepad({
  focusedItemId,
  setFocusedItemId,
  onActivate,
  onBack,
  onMenu,
  onSecondary,
  inputOwner,
  enabled = true,
  focusRoot
}: UseGamepadOptions) {
  const focusedItemIdRef = useRef(focusedItemId);
  const setFocusedItemIdRef = useRef(setFocusedItemId);
  const onActivateRef = useRef(onActivate);
  const onBackRef = useRef(onBack);
  const onMenuRef = useRef(onMenu);
  const onSecondaryRef = useRef(onSecondary);
  const inputOwnerRef = useRef(inputOwner);
  const enabledRef = useRef(enabled);
  const focusRootRef = useRef(focusRoot);

  focusedItemIdRef.current = focusedItemId;
  setFocusedItemIdRef.current = setFocusedItemId;
  onActivateRef.current = onActivate;
  onBackRef.current = onBack;
  onMenuRef.current = onMenu;
  onSecondaryRef.current = onSecondary;
  inputOwnerRef.current = inputOwner;
  enabledRef.current = enabled;
  focusRootRef.current = focusRoot;

  useEffect(() => {
    const dispatchAction = (action: ReturnType<typeof actionForInput>) => {
      if (!action) return;
      if (action === 'activate' && focusedItemIdRef.current) {
        onActivateRef.current(focusedItemIdRef.current);
      } else if (action === 'back') {
        onBackRef.current();
      } else if (action === 'menu') {
        onMenuRef.current(focusedItemIdRef.current);
      } else if (action === 'secondary') {
        onSecondaryRef.current?.(focusedItemIdRef.current);
      }
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (!enabledRef.current || !isInputOwnerActive(inputOwnerRef.current)) return;

      const topModal = getTopmostModal();
      if (topModal) {
        if (event.key === 'Escape' && !event.defaultPrevented) {
          event.preventDefault();
          onBackRef.current();
        }
        return;
      }

      const target = event.target;
      // Native selects own arrows, activation and Escape; do not route them to the shell.
      if (target instanceof HTMLElement && target.tagName === 'SELECT') return;
      const isEditable = target instanceof HTMLElement && (
        target.tagName === 'INPUT' ||
        target.tagName === 'TEXTAREA' ||
        target.isContentEditable
      );

      if (isEditable) {
        if (event.key === 'Escape') {
          target.blur();
          onBackRef.current();
        }
        return;
      }

      const control = target instanceof Element ? target.closest('button, a[href], summary') : null;
      const command = gamepadCommandForKey({
        code: event.code,
        key: event.key,
        defaultPrevented: event.defaultPrevented,
        isComposing: event.isComposing,
        nativeActivation: control ? (control.matches('a[href]') ? 'link' : 'enter-space') : undefined,
        nativeControlIsShellTarget: Boolean(focusedItemIdRef.current) && control?.getAttribute('data-focus-id') === focusedItemIdRef.current
      }, { inputOwner: inputOwnerRef.current, enabled: enabledRef.current });
      if (!command) return;

      const root = focusRootRef.current ? focusRootRef.current.current : document;
      if (!root) return;

      if (command === 'left' || command === 'right' || command === 'up' || command === 'down') {
        const targets = collectFocusTargets(root);
        const next = nextFocusTarget(targets, focusedItemIdRef.current, command);
        if (next) {
          event.preventDefault();
          setFocusedItemIdRef.current(next.id);
          // Compare IDs as data, not selector syntax (also safe without CSS.escape).
          Array.from(root.querySelectorAll<HTMLElement>('[data-focus-id]'))
            .find((element) => element.dataset.focusId === next.id)?.focus({ preventScroll: false });
        }
        return;
      }

      event.preventDefault();
      dispatchAction(command);
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, []);

  useEffect(() => {
    if (typeof navigator === 'undefined' || typeof navigator.getGamepads !== 'function') {
      return undefined;
    }

    const inputState = createGamepadInputState();
    let rafId: number | null = null;
    let active = true;

    const readPads = (): readonly (GamepadSnapshot | null)[] => {
      try {
        return Array.from(navigator.getGamepads() ?? []);
      } catch {
        return [];
      }
    };

    const hasConnectedGamepad = () => readPads().some((pad) => Boolean(pad && pad.connected));

    const handleModalGamepadEvent = (topModal: HTMLElement, event: ReturnType<typeof pollGamepadEvents>[number]) => {
      if (event.kind === 'direction') {
        const controls = getModalFocusableElements(topModal);
        if (controls.length === 0) return;

        const targets: FocusTarget[] = controls.map((element, index) => {
          const rect = element.getBoundingClientRect();
          return {
            id: `modal-control:${index}`,
            zone: 'modal',
            rect: {
              left: rect.left,
              top: rect.top,
              width: rect.width,
              height: rect.height
            }
          };
        });

        const currentIndex = document.activeElement instanceof HTMLElement
          ? controls.indexOf(document.activeElement)
          : -1;
        const currentId = currentIndex >= 0 ? `modal-control:${currentIndex}` : null;
        const next = nextFocusTarget(targets, currentId, event.direction);
        if (!next) return;

        const nextIndex = Number.parseInt(next.id.slice('modal-control:'.length), 10);
        controls[nextIndex]?.focus({ preventScroll: false });
        return;
      }

      if (event.code === 'GamepadA') {
        const activeEl = document.activeElement;
        if (activeEl instanceof HTMLElement && topModal.contains(activeEl)) {
          activeEl.click();
        } else {
          getModalFocusableElements(topModal)[0]?.focus({ preventScroll: false });
        }
      } else if (event.code === 'GamepadB') {
        onBackRef.current();
      }
    };

    const tick = (nowMs: number) => {
      if (!active) return;

      const pads = readPads();
      const connected = pads.some((pad) => Boolean(pad && pad.connected));
      const events = pollGamepadEvents(inputState, pads, nowMs, {
        paused: document.hidden || !document.hasFocus() || !enabledRef.current || !isInputOwnerActive(inputOwnerRef.current)
      });

      for (const event of events) {
        const topModal = getTopmostModal();
        if (topModal) {
          handleModalGamepadEvent(topModal, event);
          continue;
        }

        const root = focusRootRef.current ? focusRootRef.current.current : document;
        if (!root) continue;

        if (event.kind === 'direction') {
          const targets = collectFocusTargets(root);
          const next = nextFocusTarget(targets, focusedItemIdRef.current, event.direction);
          if (next) {
            setFocusedItemIdRef.current(next.id);
            Array.from(root.querySelectorAll<HTMLElement>('[data-focus-id]'))
              .find((element) => element.dataset.focusId === next.id)?.focus({ preventScroll: false });
          }
          continue;
        }

        const action = actionForInput(event.code);
        if (action === 'activate' && focusedItemIdRef.current) {
          onActivateRef.current(focusedItemIdRef.current);
        } else if (action === 'back') {
          onBackRef.current();
        } else if (action === 'menu') {
          onMenuRef.current(focusedItemIdRef.current);
        } else if (action === 'secondary') {
          onSecondaryRef.current?.(focusedItemIdRef.current);
        }
      }

      if (connected && active) {
        rafId = window.requestAnimationFrame(tick);
      } else {
        rafId = null;
      }
    };

    const startPolling = () => {
      if (rafId !== null || !active || !hasConnectedGamepad()) return;
      rafId = window.requestAnimationFrame(tick);
    };

    const stopPolling = () => {
      if (rafId !== null) {
        window.cancelAnimationFrame(rafId);
        rafId = null;
      }
      if (!hasConnectedGamepad()) {
        pollGamepadEvents(inputState, [], performance.now(), { paused: true });
      }
    };

    const handleConnect = () => startPolling();
    const handleDisconnect = () => {
      if (!hasConnectedGamepad()) stopPolling();
    };

    window.addEventListener('gamepadconnected', handleConnect);
    window.addEventListener('gamepaddisconnected', handleDisconnect);
    startPolling();

    return () => {
      active = false;
      window.removeEventListener('gamepadconnected', handleConnect);
      window.removeEventListener('gamepaddisconnected', handleDisconnect);
      if (rafId !== null) window.cancelAnimationFrame(rafId);
    };
  }, []);
}

function collectFocusTargets(root: ParentNode): FocusTarget[] {
  return Array.from(root.querySelectorAll<HTMLElement>('[data-focus-id]'))
    .filter((element) => !element.hasAttribute('disabled') && element.offsetParent !== null)
    .map((element) => {
      const rect = element.getBoundingClientRect();
      return {
        id: element.dataset.focusId ?? '',
        zone: element.dataset.focusZone ?? 'global',
        rect: {
          left: rect.left,
          top: rect.top,
          width: rect.width,
          height: rect.height
        }
      };
    })
    .filter((target) => target.id);
}
