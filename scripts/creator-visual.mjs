import assert from 'node:assert/strict';
import { expect } from '@playwright/test';

const viewports = [
  { width: 1280, height: 720 },
  { width: 1365, height: 768 },
  { width: 1440, height: 900 },
  { width: 1920, height: 1080 }
];
const stages = ['base', 'composition', 'profiles', 'verification', 'release'];
const labels = {
  en: ['Base', 'Composition', 'Profiles', 'Verification', 'Release'],
  ru: ['Основа', 'Состав', 'Профили', 'Проверка', 'Выпуск']
};
const unavailable = { en: /unavailable|not available|not implemented|deferred/i, ru: /недоступ|не реализован|отложен/i };
const emptySuccess = /\bno (?:projects|devices|jobs)\b|\b(?:projects|devices|jobs) (?:not found|list is empty)\b|нет (?:проектов|устройств|заданий)|(?:список|списки) (?:пуст|пусты)/i;
const settingsClose = /^(?:Close settings|Закрыть настройки)$/;
const saveLabel = /^(?:Save|Сохранить)$/;
const demoSourceUrl = 'https://demo.fusion.invalid/index.json';

/**
 * Called by mvp-visual with its existing helpers; does not launch a server/browser.
 * Uses the production-default Creator bridge: unavailable is not an empty success.
 * The existing Fusion source preview is used ONLY to reach Fusion game details via
 * real settings interactions, never to synthesize Creator projects/devices/jobs.
 * Entry may have Settings open. Exit is Home, English, original viewport, no dialogs.
 * No baselines, visual roles, global API injection or native capture are approved here.
 */
