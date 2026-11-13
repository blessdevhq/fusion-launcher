'use client';

import { useLayoutEffect, useRef, type RefObject } from 'react';
import { mountCreatorDialogFocus, type CreatorDialogFocusOptions } from '../lib/creatorDialogFocus.ts';

/**
 * Call once in each mounted dialog, including nested assignment dialogs.
 * Attach ref to the dialog root; put the intended initial control first in DOM order.
 * onClose requests closure; the owner/trap remain held until this component unmounts
 * (including while native stop/release ACK is pending). No extra useGamepad is needed.
 * Native buttons retain their normal onClick: do not also activate them on keydown.
 * capture pauses arrows/activation but keeps Tab and Escape available.
 */
export function useCreatorDialogFocus(
  ref: RefObject<HTMLElement | null>,
  onClose: () => void,
  options: CreatorDialogFocusOptions = {}
): void {
  const current = useRef({ onClose, options });
  current.current = { onClose, options };
  useLayoutEffect(() => {
    const root = ref.current;
    if (!root) return;
    return mountCreatorDialogFocus(root, () => current.current.onClose(), {
      get capture() { return current.current.options.capture; }
    });
  }, [ref]);
}
