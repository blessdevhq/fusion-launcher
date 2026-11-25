import assert from 'node:assert/strict';
import { it } from 'node:test';
import { isInputOwnerActive } from './focusManager.ts';
import * as dialog from './creatorDialogFocus.ts';

// Minimal DOM boundary: no DOM package or fabricated layout/navigation algorithm.
class Element {
  ownerDocument: DocumentBoundary;
  parent: Element | null;
  children: Element[] = [];
  attributes = new Map<string, string>();
  tabIndex = 0;
  isConnected = true;
  isContentEditable = false;
  tagName: string;
  rect = { left: 0, top: 0, width: 10, height: 10 };
  constructor(doc: DocumentBoundary, tag = 'BUTTON', parent: Element | null = null) {
    this.ownerDocument = doc; this.tagName = tag; this.parent = parent; parent?.children.push(this);
  }
  contains(other: Element | null): boolean { return other === this || this.children.some((child) => child.contains(other)); }
  hasAttribute(name: string) { return this.attributes.has(name); }
  getAttribute(name: string) { return this.attributes.get(name) ?? null; }
  setAttribute(name: string, value: string) { this.attributes.set(name, value); }
  removeAttribute(name: string) { this.attributes.delete(name); }
  matches(selector: string) { return selector === ':disabled' && this.hasAttribute('disabled'); }
  closest(selector: string): Element | null {
    const match = selector.includes('data-creator-dialog-scope')
      ? this.hasAttribute('data-creator-dialog-scope')
      : ['hidden', 'inert'].some((attr) => this.hasAttribute(attr)) || this.getAttribute('aria-hidden') === 'true';
    return match ? this : this.parent?.closest(selector) ?? null;
  }
  querySelectorAll() { return this.children.flatMap((child) => [child, ...child.querySelectorAll()]); }
  getClientRects() { return this.hasAttribute('hidden') ? [] : [this.rect]; }
  getBoundingClientRect() { return this.rect; }
  focus() { this.ownerDocument.activeElement = this; }
  click() { this.ownerDocument.clicks++; }
}
class DocumentBoundary extends EventTarget {
  activeElement: Element | null = null;
  clicks = 0;
  key(key: string, shiftKey = false, repeat = false) {
    const event = new Event('keydown', { cancelable: true });
    Object.defineProperties(event, {
      key: { value: key }, code: { value: key === ' ' ? 'Space' : key },
      shiftKey: { value: shiftKey }, repeat: { value: repeat },
      target: { value: this.activeElement }
    });
    this.dispatchEvent(event);
    return event;
  }
}
const mount = (root: Element, close: () => void, options = {}) =>
  dialog.mountCreatorDialogFocus(root as unknown as HTMLElement, close, options);

it('traps Tab in a dialog and restores the opener on cleanup', () => {
  const doc = new DocumentBoundary();
  const opener = new Element(doc); opener.focus();
  const root = new Element(doc, 'SECTION');
  const first = new Element(doc, 'BUTTON', root);
  const last = new Element(doc, 'BUTTON', root);
  const dispose = mount(root, () => {});
  try {
    assert.equal(isInputOwnerActive(), false);
    assert.equal(doc.activeElement, first);
    assert.equal(doc.key('Tab').defaultPrevented, true);
    assert.equal(doc.activeElement, last);
    doc.key('Tab'); assert.equal(doc.activeElement, first);
    doc.key('Tab', true); assert.equal(doc.activeElement, last);
  } finally { dispose(); }
  assert.equal(doc.activeElement, opener);
  assert.equal(isInputOwnerActive(), true);
  assert.equal(root.hasAttribute('data-creator-dialog-scope'), false);
});

it('lets only the top nested dialog close and restores its parent opener', () => {
  const doc = new DocumentBoundary();
  const parent = new Element(doc, 'SECTION');
  const parentButton = new Element(doc, 'BUTTON', parent);
  let parentClosed = 0;
  const disposeParent = mount(parent, () => { parentClosed++; });
  const child = new Element(doc, 'SECTION', parent);
  const childButton = new Element(doc, 'BUTTON', child);
  let childClosed = 0;
  const disposeChild = mount(child, () => { childClosed++; });
  try {
    assert.equal(doc.activeElement, childButton);
    doc.key('Tab'); assert.equal(doc.activeElement, childButton);
    assert.equal(doc.key('Escape').defaultPrevented, true);
    assert.equal(childClosed, 1);
    assert.equal(parentClosed, 0);
    doc.key('Escape', false, true);
    assert.equal(childClosed, 1, 'held Escape cannot close the next dialog');
    disposeChild();
    assert.equal(doc.activeElement, parentButton);
    doc.key('Escape', false, true);
    assert.equal(parentClosed, 0, 'repeat does not close restored parent');
    doc.key('Escape'); assert.equal(parentClosed, 1);
  } finally { disposeChild(); disposeParent(); }
});

