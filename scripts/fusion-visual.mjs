import assert from 'node:assert/strict';
import { expect } from '@playwright/test';

/** Exercises only localhost fixtures; no payloads or live reports are sent. */
export async function runFusionVisual(page, { screenshot, assertNoHorizontalOverflow, expectVisible, clickNav }) {
  await page.getByTestId('settings-tab-sources').click();
  await page.getByTestId('add-source-input').fill('https://demo.fusion.invalid/index.json');
  await page.getByTestId('add-source-check').click();
  await expectVisible(page.getByTestId('add-source-preview').getByText('Fusion Source v1.0', { exact: false }), 'Fusion source index preview');
  await page.getByTestId('add-source-add').click();
  await expectVisible(page.getByTestId('fusion-source-card'), 'connected Fusion source');
  await screenshot(page, '11-fusion-source.png');
  await page.getByTitle('Close settings').click();
  await clickNav(page, 'library');
  await page.locator('[data-collection-id="all"]').click();
  await page.getByTestId('library-search').fill('');
  await expectVisible(page.getByTestId('fusion-catalog-runtime-demo'), 'Fusion poster in the shared grid');
  assert.equal(await page.getByTestId('fusion-catalog').count(), 0, 'separate Fusion banner is removed');
  assert.equal(await page.getByTestId('library-filters').count(), 0, 'duplicate status filter row is removed');
  await assertCollectionCount(page, 'all');
  await page.locator('[data-collection-id="ready"]').click();
  await assertCollectionCount(page, 'ready');
  await expectVisible(page.getByTestId('fusion-catalog-runtime-demo'), 'ready collection includes a trusted Fusion installation');
  await page.locator('[data-collection-id="platform:pc"]').click();
  await assertCollectionCount(page, 'platform:pc');
  await expectVisible(page.getByTestId('fusion-catalog-coop-demo'), 'PC collection includes uninstalled Fusion titles');
  await page.locator('[data-collection-id="all"]').click();
  await screenshot(page, '12-fusion-mixed-library.png');
  await assertNoHorizontalOverflow(page, 'mixed Fusion library');

  await openGame(page, 'Orbit Runtime', 'runtime-demo');
  await page.getByTestId('fusion-game-readiness').getByText('Ready to launch').waitFor();
  await expect(page.getByTestId('fusion-options-apply')).toBeDisabled();
  await expect(page.getByTestId('fusion-integrity-details')).not.toHaveAttribute('open');
  await expect(page.getByTestId('fusion-game-version')).toContainText('1.0.0 → 1.1.0');
  await expectVisible(page.getByTestId('fusion-update-badge'), 'new source version badge');
  await page.getByTestId('fusion-runtime-channel').selectOption('dynamic');
  await page.getByTestId('fusion-addon-coop').check();
  await expect(page.getByTestId('fusion-options-apply')).toBeEnabled();
  await page.getByTestId('fusion-options-apply').click();
  await waitForOperation(page);
  await expect(page.getByTestId('fusion-options-apply')).toBeDisabled();
  await expect(page.getByTestId('fusion-options-apply')).toHaveText('Settings applied');
  await expect(page.getByTestId('fusion-game-version')).toHaveText('Version 1.1.0');
  await expect(page.getByTestId('fusion-update-badge')).toHaveCount(0);
  await expect(page.getByTestId('fusion-game-install')).toHaveText('Rebuild');
  await expect(page.getByTestId('fusion-operation-progress')).toHaveCount(0);
  await screenshot(page, '13-fusion-ready-options.png');
  const original = page.viewportSize();
  for (const viewport of [{ width: 1280, height: 720 }, { width: 1365, height: 768 }, { width: 1440, height: 900 }, { width: 1920, height: 1080 }]) {
    await page.setViewportSize(viewport);
    await assertNoHorizontalOverflow(page, `Fusion modal ${viewport.width}`);
    await assertModalBounds(page);
    await screenshot(page, `13-fusion-ready-${viewport.width}.png`);
  }
  await page.setViewportSize(original);
  await page.getByTestId('fusion-integrity-details').locator('summary').click();
  await page.getByTestId('fusion-integrity-details').locator('summary').focus();
  await page.keyboard.press('Tab');
  assert.equal(await page.getByTestId('fusion-game-modal').evaluate((node) => node.contains(document.activeElement)), true, 'summary participates in modal keyboard focus');
  await page.getByTestId('fusion-crash-preview').first().click();
  await expectVisible(page.getByTestId('fusion-crash-report'), 'cleaned crash report');
  assert.match(await page.getByTestId('fusion-crash-report').innerText(), /<WORKSPACE_ROOT>/);
  assert.equal(await page.getByTestId('fusion-crash-send').isDisabled(), true, 'report sending requires a configured endpoint');
  await page.getByTestId('fusion-crash-report').scrollIntoViewIfNeeded();
  await screenshot(page, '14-fusion-cleaned-report.png');
  await page.getByTestId('fusion-crash-row').first().getByTestId('fusion-crash-discard').click();
  await expect(page.getByTestId('fusion-crash-row')).toHaveCount(0);
  await expect(page.getByTestId('fusion-crash-report')).toHaveCount(0);
  await page.getByTestId('fusion-game-close').click();

  await openGame(page, 'Overlay Capability', 'overlay-demo');
  await expectVisible(page.getByTestId('fusion-blockers'), 'per-game capability blockers');
  assert.equal(await page.getByTestId('fusion-game-install').isDisabled(), true);
  await screenshot(page, '15-fusion-blocked.png');
  await assertNoHorizontalOverflow(page, 'Fusion blocked game');
  await page.keyboard.press('Escape');
  await page.getByTestId('fusion-game-modal').waitFor({ state: 'hidden' });

  await openGame(page, 'Cooperative Workspace', 'coop-demo');
  await expectVisible(page.getByTestId('fusion-user-layer-base'), 'user-provided Fusion layer');
  await expect(page.getByTestId('fusion-options-apply')).toHaveCount(0);
  await page.getByTestId('fusion-user-layer-base').getByRole('button', { name: 'Choose file', exact: true }).click();
  await page.getByPlaceholder('game/game.bin').fill('game/owned.bin');
  await screenshot(page, '16-fusion-user-files.png');
  await page.getByTestId('fusion-game-install').click();
  await expectVisible(page.getByTestId('fusion-game-cancel'), 'independent Fusion cancellation');
  await screenshot(page, '17-fusion-operation.png');
  await page.getByTestId('fusion-game-cancel').click();
  await expect(page.getByTestId('fusion-game-cancel')).toHaveCount(0);
  await expect(page.getByTestId('fusion-operation-progress')).toHaveCount(0);
  await expect(page.getByTestId('fusion-game-install')).toBeEnabled();
  await page.getByTestId('fusion-game-install').click();
  await waitForOperation(page);
  await expect(page.getByTestId('fusion-integrity-details')).toHaveAttribute('open', '');
  await page.getByTestId('fusion-trust-executable').click();
  await page.getByTestId('fusion-game-readiness').getByText('Ready to launch').waitFor();
  await expect(page.getByTestId('fusion-integrity-details')).not.toHaveAttribute('open');
  await screenshot(page, '18-fusion-installed-trust.png');
  await page.getByTestId('fusion-game-rollback').click();
  await expect(page.getByTestId('fusion-game-version')).toHaveText('Version: 1.0.0 → 1.1.0');
  await screenshot(page, '19-fusion-rollback.png');
  await assertNoHorizontalOverflow(page, 'Fusion rollback');
  await removeInstalledGame(page);
  await page.getByTestId('fusion-game-close').click();

  await openGame(page, 'Integrity Recovery', 'repair-demo');
  await expect(page.getByTestId('fusion-integrity-details')).toHaveAttribute('open', '');
  await expectVisible(page.getByRole('button', { name: 'Restore files', exact: true }), 'integrity repair action');
  await page.getByTestId('fusion-addon-coop').check();
  await page.getByTestId('fusion-options-apply').click();
  await expect(page.getByTestId('fusion-operation-progress')).toContainText('Operation failed');
  await expect(page.getByTestId('fusion-options-apply')).toBeEnabled();
  await expect(page.getByTestId('fusion-addon-coop')).toBeChecked();
  await screenshot(page, '20-fusion-repair-failed.png');
  await page.getByTestId('fusion-options-apply').click();
  await waitForOperation(page);
  await page.getByTestId('fusion-game-readiness').getByText('Ready to launch').waitFor();
  await expect(page.getByTestId('fusion-options-apply')).toBeDisabled();
  await expect(page.getByTestId('fusion-integrity-details')).not.toHaveAttribute('open');
  await screenshot(page, '21-fusion-repaired.png');
  await removeInstalledGame(page);
  await page.getByTestId('fusion-game-close').click();

  await openGame(page, 'Orbit Runtime', 'runtime-demo');
  await removeInstalledGame(page);
  await page.getByTestId('fusion-game-close').click();
  await page.getByTestId('top-settings').click();
  await page.getByTestId('settings-tab-sources').click();
  await page.getByTestId('fusion-source-refresh').click();
  await expectVisible(page.getByTestId('fusion-source-card'), 'Fusion source after refresh');
  page.once('dialog', (dialog) => dialog.accept());
  await page.getByTestId('fusion-source-remove').click();
  await page.getByTestId('fusion-source-card').waitFor({ state: 'hidden' });
  await page.getByTestId('add-source-local').click();
  await expect(page.getByTestId('add-source-preview')).toContainText('preview://Sources/local-release');
  await page.getByTestId('add-source-prepare-eden').check();
  await screenshot(page, '22-fusion-local-preview.png');
  await page.getByTestId('add-source-add').click();
  await expect(page.getByTestId('fusion-source-location')).toHaveText('preview://Sources/local-release');
  await expect(page.getByTestId('fusion-source-card')).toContainText('Prepare portable Eden profiles');
  await screenshot(page, '22-fusion-local-source.png');
  await assertNoHorizontalOverflow(page, 'local source settings');
  const localViewport = page.viewportSize();
  for (const viewport of [{ width: 1280, height: 720 }, { width: 1920, height: 1080 }]) {
    await page.setViewportSize(viewport);
    await assertNoHorizontalOverflow(page, `local source settings ${viewport.width}`);
    await page.getByTestId('fusion-source-replace-folder').scrollIntoViewIfNeeded();
    await expect(page.getByTestId('fusion-source-replace-folder')).toBeInViewport();
    await screenshot(page, `22-fusion-local-source-${viewport.width}.png`);
  }
  await page.setViewportSize(localViewport);
  await page.getByTitle('Close settings').click();
  await clickNav(page, 'library');
  await page.locator('[data-collection-id="all"]').click();
  await openGame(page, 'Orbit Runtime', 'runtime-demo');
  await page.getByTestId('fusion-game-install').click();
  await waitForOperation(page);
  while (await page.getByTestId('fusion-trust-executable').count()) await page.getByTestId('fusion-trust-executable').first().click();
  await page.getByTestId('fusion-game-launch').click();
  await waitForOperation(page);
  await page.getByTestId('fusion-game-close').click();
  await page.getByTestId('top-settings').click();
  await page.getByTestId('settings-tab-sources').click();
  await page.getByTestId('fusion-source-replace-folder').click();
  await expect(page.getByTestId('fusion-source-location')).toHaveText('preview://Sources/local-release-v2');
  await screenshot(page, '23-fusion-local-next-release.png');
  await page.getByTitle('Close settings').click();
  await clickNav(page, 'library');
  await openGame(page, 'Orbit Runtime', 'runtime-demo');
  await expect(page.getByTestId('fusion-update-badge')).toBeVisible();
  await page.getByTestId('fusion-game-install').click();
  await waitForOperation(page);
  await expect(page.getByTestId('fusion-game-version')).toContainText('1.2.0');
  await page.getByTestId('fusion-game-rollback').click();
  await expect(page.getByTestId('fusion-game-version')).toContainText('1.0.0 → 1.2.0');
  await screenshot(page, '24-fusion-local-rollback.png');
  await removeInstalledGame(page);
  await page.getByTestId('fusion-game-close').click();
  await page.getByTestId('top-settings').click();
  await page.getByTestId('settings-tab-sources').click();
  page.once('dialog', (dialog) => dialog.accept());
  await page.getByTestId('fusion-source-remove').click();
  await page.getByTestId('fusion-source-card').waitFor({ state: 'hidden' });
  await page.getByTestId('settings-tab-emulators').click();
  console.log('PASS unified Fusion grid/counts, actual settings rebuild, version labels, collapsed trust, cancellation, repair failure/retry and crash deletion');
}

