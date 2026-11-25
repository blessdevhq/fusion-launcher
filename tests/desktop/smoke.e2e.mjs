import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import path from 'node:path';

const byTestId = (id) => $(`[data-testid="${id}"]`);

describe('Fusion Launcher native desktop smoke', () => {
  it('opens the real first-run UI and navigates to the source step', async () => {
    // Explicitly select the only window; this prevents the service's optional
    // multi-window focus helper from polling an uninstalled WDIO helper plugin.
    await browser.switchToWindow(await browser.getWindowHandle());
    const welcome = await byTestId('onboarding-step-welcome');
    await welcome.waitForDisplayed({ timeout: 60000 });
    assert.equal(await (await byTestId('onboarding-screen')).isDisplayed(), true);
    assert.equal(await (await byTestId('onboarding-language')).isDisplayed(), true);
    await (await byTestId('onboarding-next')).click();
    await (await byTestId('onboarding-step-source')).waitForDisplayed();
    assert.equal(await (await byTestId('onboarding-next-source')).isEnabled(), false);
    const screenshotDir = process.env.FUSION_E2E_ARTIFACTS;
    if (screenshotDir) {
      await mkdir(screenshotDir, { recursive: true });
      await browser.saveScreenshot(path.join(screenshotDir, 'onboarding-source.png'));
    }
  });
});
