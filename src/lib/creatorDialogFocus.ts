import { acquireInputOwner, getActiveInputScope, isInputOwnerActive, nextFocusTarget } from './focusManager.ts';
import { gamepadCommandForKey } from './gamepadInput.ts';

export interface CreatorDialogFocusOptions {
  /** Keep the trap and Escape, but pause arrow navigation and activation during assignment. */
  capture?: boolean;
}

const SCOPE_ATTRIBUTE = 'data-creator-dialog-scope';
const FOCUSABLE = 'button, [href], input:not([type="hidden"]), select, textarea, [tabindex], [contenteditable="true"]';

function focusableElements(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE))
    .filter((element) => element.tabIndex >= 0 && !element.matches(':disabled') &&
      !element.closest('[hidden], [inert], [aria-hidden="true"]') &&
      element.closest(`[${SCOPE_ATTRIBUTE}]`) === root && element.getClientRects().length > 0)
    .sort((a, b) => (a.tabIndex > 0 ? a.tabIndex : Infinity) - (b.tabIndex > 0 ? b.tabIndex : Infinity));
}

/** Imperative boundary used by the React hook; returns an idempotent cleanup. */
export function mountCreatorDialogFocus(
  root: HTMLElement,
  onClose: () => void,
  options: CreatorDialogFocusOptions = {}
): () => void {
  const doc = root.ownerDocument;
  const previous = doc.activeElement as HTMLElement | null;
  const owner = Symbol('creator-dialog');
  const release = acquireInputOwner(owner, root);
  const previousScope = root.getAttribute(SCOPE_ATTRIBUTE);
  const previousTabIndex = root.getAttribute('tabindex');
  root.setAttribute(SCOPE_ATTRIBUTE, '');
  if (previousTabIndex === null) root.setAttribute('tabindex', '-1');
  const focusFirst = () => (focusableElements(root)[0] ?? root).focus();
  const onKeyDown = (event: KeyboardEvent) => {
    if (!isInputOwnerActive(owner)) return;
    if (event.key === 'Escape' && !event.isComposing) {
      event.preventDefault();
      event.stopImmediatePropagation();
      if (!event.repeat) onClose();
      return;
    }
    if (event.key === 'Tab') {
      event.preventDefault();
      event.stopImmediatePropagation();
      const elements = focusableElements(root);
      const index = elements.indexOf(doc.activeElement as HTMLElement);
      const next = index < 0 ? (event.shiftKey ? elements.length - 1 : 0)
        : (index + (event.shiftKey ? -1 : 1) + elements.length) % elements.length;
      (elements[next] ?? root).focus();
      return;
    }
    const command = gamepadCommandForKey(event, { inputOwner: owner });
    if (options.capture && command) {
      event.preventDefault();
      event.stopImmediatePropagation();
      return;
    }
    const target = doc.activeElement as HTMLElement | null;
    const editable = target && (['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName) || target.isContentEditable);
    if (!editable && (command === 'left' || command === 'right' || command === 'up' || command === 'down')) {
      event.preventDefault();
      event.stopImmediatePropagation();
      const elements = focusableElements(root);
      const targets = elements.map((element, index) => ({
        id: String(index), zone: 'dialog', scope: owner, rect: element.getBoundingClientRect()
      }));
      const next = nextFocusTarget(targets, String(elements.indexOf(target as HTMLElement)), command, owner);
      (elements[Number(next?.id)] ?? root).focus();
    }
  };
  const onFocusIn = (event: FocusEvent) => {
    if (isInputOwnerActive(owner) && !root.contains(event.target as Node | null)) focusFirst();
  };
  doc.addEventListener('keydown', onKeyDown, true);
  doc.addEventListener('focusin', onFocusIn, true);
  if (isInputOwnerActive(owner)) focusFirst();
  let disposed = false;
  return () => {
    if (disposed) return;
    disposed = true;
    const wasTop = isInputOwnerActive(owner);
    doc.removeEventListener('keydown', onKeyDown, true);
    doc.removeEventListener('focusin', onFocusIn, true);
    release();
    if (previousScope === null) root.removeAttribute(SCOPE_ATTRIBUTE);
    else root.setAttribute(SCOPE_ATTRIBUTE, previousScope);
    if (previousTabIndex === null) root.removeAttribute('tabindex');
    if (wasTop) {
      const activeScope = getActiveInputScope();
      if (previous?.isConnected && (activeScope ? activeScope.contains(previous) : isInputOwnerActive())) previous.focus();
      else if (activeScope) (focusableElements(activeScope)[0] ?? activeScope).focus();
    }
  };
}
