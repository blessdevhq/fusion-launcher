'use client';

import { useEffect, type RefObject } from 'react';
import {
  nextFocusTarget,
  type FocusTarget
} from '../lib/focusManager.ts';
import { gamepadCommandForKey, type InputOwnershipOptions } from '../lib/gamepadInput.ts';

interface UseGamepadOptions extends InputOwnershipOptions {
  focusedItemId: string | null;
  setFocusedItemId: (id: string | null) => void;
  onActivate: (id: string) => void;
  onBack: () => void;
  onMenu: (id: string | null) => void;
  /** Limit collection and focus lookup to this root when using an explicit inputOwner. */
  focusRoot?: RefObject<HTMLElement | null>;
}

export function useGamepad({
  focusedItemId,
  setFocusedItemId,
  onActivate,
  onBack,
  onMenu,
  inputOwner,
  enabled = true,
  focusRoot
}: UseGamepadOptions) {
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const target = event.target;
      const control = target instanceof Element ? target.closest('button, a[href], summary') : null;
      const command = gamepadCommandForKey({
        code: event.code, key: event.key, defaultPrevented: event.defaultPrevented, isComposing: event.isComposing,
        nativeActivation: control ? (control.matches('a[href]') ? 'link' : 'enter-space') : undefined,
        nativeControlIsShellTarget: !!focusedItemId && control?.getAttribute('data-focus-id') === focusedItemId
      }, { inputOwner, enabled });
      if (!command) return;
      const root = focusRoot ? focusRoot.current : document;
      if (!root) return;
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
          onBack();
        }
        return;
      }

      if (command === 'left' || command === 'right' || command === 'up' || command === 'down') {
        const targets = collectFocusTargets(root);
        const next = nextFocusTarget(targets, focusedItemId, command);
        if (next) {
          event.preventDefault();
          setFocusedItemId(next.id);
          // Compare IDs as data, not selector syntax (also safe without CSS.escape).
          Array.from(root.querySelectorAll<HTMLElement>('[data-focus-id]'))
            .find((element) => element.dataset.focusId === next.id)?.focus({ preventScroll: false });
        }
        return;
      }

      event.preventDefault();
      if (command === 'activate' && focusedItemId) onActivate(focusedItemId);
      if (command === 'back') onBack();
      if (command === 'menu') onMenu(focusedItemId);
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [focusedItemId, onActivate, onBack, onMenu, setFocusedItemId, inputOwner, enabled, focusRoot]);
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