export async function runCreatorVisual(page, { screenshot, assertNoHorizontalOverflow, expectVisible, clickNav }) {
  const originalViewport = page.viewportSize();
  let addedDemoSource = false;
  let failure;
  try {
    await setLocale(page, 'en');
    await clickNav(page, 'creator');
    await expectVisible(page.getByTestId('nav-creator'), 'Creator navigation entry');
    await expectVisible(page.getByTestId('creator-screen'), 'Creator screen');
    await expect(page.getByTestId('creator-context-title')).toHaveCount(0);

    for (const locale of ['en', 'ru']) {
      await setLocale(page, locale);
      await clickNav(page, 'creator');
      await assertUnavailable(page.getByTestId('creator-projects-state'), locale, 'Creator project list');
      // A container can remain mounted; it must not invent selectable projects.
      await expect(page.getByTestId('creator-project-list').locator('[role="listitem"], button, [data-focus-id]')).toHaveCount(0);
      // No command has been called: this is idle, not a fabricated unavailable reply.
      await expect(page.getByTestId('creator-jobs-state')).toHaveAttribute('data-state', 'idle');
      await expect(page.getByTestId('creator-jobs-state')).toContainText(locale === 'ru' ? 'Не запрошено' : 'Not requested');
      await expect(page.getByTestId('creator-screen')).toContainText(locale === 'ru'
        ? 'В этой сессии задания не открывались.' : 'No jobs opened in this session.');
      await expectVisible(page.getByTestId('creator-job-id'), 'Creator job ID input');
      await expect(page.getByTestId('creator-job-id')).toHaveValue('');
      await expect(page.getByTestId('creator-job-load')).toBeDisabled();

      for (const viewport of viewports) {
        await page.setViewportSize(viewport);
        const label = `Creator ${locale} ${viewport.width}x${viewport.height}`;
        // Let layout and focus restoration settle without relying on screenshot timing.
        await expect(page.getByTestId('creator-screen')).toBeVisible();
        await assertStages(page, locale);
        await assertNoHorizontalOverflow(page, label);
        await assertHintClearance(page, label);
        await screenshot(page, `creator-${locale}-${viewport.width}x${viewport.height}-main.png`);

        const opener = page.getByTestId('creator-device-open');
        const devices = page.getByTestId('creator-device-dialog');
        const assignmentOpener = page.getByTestId('creator-assignment-open');
        const assignment = page.getByTestId('creator-assignment-dialog');
        await opener.click();
        await expectVisible(devices, `${label} devices dialog`);
        await assertUnavailable(page.getByTestId('creator-devices-state'), locale, 'Creator devices');
        await assertFocusTrap(page, devices, `${label} devices`);
        await assertDialogBounds(page, devices, page.getByTestId('creator-device-close'), `${label} devices`);
        await assertNoHorizontalOverflow(page, `${label} devices`);
        await screenshot(page, `creator-${locale}-${viewport.width}x${viewport.height}-devices.png`);

        await assignmentOpener.click();
        await expectVisible(assignment, `${label} nested assignment dialog`);
        await assertFocusTrap(page, assignment, `${label} assignment`);
        await assertDialogBounds(page, assignment, page.getByTestId('creator-assignment-close'), `${label} assignment`);
        await assertNoHorizontalOverflow(page, `${label} assignment`);
        await assertHintClearance(page, `${label} assignment`);
        await screenshot(page, `creator-${locale}-${viewport.width}x${viewport.height}-assignment.png`);

        await page.keyboard.press('Escape');
        await expect(assignment).toBeHidden();
        await expect(devices).toBeVisible();
        await expect(assignmentOpener).toBeFocused();
        await assertCreatorShell(page);
        await page.keyboard.press('Escape');
        await expect(devices).toBeHidden();
        await expect(opener).toBeFocused();
        await assertCreatorShell(page);

        // Explicit close controls must obey the same stack/restoration contract.
        await opener.click();
        await assignmentOpener.click();
        await page.getByTestId('creator-assignment-close').click();
        await expect(assignment).toBeHidden();
        await expect(devices).toBeVisible();
        await expect(assignmentOpener).toBeFocused();
        await page.getByTestId('creator-device-close').click();
        await expect(devices).toBeHidden();
        await expect(opener).toBeFocused();
        await assertCreatorShell(page);
        console.log(`PASS ${label}: default unavailable, nested focus/escape/restoration and layout`);
      }

      // Existing demo title; no HTML replacement, store mutation or API interception.
      await openLibrary(page, clickNav, 'Star Orbit Prototype');
      await page.locator('[data-focus-zone="library"]').first().click();
      await assertContextEntry(page, 'game-details-modal', locale, expectVisible);
    }

    // Exercise the second details implementation through the existing source UI.
    await setLocale(page, 'en');
    await openSettings(page);
    await page.getByTestId('settings-tab-sources').click();
    const source = demoSourceCard(page);
    if (await source.count() === 0) {
      await page.getByTestId('add-source-input').fill(demoSourceUrl);
      await page.getByTestId('add-source-check').click();
      await expect(page.getByTestId('add-source-preview')).toContainText('Fusion Source v1.0');
      await page.getByTestId('add-source-add').click();
      addedDemoSource = true;
      await expect(source).toBeVisible();
    }
    await closeSettings(page);
    await openLibrary(page, clickNav, 'Orbit Runtime');
    await page.getByTestId('fusion-catalog-runtime-demo').click();
    await assertContextEntry(page, 'fusion-game-modal', 'en', expectVisible);
    await assertNoHorizontalOverflow(page, 'Creator context from Fusion details');
  } catch (error) {
    failure = error;
  }

  // Also restore on assertion failure; never conceal the original test failure.
  try {
    await closeKnownDialogs(page);
    await setLocale(page, 'en');
    if (addedDemoSource) {
      await openSettings(page);
      await page.getByTestId('settings-tab-sources').click();
      const source = demoSourceCard(page);
      page.once('dialog', (dialog) => dialog.accept());
      await source.getByTestId('fusion-source-remove').click();
      await expect(source).toHaveCount(0);
      await closeSettings(page);
    }
    await clickNav(page, 'home');
    await expectVisible(page.getByTestId('home-screen'), 'Home restored after Creator visual helper');
    if (originalViewport) await page.setViewportSize(originalViewport);
    await expect(page.getByTestId('settings-modal')).toBeHidden();
    await expect(page.getByTestId('creator-device-dialog')).toBeHidden();
    await expect(page.getByTestId('creator-assignment-dialog')).toBeHidden();
  } catch (cleanupError) {
    if (failure) throw new AggregateError([failure, cleanupError], 'Creator visual assertion and state restoration failed');
    throw cleanupError;
  }
  if (failure) throw failure;
  console.log('PASS Creator visual matrix, legacy/Fusion context entry; restored Home/en');
}

