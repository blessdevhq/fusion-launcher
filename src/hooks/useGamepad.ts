'use client';

import { useEffect } from 'react';
import {
  actionForInput,
  nextFocusTarget,
  type FocusDirection,
  type FocusTarget
} from '../lib/focusManager.ts';

interface UseGamepadOptions {
  focusedItemId: string | null;
  setFocusedItemId: (id: string | null) => void;
  onActivate: (id: string) => void;
  onBack: () => void;
  onSearch: () => void;
  onMenu: (id: string | null) => void;
}

const DIRECTION_KEYS: Record<string, FocusDirection> = {
  ArrowLeft: 'left',
  ArrowRight: 'right',
  ArrowUp: 'up',
  ArrowDown: 'down'
};

export function useGamepad({
  focusedItemId,
  setFocusedItemId,
  onActivate,
  onBack,
  onSearch,
  onMenu
}: UseGamepadOptions) {
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const direction = DIRECTION_KEYS[event.code] ?? DIRECTION_KEYS[event.key];
      if (direction) {
        const targets = collectFocusTargets();
        const next = nextFocusTarget(targets, focusedItemId, direction);
        if (next) {
          event.preventDefault();
          setFocusedItemId(next.id);
          document.querySelector<HTMLElement>(`[data-focus-id="${cssEscape(next.id)}"]`)?.focus({ preventScroll: false });
        }
        return;
      }

      const action = actionForInput(event.code);
      if (!action) return;

      event.preventDefault();
      if (action === 'activate' && focusedItemId) onActivate(focusedItemId);
      if (action === 'back') onBack();
      if (action === 'search') onSearch();
      if (action === 'menu') onMenu(focusedItemId);
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [focusedItemId, onActivate, onBack, onMenu, onSearch, setFocusedItemId]);
}

function collectFocusTargets(): FocusTarget[] {
  return Array.from(document.querySelectorAll<HTMLElement>('[data-focus-id]'))
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

function cssEscape(value: string) {
  if (typeof CSS !== 'undefined' && CSS.escape) {
    return CSS.escape(value);
  }
  return value.replace(/"/g, '\\"');
}
