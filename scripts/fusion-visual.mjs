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
  const history = page.getByTestId('fusion-save-history');
  const historySummary = history.locator('summary');
  await expect(historySummary).toBeVisible();
  await expect(history).not.toHaveAttribute('open');
  await expect(history.getByTestId('fusion-save-row')).toHaveCount(0);
  await historySummary.focus();
  await page.keyboard.press('Enter');
  await expect(history.getByTestId('fusion-save-row')).toHaveCount(2);
  await expect(history.getByTestId('fusion-save-row').first().locator('time')).toHaveAttribute('datetime', '2026-10-18T10:00:00Z');
  await expect(history.getByTestId('fusion-save-row').first()).toContainText('3 files');
  await expect(history.getByTestId('fusion-save-row').first()).toContainText('4 KB');
  const createBackup = history.getByRole('button', { name: 'Create local backup', exact: true });
  await expect(createBackup).toBeEnabled();
  await expect(history.getByRole('button', { name: /restore/i })).toHaveCount(0);
  await historySummary.scrollIntoViewIfNeeded();
  await screenshot(page, '13-fusion-save-history.png');
  await page.keyboard.press('Enter');
  await expect(history).not.toHaveAttribute('open');
  await expect(page.getByTestId('fusion-options-apply')).toBeDisabled();
  await expect(page.getByTestId('fusion-integrity-details')).not.toHaveAttribute('open');
  await expect(page.getByTestId('fusion-game-version')).toContainText('1.0.0 → 1.1.0');
  await expectVisible(page.getByTestId('fusion-update-badge'), 'new source version badge');
  await page.getByTestId('fusion-game-options').locator('summary').first().click();
  await page.getByTestId('fusion-runtime-channel').selectOption('dynamic');
  await page.getByTestId('fusion-addon-coop').check();
  await expect(page.getByTestId('fusion-options-apply')).toBeEnabled();
  await historySummary.click();
  await expect(history.getByTestId('fusion-save-row')).toHaveCount(2);
  await page.getByTestId('fusion-options-apply').click();
  await waitForOperation(page);
  await expect(history.getByTestId('fusion-save-row')).toHaveCount(3);
  await expect(history.getByTestId('fusion-save-row').first()).toContainText('4 files');
  await historySummary.click();
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
    if (viewport.width === 1280) {
      await historySummary.click();
      await expect(history.getByTestId('fusion-save-row')).toHaveCount(3);
      await history.getByTestId('fusion-save-row').last().scrollIntoViewIfNeeded();
      await expect(history.getByTestId('fusion-save-row').last()).toBeInViewport();
      await screenshot(page, '13-fusion-save-history-1280.png');
      await historySummary.click();
    }
    await assertNoHorizontalOverflow(page, `Fusion modal ${viewport.width}`);
    await assertModalBounds(page);
    await screenshot(page, `13-fusion-ready-${viewport.width}.png`);
  }
  await page.setViewportSize(original);
  await historySummary.click();
  await expect(history.getByTestId('fusion-save-row')).toHaveCount(3);
  // Simulate a guarded native rejection, then retry through the browser fixture.
  await rejectManualBackup(page, { code: 'operation_in_progress', details: {} });
  await createBackup.click();
  await expect(history.getByRole('alert')).toHaveText('Wait for the current game operation to finish, then try again.');
  await rejectManualBackup(page, { code: 'persistence_conflict', details: { reason: 'no_save_files' } });
  await createBackup.click();
  await expect(history.getByRole('alert')).toHaveText('No managed save files were found for this installation.');
  await expect(history.getByTestId('fusion-save-row')).toHaveCount(3);
  await createBackup.focus();
  await page.keyboard.press('Enter');
  await expect(createBackup).toBeDisabled();
  await expect(page.getByTestId('fusion-game-launch')).toBeDisabled();
  await expect(history.getByText('Creating local backup…', { exact: true })).toBeVisible();
  await expect(history.getByTestId('fusion-save-row')).toHaveCount(3);
  await expect(history.getByText('Local backup created.', { exact: true })).toBeVisible();
  await expect(history.getByTestId('fusion-save-row')).toHaveCount(4);
  await expect(createBackup).toBeFocused();
  await expect(createBackup).toBeEnabled();
  await expect(page.getByTestId('fusion-game-version')).toHaveText('Version 1.1.0');
  await historySummary.scrollIntoViewIfNeeded();
  await screenshot(page, '13-fusion-manual-backup.png');
  await historySummary.click();
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
  await expect(history).toHaveCount(0);
  assert.equal(await page.getByTestId('fusion-game-install').isDisabled(), true);
  await screenshot(page, '15-fusion-blocked.png');
  await assertNoHorizontalOverflow(page, 'Fusion blocked game');
  await page.keyboard.press('Escape');
  await page.getByTestId('fusion-game-modal').waitFor({ state: 'hidden' });

  // A failure can arrive while this modal is closed. Inject only that IPC DTO;
  // subsequent commands return to the ordinary in-memory preview source.
  await mockFailedSetupState(page);
  await openGame(page, 'Cooperative Workspace', 'coop-demo');
  const primary = page.getByTestId('fusion-game-modal').locator('[data-fusion-primary]');
  await expect(primary).toHaveText('Choose game files', { timeout: 3000 });
  const setupHeading = page.getByRole('heading', { name: 'Before you play' });
  await expect(setupHeading).toBeInViewport();
  await expect(page.getByTestId('fusion-game-options')).not.toHaveAttribute('open');
  await primary.press('Enter');
  await expect(page.getByTestId('fusion-user-layer-base').getByRole('button', { name: 'Choose file', exact: true })).toBeFocused();
  await expectVisible(page.getByTestId('fusion-input-error-base'), 'reopened setup hydrates the last failed operation');
  await expectVisible(page.getByTestId('fusion-user-layer-base'), 'user-provided Fusion layer');
  await expectVisible(page.getByTestId('fusion-setup-required'), 'required file group');
  await expect(page.getByTestId('fusion-input-status-base')).toHaveText('Not selected');
  await expect(page.getByTestId('fusion-setup-optional')).not.toHaveAttribute('open');
  await expect(page.getByTestId('fusion-options-apply')).toHaveCount(0);
  await expect(page.getByTestId('fusion-game-install')).toHaveCount(0);
  await page.getByTestId('fusion-input-error-base').scrollIntoViewIfNeeded();
  await screenshot(page, '16-fusion-required-error.png');
  await page.getByTestId('fusion-user-layer-base').getByRole('button', { name: 'Choose file', exact: true }).click();
  await expect(page.getByTestId('fusion-input-status-base')).toHaveText('Selected · not yet checked');
  await expect(primary).toHaveText('Install');
  await expect(page.getByTestId('fusion-input-error-base')).toHaveCount(0);
  await page.getByPlaceholder('game/game.bin').fill('game/owned.bin');
  const selectedPath = `F:/Owned/${'long-folder-name/'.repeat(12)}Йоши-é-game.bin`;
  await mockNextPicker(page, { result: selectedPath });
  await page.getByTestId('fusion-user-layer-base').getByRole('button', { name: 'Choose file', exact: true }).click();
  await expect(page.getByTestId('fusion-input-path-base')).toHaveText(selectedPath);
  await mockNextPicker(page, { result: null });
  await page.getByTestId('fusion-user-layer-base').getByRole('button', { name: 'Choose file', exact: true }).click();
  await expect(page.getByTestId('fusion-input-path-base')).toHaveText(selectedPath);
  await expect(page.getByTestId('fusion-input-destination-base')).toHaveValue('game/owned.bin');
  await mockNextPicker(page, { error: 'Picker unavailable (test fixture)' });
  await page.getByTestId('fusion-user-layer-base').getByRole('button', { name: 'Choose file', exact: true }).click();
  await expect(page.getByTestId('fusion-game-message')).toContainText('Picker unavailable');
  await expect(page.getByTestId('fusion-input-path-base')).toHaveText(selectedPath);
  await expect(page.getByTestId('fusion-game-install')).toBeEnabled();
  await page.getByTestId('fusion-input-path-base').scrollIntoViewIfNeeded();
  await screenshot(page, '16-fusion-long-path.png');
  await page.getByTestId('fusion-user-layer-base').getByRole('button', { name: 'Choose folder', exact: true }).click();
  await expect(page.getByTestId('fusion-input-destination-base')).toHaveCount(0);
  await page.getByTestId('fusion-game-options').locator('summary').first().click();
  await page.getByTestId('fusion-addon-coop').check();
  await page.getByTestId('fusion-game-install').click();
  await expect(page.getByTestId('fusion-setup-optional')).toHaveAttribute('open', '');
  await expectVisible(page.getByTestId('fusion-input-error-coop'), 'optional input error expands its group');
  await page.getByTestId('fusion-input-error-coop').scrollIntoViewIfNeeded();
  await screenshot(page, '16-fusion-optional-error.png');
  await expect(page.getByTestId('fusion-input-path-base')).toHaveText('preview://UserFiles/base/');
  await page.getByTestId('fusion-user-layer-coop').getByRole('button', { name: 'Choose file', exact: true }).click();
  await expect(page.getByTestId('fusion-input-error-coop')).toHaveCount(0);
  await page.getByTestId('fusion-addon-coop').uncheck();
  await expect(page.getByTestId('fusion-user-layer-coop').getByRole('button', { name: 'Choose file', exact: true })).toBeDisabled();
  await screenshot(page, '16-fusion-optional-inputs.png');
  for (const viewport of [{ width: 1280, height: 720 }, { width: 1365, height: 768 }, { width: 1440, height: 900 }, { width: 1920, height: 1080 }]) {
    await page.setViewportSize(viewport);
    await page.getByTestId('fusion-input-path-base').scrollIntoViewIfNeeded();
    await expect(page.getByTestId('fusion-input-path-base')).toBeInViewport();
    assert.equal(await page.getByTestId('fusion-game-modal').evaluate((node) => [...node.querySelectorAll('*')].every((child) => child.scrollWidth <= child.clientWidth + 1)), true, 'setup has no internally clipped horizontal content');
    await assertModalBounds(page);
    await assertNoHorizontalOverflow(page, `Fusion setup ${viewport.width}`);
    await screenshot(page, `16-fusion-setup-${viewport.width}.png`);
  }
  await page.setViewportSize(original);
  await screenshot(page, '16-fusion-user-files.png');
  await page.getByTestId('fusion-setup-optional').locator('summary').focus();
  await page.keyboard.press('Tab');
  await expect(page.getByTestId('fusion-game-install')).toBeFocused();
  await page.getByTestId('fusion-game-install').click();
  await expectVisible(page.getByTestId('fusion-game-cancel'), 'independent Fusion cancellation');
  await screenshot(page, '17-fusion-operation.png');
  await page.getByTestId('fusion-game-cancel').click();
  await expect(page.getByTestId('fusion-game-cancel')).toHaveCount(0);
  await expect(page.getByTestId('fusion-operation-progress')).toHaveCount(0);
  await expect(page.getByTestId('fusion-game-install')).toBeEnabled();
  await expect(page.getByTestId('fusion-input-path-base')).toHaveText('preview://UserFiles/base/');
  await page.getByTestId('fusion-game-install').click();
  await waitForOperation(page);
  await expect(page.getByTestId('fusion-integrity-details')).toHaveAttribute('open', '');
  await expect(primary).toHaveText('Review launch files');
  await expect(page.getByTestId('fusion-game-launch')).toHaveCount(0);
  await primary.press('Enter');
  await expect(page.getByTestId('fusion-trust-executable')).toBeFocused();
  await page.getByTestId('fusion-trust-executable').click();
  await page.getByTestId('fusion-game-readiness').getByText('Ready to launch').waitFor();
  await expect(primary).toHaveText('Play');
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
  await mockSaveHistoryRead(page, 'repair-demo');
  await historySummary.click();
  await expect(history.getByRole('status')).toHaveText('Reading backup history…');
  await finishSaveHistoryRead(page, null);
  await expect(history.getByRole('alert')).toHaveText('Could not read local backup history.');
  await mockSaveHistoryRead(page, 'repair-demo');
  await history.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(historySummary).toBeFocused();
  await finishSaveHistoryRead(page, { snapshots: [], skippedCount: 1, hasMore: true });
  await expect(history.getByRole('status')).toHaveText('No readable backup records found.');
  await expect(history).toContainText('Unreadable backup records skipped: 1.');
  await expect(history).toContainText('The list is limited.');
  await historySummary.click();
  await expect(history.getByRole('status')).toHaveCount(0);
  await historySummary.click();
  await expect(history.getByRole('status')).toHaveText('No local save backups yet.');
  await historySummary.click();
  await expectVisible(page.getByRole('button', { name: 'Restore files', exact: true }), 'integrity repair action');
  await expect(primary).toHaveText('Restore files');
  await page.getByTestId('fusion-game-options').locator('summary').first().click();
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