async function openSettings(page) {
  if (!await page.getByTestId('settings-modal').isVisible()) await page.getByTestId('top-settings').click();
  await expect(page.getByTestId('settings-modal')).toBeVisible();
}

async function closeSettings(page) {
  await page.getByTestId('settings-modal').getByTitle(settingsClose).click();
  await expect(page.getByTestId('settings-modal')).toBeHidden();
}

async function setLocale(page, locale) {
  await openSettings(page);
  await page.getByTestId('settings-tab-general').click();
  const language = page.getByTestId('settings-language');
  await expect(language).toBeVisible();
  const changed = await language.inputValue() !== locale;
  await language.selectOption(locale);
  // Save is disabled when clean. Do not bypass it by writing localStorage/store.
  const save = page.getByTestId('settings-modal').getByRole('button', { name: saveLabel });
  if (changed || await save.isEnabled()) await save.click();
  await expect(page.getByTitle(locale === 'ru' ? 'Закрыть настройки' : 'Close settings')).toBeVisible();
  await closeSettings(page);
  // Reopen once to prove persisted configuration, not only an unsaved draft.
  await openSettings(page);
  await page.getByTestId('settings-tab-general').click();
  await expect(page.getByTestId('settings-language')).toHaveValue(locale);
  await closeSettings(page);
}

async function assertStages(page, locale) {
  const placeholder = page.getByTestId('creator-stage-unavailable');
  for (const [index, stage] of stages.entries()) {
    const step = page.getByTestId(`creator-step-${stage}`);
    await expect(step).toBeVisible();
    await expect(step).toContainText(labels[locale][index]);
    await expect(step).toBeEnabled();
    await step.click();
    if (stage === 'base') {
      await expect(placeholder).toBeHidden();
    } else {
      await assertUnavailable(placeholder, locale, `Creator ${labels[locale][index]} stage`);
    }
    await assertCreatorShell(page);
  }
  await page.getByTestId('creator-step-base').click();
  await expect(placeholder).toBeHidden();
}

async function assertUnavailable(locator, locale, label) {
  await expect(locator, `${label} must report actual bridge unavailability`).toHaveAttribute('data-state', 'unavailable');
  await expect(locator).toBeVisible();
  await expect(locator).toContainText(unavailable[locale]);
  assert.doesNotMatch(await locator.innerText(), emptySuccess, `${label} must not misreport unavailable as an empty successful result`);
}

async function assertCreatorShell(page) {
  await expect(page.getByTestId('creator-screen')).toBeVisible();
  await expect(page.getByTestId('home-screen')).toBeHidden();
  await expect(page.getByTestId('library-screen')).toBeHidden();
}

