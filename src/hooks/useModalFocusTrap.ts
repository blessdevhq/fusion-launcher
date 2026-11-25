'use client';

import { useEffect, useRef, type RefObject } from 'react';

const FOCUSABLE_SELECTOR = [
  'button:not(:disabled)',
  'a[href]',
  'input:not(:disabled)',
  'select:not(:disabled)',
  'textarea:not(:disabled)',
  'summary',
  '[tabindex]:not([tabindex="-1"])'
].join(',');

export function getTopmostModal(): HTMLElement | null {
  if (typeof document === 'undefined') return null;
  const modals = Array.from(document.querySelectorAll<HTMLElement>('[aria-modal="true"]')).filter(
    (node) => node.getClientRects().length > 0
  );
  return modals.at(-1) ?? null;
}

export function getModalFocusableElements(container: HTMLElement): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
    (node) => node.getClientRects().length > 0
  );
}

export function useModalFocusTrap({
  containerRef,
  onClose,
  initialFocusRef,
  active = true
}: {
  containerRef: RefObject<HTMLElement | null>;
  onClose: () => void;
  initialFocusRef?: RefObject<HTMLElement | null>;
  active?: boolean;
}) {
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    if (!active) return undefined;
    const container = containerRef.current;
    if (!container) return undefined;

    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const initialTarget = initialFocusRef?.current ?? container;
    initialTarget.focus({ preventScroll: true });

    const handleKeyDown = (event: KeyboardEvent) => {
      if (getTopmostModal() !== container) return;

      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        onCloseRef.current();
        return;
      }

      if (event.key !== 'Tab') return;
      const controls = getModalFocusableElements(container);
      if (controls.length === 0) {
        event.preventDefault();
        container.focus({ preventScroll: true });
        return;
      }

      const first = controls[0];
      const last = controls[controls.length - 1];
      if (!container.contains(document.activeElement)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
        return;
      }

      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('keydown', handleKeyDown);
      if (previous && previous.isConnected) {
        previous.focus({ preventScroll: true });
      }
    };
  }, [active, containerRef, initialFocusRef]);
}