async function rejectManualBackup(page, error) {
  await page.evaluate((error) => {
    window.__TAURI_INTERNALS__ = { invoke: async (command, args) => {
      delete window.__TAURI_INTERNALS__;
      if (command !== 'fusion_game_create_save_snapshot' || args.game.sourceId !== 'fusion-preview' || args.game.gameId !== 'runtime-demo') {
        throw new Error(`Unexpected mocked backup command: ${command}`);
      }
      throw { ...error, message: 'Test fixture: D:/Private/save' };
    } };
  }, error);
}

async function mockSaveHistoryRead(page, gameId) {
  await page.evaluate((gameId) => {
    window.__TAURI_INTERNALS__ = { invoke: async (command, args) => {
      delete window.__TAURI_INTERNALS__;
      if (command !== 'fusion_game_list_save_snapshots' || args.game.sourceId !== 'fusion-preview' || args.game.gameId !== gameId) {
        throw new Error(`Unexpected mocked backup command: ${command}`);
      }
      return new Promise((resolve, reject) => {
        window.finishBackupRead = (inventory) => {
          delete window.finishBackupRead;
          if (inventory) resolve(inventory);
          else reject(new Error('Test fixture: backup metadata is unavailable'));
        };
      });
    } };
  }, gameId);
}

async function finishSaveHistoryRead(page, inventory) {
  await expect.poll(() => page.evaluate(() => typeof window.finishBackupRead)).toBe('function');
  await page.evaluate((result) => window.finishBackupRead(result), inventory);
}