it('navigates only dialog controls while preserving native activation and editable arrows', () => {
  const doc = new DocumentBoundary();
  const shell = new Element(doc); shell.rect.left = 15;
  const root = new Element(doc, 'SECTION');
  const first = new Element(doc, 'BUTTON', root);
  const last = new Element(doc, 'BUTTON', root); last.rect.left = 30;
  const input = new Element(doc, 'INPUT', root); input.rect.top = 30;
  const dispose = mount(root, () => {});
  try {
    assert.equal(doc.key('ArrowRight').defaultPrevented, true);
    assert.equal(doc.activeElement, last);
    assert.equal(doc.key('Enter').defaultPrevented, false, 'native click remains the only activation');
    assert.equal(doc.clicks, 0, 'no synthetic duplicate click');
    input.focus();
    assert.equal(doc.key('ArrowDown').defaultPrevented, false);
    assert.equal(doc.activeElement, input);
    first.focus();
  } finally { dispose(); }
});

it('pauses arrows and native activation during capture without releasing shell isolation', () => {
  const doc = new DocumentBoundary();
  const root = new Element(doc, 'SECTION');
  const first = new Element(doc, 'BUTTON', root);
  const second = new Element(doc, 'BUTTON', root); second.rect.left = 30;
  const options = { capture: true };
  const dispose = mount(root, () => {}, options);
  try {
    for (const key of ['ArrowRight', 'Enter', ' ']) assert.equal(doc.key(key).defaultPrevented, true);
    assert.equal(doc.activeElement, first);
    assert.equal(isInputOwnerActive(), false);
    doc.key('Tab'); assert.equal(doc.activeElement, second, 'cancel controls remain keyboard reachable');
    options.capture = false;
    first.focus(); doc.key('ArrowRight'); assert.equal(doc.activeElement, second);
  } finally { dispose(); }
});

it('keeps a child topmost when layout effects register the parent after the child', () => {
  const doc = new DocumentBoundary();
  const opener = new Element(doc); opener.focus();
  const parent = new Element(doc, 'SECTION');
  const parentButton = new Element(doc, 'BUTTON', parent);
  const child = new Element(doc, 'SECTION', parent);
  const childButton = new Element(doc, 'BUTTON', child);
  let childClosed = 0;
  let parentClosed = 0;
  const disposeChild = mount(child, () => { childClosed++; });
  const disposeParent = mount(parent, () => { parentClosed++; });
  try {
    assert.equal(doc.activeElement, childButton);
    doc.key('Escape');
    assert.equal(childClosed, 1);
    assert.equal(parentClosed, 0);
    disposeChild();
    assert.equal(doc.activeElement, parentButton);
  } finally { disposeChild(); disposeParent(); }
});

it('recaptures leaked focus and contains Tab even when there are no enabled controls', () => {
  const doc = new DocumentBoundary();
  const opener = new Element(doc); opener.focus();
  const root = new Element(doc, 'SECTION');
  const disabled = new Element(doc, 'BUTTON', root); disabled.setAttribute('disabled', '');
  const hidden = new Element(doc, 'BUTTON', root); hidden.setAttribute('hidden', '');
  const dispose = mount(root, () => {});
  try {
    assert.equal(doc.activeElement, root);
    opener.focus();
    const event = new Event('focusin');
    Object.defineProperty(event, 'target', { value: opener });
    doc.dispatchEvent(event);
    assert.equal(doc.activeElement, root);
    assert.equal(doc.key('Tab').defaultPrevented, true);
    assert.equal(doc.activeElement, root);
    opener.isConnected = false;
  } finally { dispose(); dispose(); }
  assert.equal(doc.activeElement, root, 'disconnected opener is not focused');
});