async function assertFocusTrap(page, dialog, label) {
  await expect.poll(() => dialog.evaluate((node) => node.contains(document.activeElement)), { message: `${label} takes focus` }).toBe(true);
  const candidates = dialog.locator('button, input, select, textarea, a[href], summary, [tabindex]');
  const focusable = [];
  for (const candidate of await candidates.all()) {
    if (await candidate.isVisible() && await candidate.isEnabled() && await candidate.evaluate((node) => node.tabIndex >= 0)) focusable.push(candidate);
  }
  assert.ok(focusable.length > 0, `${label} has a keyboard-accessible action`);
  await focusable.at(-1).focus();
  await page.keyboard.press('Tab');
  await expect(focusable[0], `${label} wraps forward Tab`).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(focusable.at(-1), `${label} wraps backward Tab`).toBeFocused();
  for (const key of ['Tab', 'Tab', 'Shift+Tab', 'ArrowDown', 'ArrowRight', 'ArrowUp', 'ArrowLeft']) {
    await page.keyboard.press(key);
    await expect.poll(() => dialog.evaluate((node) => node.contains(document.activeElement)), { message: `${label}: ${key} cannot escape top dialog` }).toBe(true);
    await expect(dialog).toBeVisible();
    await assertCreatorShell(page);
  }
}

async function assertHintClearance(page, label) {
  const screen = await page.getByTestId('creator-screen').boundingBox();
  const hints = await page.locator('.rh-hint-bar').boundingBox();
  assert.ok(screen && hints, `${label}: Creator and hint bar have bounds`);
  assert.ok(screen.y + screen.height <= hints.y + 1, `${label} overlaps controller hint bar`);
  const overflow = await page.getByTestId('creator-screen').evaluate((node) => ({ client: node.clientWidth, scroll: node.scrollWidth }));
  assert.ok(overflow.scroll <= overflow.client + 1, `${label} has nested horizontal overflow`);
}

async function assertDialogBounds(page, dialog, close, label) {
  const viewport = page.viewportSize();
  const box = await dialog.boundingBox();
  assert.ok(viewport && box && box.width > 0 && box.height > 0, `${label} has bounds`);
  assert.ok(box.x >= -1 && box.y >= -1 && box.x + box.width <= viewport.width + 1 && box.y + box.height <= viewport.height + 1, `${label} fits desktop viewport`);
  await expect(close).toBeInViewport({ ratio: 1 });
  const overflow = await dialog.evaluate((node) => ({ client: node.clientWidth, scroll: node.scrollWidth }));
  assert.ok(overflow.scroll <= overflow.client + 1, `${label} has nested horizontal overflow`);
}

async function openLibrary(page, clickNav, query) {
  await clickNav(page, 'library');
  await page.locator('[data-collection-id="all"]').click();
  await page.getByTestId('library-search').fill(query);
  await expect(page.getByTestId('library-grid').getByText(query, { exact: true })).toBeVisible();
}

async function assertContextEntry(page, modalId, locale, expectVisible) {
  const modal = page.getByTestId(modalId);
  await expectVisible(modal, `${modalId} before Creator context action`);
  const title = (await modal.locator('h2').first().innerText()).trim();
  assert.ok(title, `${modalId} supplies a real context title`);
  await modal.getByTestId('creator-context-action').click();
  await expect(modal).toBeHidden();
  await assertCreatorShell(page);
  await expect(page.getByTestId('creator-context-title')).toBeVisible();
  await expect(page.getByTestId('creator-context-title')).toContainText(title);
  await assertUnavailable(page.getByTestId('creator-projects-state'), locale, 'Context entry still has no fabricated project');
  console.log(`PASS ${modalId} context action enters Creator with title: ${title}`);
}

function demoSourceCard(page) {
  return page.getByTestId('fusion-source-card').filter({ has: page.getByTestId('fusion-source-location').filter({ hasText: demoSourceUrl }) });
}

async function closeKnownDialogs(page) {
  for (const testId of ['creator-assignment-close', 'creator-device-close', 'fusion-game-close']) {
    const close = page.getByTestId(testId);
    if (await close.isVisible()) await close.click();
  }
  const legacy = page.getByTestId('game-details-modal');
  if (await legacy.isVisible()) await legacy.getByTitle(/^(?:Close|Закрыть)$/).click();
  if (await page.getByTestId('settings-modal').isVisible()) await closeSettings(page);
}