async function mockFailedSetupState(page) {
  await page.evaluate(() => {
    window.__TAURI_INTERNALS__ = {
      invoke: async (command) => {
        delete window.__TAURI_INTERNALS__;
        if (command !== 'fusion_game_get_state') throw new Error(`Unexpected mocked command: ${command}`);
        return {
          sourceId: 'fusion-preview', gameId: 'coop-demo', title: 'Cooperative Workspace Demo', version: '1.1.0', platform: 'pc',
          installed: false, activeVersion: null, activeSnapshotId: null, ready: false, updateAvailable: false, rollbackAvailable: false,
          options: { enabledAddons: ['shader-cache'], runtimeChannel: 'pinned' }, appliedOptions: null, defaultRuntimeChannel: 'pinned', externalWorkspaceRoot: null,
          addons: [{ id: 'shader-cache', title: 'Shader cache', description: null, kind: 'shader_cache', enabled: true }, { id: 'coop', title: null, description: null, kind: 'addon', enabled: false }],
          runtimeChannels: ['pinned', 'dynamic'],
          userLayers: [{ layerId: 'base', title: 'Your game files', optional: false, instructions: 'Use your own files.', archive: 'raw', rawPathRequired: true }, { layerId: 'coop', title: 'Cooperative content', optional: true, instructions: null, archive: 'zip', rawPathRequired: false }],
          executables: [], blockers: [], warnings: [], crashReportIds: [], crashReports: [],
          operation: { operationId: 'fixture-reopened-failure', sourceId: 'fusion-preview', gameId: 'coop-demo', layerId: 'base', phase: 'failed', sequence: 1, status: 'failed', error: { code: 'integrity_violation', message: 'Imported file did not match the source (test fixture).', details: { layerId: 'base' } } },
        };
      },
    };
  });
}

/** Mock only a single native picker response; all game operations remain in preview. */
async function mockNextPicker(page, reply) {
  await page.evaluate((reply) => {
    window.__TAURI_INTERNALS__ = {
      invoke: async (command) => {
        delete window.__TAURI_INTERNALS__;
        if (command !== 'plugin:dialog|open') throw new Error(`Unexpected mocked command: ${command}`);
        if (reply.error) throw new Error(reply.error);
        return reply.result;
      },
    };
  }, reply);
}

async function waitForOperation(page) {
  await expect(page.getByTestId('fusion-game-cancel')).toBeVisible();
  await expect(page.getByTestId('fusion-game-cancel')).toHaveCount(0);
  await expect(page.getByTestId('fusion-game-modal').locator('[data-fusion-primary]')).toBeEnabled();
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
