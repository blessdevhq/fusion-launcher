import assert from 'node:assert/strict';

/** Exercises only the localhost browser preview; no payloads or live reports are sent. */
export async function runFusionVisual(page, { screenshot, assertNoHorizontalOverflow, expectVisible, clickNav }) {
  await page.getByTestId('settings-tab-sources').click();
  await page.getByTestId('add-source-input').fill('https://demo.fusion.invalid/index.json');
  await page.getByTestId('add-source-check').click();
  await expectVisible(page.getByTestId('add-source-preview').getByText('Fusion Source v1.0', { exact: false }), 'Fusion source index preview');
  await page.getByTestId('add-source-add').click();
  await expectVisible(page.getByTestId('fusion-source-card'), 'connected Fusion source');
  await screenshot(page, '11-fusion-source.png');
  await assertNoHorizontalOverflow(page, 'Fusion source');
  await page.getByTitle('Close settings').click();
  await clickNav(page, 'library');
  await page.locator('[data-focus-id="filter:all"]').click();
  await page.getByTestId('library-search').fill('');
  await expectVisible(page.getByTestId('fusion-catalog'), 'mixed Fusion catalog');
  await screenshot(page, '12-fusion-mixed-library.png');
  await assertNoHorizontalOverflow(page, 'mixed Fusion library');

  await page.getByTestId('library-search').fill('Orbit Runtime');
  await page.getByTestId('fusion-catalog-runtime-demo').click();
  await expectVisible(page.getByTestId('fusion-game-modal'), 'Fusion lazy game details');
  await page.getByTestId('fusion-game-readiness').getByText('Ready to launch').waitFor();
  await page.getByTestId('fusion-runtime-channel').selectOption('dynamic');
  await page.getByTestId('fusion-addon-coop').check();
  await page.getByTestId('fusion-game-modal').getByRole('button', { name: 'Apply options', exact: true }).click();
  await page.getByTestId('fusion-game-message').getByText(/Options saved/).waitFor();
  const messageBounds = await page.getByTestId('fusion-game-message').boundingBox();
  const readinessBounds = await page.getByTestId('fusion-game-readiness').boundingBox();
  assert.ok(messageBounds && readinessBounds && messageBounds.y + messageBounds.height <= readinessBounds.y, 'Fusion notice does not cover readiness');
  await screenshot(page, '13-fusion-ready-options.png');
  const original = page.viewportSize();
  for (const width of [1280, 1920]) {
    await page.setViewportSize({ width, height: width === 1280 ? 720 : 1080 });
    await assertNoHorizontalOverflow(page, `Fusion game ${width}`);
    await assertModalBounds(page);
    await screenshot(page, `13-fusion-ready-${width}.png`);
  }
  await page.setViewportSize(original);
  await page.getByTestId('fusion-crash-preview').click();
  await expectVisible(page.getByTestId('fusion-crash-report'), 'cleaned crash report');
  assert.match(await page.getByTestId('fusion-crash-report').innerText(), /<WORKSPACE_ROOT>/);
  assert.equal(await page.getByTestId('fusion-crash-send').isDisabled(), true, 'report sending requires a configured endpoint');
  await page.getByTestId('fusion-crash-report').scrollIntoViewIfNeeded();
  await screenshot(page, '14-fusion-cleaned-report.png');
  await page.getByTestId('fusion-crash-discard').click();
  await page.getByTestId('fusion-crash-report').waitFor({ state: 'hidden' });
  await page.getByTestId('fusion-game-close').click();

  await page.getByTestId('library-search').fill('Overlay Capability');
  await page.getByTestId('fusion-catalog-overlay-demo').click();
  await expectVisible(page.getByTestId('fusion-blockers'), 'per-game capability blockers');
  assert.equal(await page.getByTestId('fusion-game-install').isDisabled(), true);
  await screenshot(page, '15-fusion-blocked.png');
  await assertNoHorizontalOverflow(page, 'Fusion blocked game');
  await page.keyboard.press('Escape');
  await page.getByTestId('fusion-game-modal').waitFor({ state: 'hidden' });

  await page.getByTestId('library-search').fill('Cooperative Workspace');
  await page.getByTestId('fusion-catalog-coop-demo').click();
  await expectVisible(page.getByTestId('fusion-user-layer-base'), 'user-provided Fusion layer');
  await page.getByTestId('fusion-user-layer-base').getByRole('button', { name: 'Choose file', exact: true }).click();
  await page.getByPlaceholder('game/game.bin').fill('game/owned.bin');
  await screenshot(page, '16-fusion-user-files.png');
  await page.getByTestId('fusion-game-install').click();
  await expectVisible(page.getByTestId('fusion-game-cancel'), 'independent Fusion cancellation');
  await screenshot(page, '17-fusion-operation.png');
  await page.getByTestId('fusion-game-cancel').click();
  await page.getByTestId('fusion-operation-progress').getByText(/cancelled/).waitFor();
  await page.getByTestId('fusion-game-install').click();
  await page.getByTestId('fusion-operation-progress').getByText(/completed/).waitFor();
  await page.getByTestId('fusion-trust-executable').click();
  await page.getByTestId('fusion-game-readiness').getByText('Ready to launch').waitFor();
  await screenshot(page, '18-fusion-installed-trust.png');
  await page.getByTestId('fusion-game-rollback').click();
  await page.getByTestId('fusion-game-modal').getByText('Version: 1.0.0 → 1.1.0').waitFor();
  await screenshot(page, '19-fusion-rollback.png');
  await assertNoHorizontalOverflow(page, 'Fusion rollback');
  page.once('dialog', (dialog) => dialog.accept());
  await page.getByTestId('fusion-game-remove').click();
  await page.getByTestId('fusion-game-readiness').getByText('Available').waitFor();
  await page.getByTestId('fusion-game-close').click();
  await page.getByTestId('library-search').fill('Orbit Runtime');
  await page.getByTestId('fusion-catalog-runtime-demo').click();
  await page.getByTestId('fusion-game-readiness').getByText('Ready to launch').waitFor();
  page.once('dialog', (dialog) => dialog.accept());
  await page.getByTestId('fusion-game-remove').click();
  await page.getByTestId('fusion-game-readiness').getByText('Available').waitFor();
  await page.getByTestId('fusion-game-close').click();

  await page.getByTestId('top-settings').click();
  await page.getByTestId('settings-tab-sources').click();
  await page.getByTestId('fusion-source-refresh').click();
  await expectVisible(page.getByTestId('fusion-source-card'), 'Fusion source after refresh');
  page.once('dialog', (dialog) => dialog.accept());
  await page.getByTestId('fusion-source-remove').click();
  await page.getByTestId('fusion-source-card').waitFor({ state: 'hidden' });
  await page.getByTestId('settings-tab-emulators').click();
  console.log('PASS Fusion source, mixed catalog, lazy details, options, blockers, user files, operation cancellation, trust, rollback and cleaned-report states');
}

async function assertModalBounds(page) {
  const bounds = await page.getByTestId('fusion-game-modal').boundingBox();
  const viewport = page.viewportSize();
  assert.ok(bounds && bounds.x >= 0 && bounds.y >= 0 && bounds.x + bounds.width <= viewport.width && bounds.y + bounds.height <= viewport.height, 'Fusion modal fits the supported desktop viewport');
  const lastAction = await page.getByTestId('fusion-game-remove').boundingBox();
  assert.ok(lastAction && lastAction.y + lastAction.height <= viewport.height, 'Fusion critical actions remain reachable');
}