async function openGame(page, query, gameId) {
  await page.getByTestId('library-search').fill(query);
  await page.getByTestId(`fusion-catalog-${gameId}`).click();
  await page.getByTestId('fusion-game-modal').waitFor({ state: 'visible' });
}

async function waitForOperation(page) {
  await expect(page.getByTestId('fusion-game-cancel')).toBeVisible();
  await expect(page.getByTestId('fusion-game-cancel')).toHaveCount(0);
  await expect(page.getByTestId('fusion-game-install')).toBeEnabled();
  await expect(page.getByTestId('fusion-operation-progress')).toHaveCount(0);
}

async function removeInstalledGame(page) {
  page.once('dialog', (dialog) => dialog.accept());
  await page.getByTestId('fusion-game-remove').click();
  await page.getByTestId('fusion-game-readiness').getByText('Available').waitFor();
}

async function assertCollectionCount(page, id) {
  const chip = page.locator(`[data-collection-id="${id}"]`);
  const count = Number((await chip.locator('.rh-collection-count').innerText()).match(/\d+/)?.[0]);
  await expect(page.locator('.rh-library-grid > .rh-game-card')).toHaveCount(count);
}

async function assertModalBounds(page) {
  const bounds = await page.getByTestId('fusion-game-modal').boundingBox();
  const viewport = page.viewportSize();
  assert.ok(bounds && bounds.x >= 0 && bounds.y >= 0 && bounds.x + bounds.width <= viewport.width && bounds.y + bounds.height <= viewport.height, 'Fusion modal fits the supported desktop viewport');
  const lastAction = await page.getByTestId('fusion-game-remove').boundingBox();
  assert.ok(lastAction && lastAction.y + lastAction.height <= viewport.height, 'Fusion critical actions remain reachable');
}
