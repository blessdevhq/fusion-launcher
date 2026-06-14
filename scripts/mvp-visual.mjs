import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, rm } from 'node:fs/promises';
import net from 'node:net';
import path from 'node:path';

import { chromium } from '@playwright/test';

const root = process.cwd();
const screenshotsDir = path.join(root, '.tmp', 'mvp-visual');
const externalUrl = process.env.FUSION_LAUNCHER_VISUAL_URL ?? process.env.RETROHYDRA_VISUAL_URL;
const timeoutMs = 60_000;

let server = null;
let browser = null;
const serverLogs = [];

async function main() {
  await rm(screenshotsDir, { recursive: true, force: true });
  await mkdir(screenshotsDir, { recursive: true });

  const baseUrl = externalUrl ?? await startDevServer();
  browser = await launchChromium();

  const page = await browser.newPage({ viewport: { width: 1365, height: 768 } });
  await page.goto(baseUrl);
  await page.waitForLoadState('networkidle', { timeout: timeoutMs });

  await expectVisible(page.getByTestId('onboarding-screen'), 'onboarding screen');
  await expectVisible(page.getByTestId('onboarding-stepper'), 'onboarding stepper');
  await expectVisible(page.getByTestId('onboarding-step-welcome'), 'onboarding welcome step');
  await screenshot(page, '01-onboarding.png');
  await assertNoHorizontalOverflow(page, 'onboarding');

  await page.getByTestId('onboarding-nav-source').click();
  await expectVisible(page.getByTestId('onboarding-demo-card'), 'onboarding demo source card');
  await expectVisible(page.getByTestId('onboarding-source-card'), 'onboarding community source card');
  const onboardingSource = page.getByTestId('onboarding-source-card');
  await onboardingSource.getByPlaceholder('https://example.com/repo.json').fill('https://community.example/fusion-launcher-repository.json');
  await onboardingSource.getByRole('button', { name: 'Check' }).click();
  await expectVisible(page.getByTestId('onboarding-source-preview'), 'onboarding source preview');

  await page.getByTestId('onboarding-use-demo').click();
  await expectVisible(page.getByTestId('onboarding-step-metadata'), 'onboarding metadata step');
  await expectVisible(page.getByTestId('onboarding-metadata-sources'), 'onboarding metadata sources');
  await expectVisible(page.getByTestId('onboarding-metadata-strategy-source'), 'onboarding source metadata strategy');
  await page.getByTestId('onboarding-next-metadata').click();
  await expectVisible(page.getByTestId('onboarding-emulator-list'), 'onboarding emulator list after metadata');
  await assertNoHorizontalOverflow(page, 'onboarding metadata');

  await page.getByTestId('onboarding-nav-emulator').click();
  await expectVisible(page.getByTestId('onboarding-emulator-list'), 'onboarding emulator list');
  await page.getByTestId('onboarding-install-nes').click();
  await expectVisible(page.getByTestId('onboarding-step-ready'), 'onboarding ready step');
  await page.getByTestId('onboarding-open-launcher').click();
  await expectVisible(page.getByTestId('app-shell'), 'app shell after setup');
  await expectVisible(page.getByTestId('home-screen'), 'home screen');
  await expectVisible(page.getByTestId('home-hero'), 'home hero');
  await screenshot(page, '02-home.png');
  await assertNoHorizontalOverflow(page, 'home');
  await assertHomeHintBarClearance(page, 'home');
  await assertHomeViewports(page);

  await clickNav(page, 'library');
  await page.waitForTimeout(250);
  await expectVisible(page.getByTestId('library-screen'), 'library screen');
  await expectVisible(page.getByTestId('library-grid'), 'library grid');
  await page.waitForTimeout(500);
  await assertLibraryCardsDoNotOverlap(page, 'library full grid');
  await page.getByTestId('library-search').fill('smoke');
  await page.getByTestId('library-grid').getByText('Fusion Launcher NES Smoke Demo').waitFor({ state: 'visible', timeout: timeoutMs });
  await page.waitForTimeout(700);
  await screenshot(page, '03-library-search.png');
  await assertNoHorizontalOverflow(page, 'library');

  await page.getByTestId('library-search').fill('star orbit');
  await page.getByTestId('library-grid').getByText('Star Orbit Prototype').waitFor({ state: 'visible', timeout: timeoutMs });
  await page.locator('[data-focus-zone="library"]').first().click();
  const gameDetails = page.getByTestId('game-details-modal');
  await expectVisible(gameDetails, 'game details modal');
  await page.getByTestId('setup-checklist').waitFor({ state: 'hidden', timeout: timeoutMs });
  console.log('PASS game setup checklist hidden by default');
  await gameDetails.getByRole('button', { name: 'Details', exact: true }).click();
  await expectVisible(page.getByTestId('setup-checklist'), 'game setup checklist');
  await expectVisible(page.getByTestId('setup-game-file-row'), 'game file setup row');
  await expectVisible(page.getByTestId('import-game-file'), 'import game file button');
  await screenshot(page, '04-game-setup.png');
  await assertNoHorizontalOverflow(page, 'game setup');
  await page.getByTestId('import-game-file').click();
  await page.getByText('Downloaded', { exact: true }).waitFor({ state: 'visible', timeout: timeoutMs });
  await screenshot(page, '05-game-imported.png');
  await assertNoHorizontalOverflow(page, 'game imported');
  await page.getByTitle('Close').click();

  await clickNav(page, 'collections');
  await page.waitForTimeout(250);
  await expectVisible(page.getByTestId('collections-screen'), 'collections screen');
  await expectVisible(page.getByTestId('collections-panel'), 'collections panel');
  await expectVisible(page.getByTestId('collection-card').first(), 'collection card');
  await screenshot(page, '06-collections.png');
  await assertNoHorizontalOverflow(page, 'collections');
  await page.getByTestId('collection-card').filter({ hasText: 'Ready to play' }).first().evaluate((node) => {
    if (!(node instanceof HTMLElement)) {
      throw new Error('Collection target is not clickable.');
    }
    node.click();
  });
  await expectVisible(page.getByTestId('library-screen'), 'library screen after collection');
  assert.equal(await page.getByTestId('library-search').inputValue(), 'ready to play');

  await clickNav(page, 'downloads');
  await page.waitForTimeout(250);
  await expectVisible(page.getByTestId('downloads-center'), 'downloads center');
  await expectVisible(page.getByTestId('download-row').first(), 'download row');
  await page.getByText(/(Active download in progress|Queue ready)/).waitFor({ state: 'visible', timeout: timeoutMs });
  await screenshot(page, '07-downloads.png');
  await assertNoHorizontalOverflow(page, 'downloads');
  await assertNestedScrollContainer(page.getByTestId('downloads-center'), '.rh-download-list', 'downloads list');

  assert.equal(await page.locator('[data-focus-id="nav:settings"]').count(), 0, 'settings should not be a sidebar nav target');
  await page.getByTestId('top-settings').click();
  await page.waitForTimeout(250);
  await expectVisible(page.getByTestId('settings-modal'), 'settings modal');
  await expectVisible(page.getByTestId('settings-modal-emulators'), 'settings modal emulators panel');
  await expectVisible(page.getByTestId('emulator-row-switch'), 'switch emulator row');
  await expectVisible(page.getByRole('button', { name: 'Choose Nintendo Switch executable' }), 'native executable picker button');
  await expectVisible(page.getByRole('button', { name: 'Save' }), 'settings modal save button');
  await assertNestedScrollContainer(page.getByTestId('settings-modal'), '[data-testid="settings-modal-emulators"]', 'settings modal emulators scroll');
  await assertLightweightSettingsCompositing(page);
  await assertResponsiveWheelScroll(page, page.getByTestId('settings-modal-emulators'), 'settings modal emulators wheel scroll');

  await page.getByTestId('settings-tab-sources').click();
  await expectVisible(page.getByTestId('settings-modal-sources'), 'settings modal sources tab');
  await expectVisible(page.getByTestId('settings-modal-sources-panel'), 'settings modal sources panel');
  await expectVisible(page.getByTestId('source-card').first(), 'connected source card');
  await page.getByTestId('settings-source-url').fill('https://community.example/fusion-launcher-repository.json');
  await page.getByTestId('settings-modal-sources-panel').getByRole('button', { name: 'Check' }).click();
  await expectVisible(page.getByTestId('source-preview'), 'settings source preview');
  await screenshot(page, '08-settings-modal-sources.png');
  await assertNoHorizontalOverflow(page, 'settings modal sources');

  await page.getByTestId('settings-tab-diagnostics').click();
  await expectVisible(page.getByTestId('settings-modal-diagnostics'), 'settings modal diagnostics tab');
  await page.getByTestId('settings-modal-diagnostics').getByRole('button', { name: 'Run' }).click();
  await page.getByTestId('settings-modal-diagnostics').getByText('Downloader session', { exact: true }).waitFor({ state: 'visible', timeout: timeoutMs });

  await page.getByTestId('settings-tab-updates').click();
  await expectVisible(page.getByTestId('settings-modal-updates'), 'settings modal updates tab');
  await expectVisible(page.getByTestId('settings-modal-updates-panel'), 'settings modal updates panel');
  await expectVisible(page.getByTestId('settings-modal-updates-panel').getByRole('button', { name: 'Check' }), 'settings modal update check button');

  await page.getByTestId('settings-tab-emulators').click();
  await expectVisible(page.getByTestId('settings-modal-emulators'), 'settings modal emulators panel');
  await expectVisible(page.getByTestId('emulator-row-switch'), 'switch emulator row');
  await screenshot(page, '09-settings-modal.png');
  await assertNoHorizontalOverflow(page, 'settings modal');
  await assertNestedScrollContainer(page.getByTestId('settings-modal'), '[data-testid="settings-modal-emulators"]', 'settings modal stable scroll');

  await page.getByTestId('settings-tab-general').click();
  await expectVisible(page.getByTestId('settings-modal-general'), 'settings modal general panel');
  await page.getByTestId('settings-language').selectOption('ru');
  await expectVisible(page.getByText('Changes are local until you save.', { exact: true }), 'settings language dirty state');
  await page.getByTestId('settings-modal').getByRole('button', { name: 'Save' }).click();
  await expectVisible(page.getByTitle('Закрыть настройки'), 'settings modal Russian language applied');
  await expectVisible(page.getByText('Основные', { exact: true }).first(), 'settings modal Russian general label');
  await screenshot(page, '10-settings-russian.png');
  await assertNoHorizontalOverflow(page, 'settings modal Russian');
  await page.getByTitle('Закрыть настройки').click();

  console.log(`MVP visual smoke passed. Screenshots: ${path.relative(root, screenshotsDir)}`);
}

async function startDevServer() {
  const port = await findFreePort();
  const url = `http://127.0.0.1:${port}`;
  const nextCli = path.join(root, 'node_modules', 'next', 'dist', 'bin', 'next');

  server = spawn(process.execPath, [nextCli, 'dev', '--hostname', '127.0.0.1', '--port', String(port)], {
    cwd: root,
    env: {
      ...process.env,
      NEXT_TELEMETRY_DISABLED: '1'
    },
    stdio: ['ignore', 'pipe', 'pipe']
  });

  server.stdout.on('data', (chunk) => pushServerLog(chunk));
  server.stderr.on('data', (chunk) => pushServerLog(chunk));

  await waitForHttp(url);
  return url;
}

async function launchChromium() {
  try {
    return await chromium.launch();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (message.includes('Executable') || message.includes('browser')) {
      throw new Error(`${message}\n\nInstall the browser once with: npx playwright install chromium`);
    }
    throw error;
  }
}

async function waitForHttp(url) {
  const startedAt = Date.now();

  while (Date.now() - startedAt < timeoutMs) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {
      // Server is still starting.
    }
    await delay(500);
  }

  throw new Error(`Timed out waiting for ${url}\n${serverLogTail()}`);
}

async function findFreePort() {
  return new Promise((resolve, reject) => {
    const socket = net.createServer();
    socket.unref();
    socket.on('error', reject);
    socket.listen(0, '127.0.0.1', () => {
      const address = socket.address();
      socket.close(() => {
        if (!address || typeof address === 'string') {
          reject(new Error('Could not allocate a local port.'));
          return;
        }
        resolve(address.port);
      });
    });
  });
}

async function expectVisible(locator, label) {
  await locator.waitFor({ state: 'visible', timeout: timeoutMs });
  console.log(`PASS visible ${label}`);
}

async function clickNav(page, view) {
  await page.locator(`[data-focus-id="nav:${view}"]`).evaluate((node) => {
    if (!(node instanceof HTMLElement)) {
      throw new Error('Navigation target is not clickable.');
    }
    node.click();
  });
}

async function screenshot(page, fileName) {
  const filePath = path.join(screenshotsDir, fileName);
  await page.screenshot({ path: filePath, fullPage: true });
  console.log(`PASS screenshot ${path.relative(root, filePath)}`);
}

async function assertHomeViewports(page) {
  const originalViewport = page.viewportSize();
  const viewports = [
    { width: 1280, height: 720, label: '1280' },
    { width: 1440, height: 900, label: '1440' },
    { width: 1920, height: 1080, label: '1920' }
  ];

  for (const viewport of viewports) {
    await page.setViewportSize({ width: viewport.width, height: viewport.height });
    await page.waitForTimeout(250);
    await expectVisible(page.getByTestId('home-screen'), `home screen ${viewport.label}`);
    await expectVisible(page.getByTestId('home-hero'), `home hero ${viewport.label}`);
    await screenshot(page, `02-home-${viewport.label}.png`);
    await assertNoHorizontalOverflow(page, `home ${viewport.label}`);
    await assertHomeHintBarClearance(page, `home ${viewport.label}`);
    if (viewport.width === 1280) {
      await assertNestedScrollContainer(page.locator('body'), '[data-testid="home-screen"]', 'home screen');
    }
  }

  if (originalViewport) {
    await page.setViewportSize(originalViewport);
    await page.waitForTimeout(250);
  }
}

async function assertNoHorizontalOverflow(page, label) {
  const metrics = await page.evaluate(() => ({
    clientWidth: document.documentElement.clientWidth,
    scrollWidth: document.documentElement.scrollWidth,
    bodyScrollWidth: document.body.scrollWidth
  }));
  const maxScrollWidth = Math.max(metrics.scrollWidth, metrics.bodyScrollWidth);
  assert.ok(
    maxScrollWidth <= metrics.clientWidth + 1,
    `${label} has horizontal overflow: ${maxScrollWidth}px > ${metrics.clientWidth}px`
  );
  console.log(`PASS no horizontal overflow ${label}`);
}

async function assertHomeHintBarClearance(page, label) {
  const metrics = await page.evaluate(() => {
    const home = document.querySelector('[data-testid="home-screen"]');
    const hintBar = document.querySelector('.rh-hint-bar');
    if (!(home instanceof HTMLElement) || !(hintBar instanceof HTMLElement)) {
      throw new Error('Missing Home screen or hint bar.');
    }

    const homeRect = home.getBoundingClientRect();
    const hintRect = hintBar.getBoundingClientRect();
    return {
      homeBottom: homeRect.bottom,
      hintTop: hintRect.top
    };
  });

  assert.ok(
    metrics.homeBottom <= metrics.hintTop + 1,
    `${label} overlaps hint bar: home bottom ${metrics.homeBottom}px > hint top ${metrics.hintTop}px`
  );
  console.log(`PASS hint bar clearance ${label}`);
}

async function assertLibraryCardsDoNotOverlap(page, label) {
  const metrics = await page.evaluate(() => {
    const cards = Array.from(document.querySelectorAll('.rh-library-grid > .rh-game-card'))
      .map((card, index) => {
        const rect = card.getBoundingClientRect();
        return {
          index,
          left: rect.left,
          top: rect.top,
          right: rect.right,
          bottom: rect.bottom,
          width: rect.width,
          height: rect.height,
          label: card.textContent?.replace(/\s+/g, ' ').trim().slice(0, 80) ?? `card ${index}`
        };
      })
      .filter((rect) => rect.width > 0 && rect.height > 0);

    const tolerance = 1;
    const overlaps = [];
    for (let leftIndex = 0; leftIndex < cards.length; leftIndex += 1) {
      for (let rightIndex = leftIndex + 1; rightIndex < cards.length; rightIndex += 1) {
        const first = cards[leftIndex];
        const second = cards[rightIndex];
        const overlapping = first.left < second.right - tolerance
          && first.right > second.left + tolerance
          && first.top < second.bottom - tolerance
          && first.bottom > second.top + tolerance;
        if (overlapping) {
          overlaps.push({
            first,
            second
          });
        }
      }
    }

    return {
      cardCount: cards.length,
      overlaps: overlaps.slice(0, 3)
    };
  });

  assert.ok(metrics.cardCount > 1, `${label} should render multiple cards before filtering`);
  assert.deepEqual(metrics.overlaps, [], `${label} has overlapping game cards`);
  console.log(`PASS no overlapping cards ${label}`);
}

async function assertNestedScrollContainer(locator, selector, label) {
  const metrics = await locator.evaluate((node, targetSelector) => {
    const target = node.querySelector(targetSelector);
    if (!(target instanceof HTMLElement)) {
      throw new Error(`Missing scroll target: ${targetSelector}`);
    }

    const before = target.scrollTop;
    target.scrollTop = target.scrollHeight;
    const after = target.scrollTop;
    target.scrollTop = before;

    return {
      clientHeight: target.clientHeight,
      overflowY: getComputedStyle(target).overflowY,
      scrollHeight: target.scrollHeight,
      scrollTopAfterJump: after
    };
  }, selector);

  assertScrollableMetrics(metrics, label);
}

async function assertLightweightSettingsCompositing(page) {
  const styles = await page.getByTestId('settings-modal').evaluate((modal) => {
    const dialog = modal.querySelector('[role="dialog"]');
    if (!(dialog instanceof HTMLElement)) {
      throw new Error('Missing settings dialog.');
    }

    return {
      backdropFilter: getComputedStyle(modal).backdropFilter,
      dialogTransform: getComputedStyle(dialog).transform
    };
  });

  assert.ok(styles.backdropFilter === 'none' || styles.backdropFilter.includes('blur'), 'settings overlay backdrop-filter should be stable');
  assert.equal(styles.dialogTransform, 'none', 'settings scroll container must not live inside a transformed dialog');
  console.log('PASS lightweight settings compositing');
}

async function assertResponsiveWheelScroll(page, locator, label) {
  await locator.evaluate((node) => {
    node.scrollTop = 0;
  });
  const box = await locator.boundingBox();
  assert.ok(box, `${label} has no bounding box`);

  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  const startedAt = Date.now();
  for (let index = 0; index < 8; index += 1) {
    await page.mouse.wheel(0, 320);
  }
  const elapsedMs = Date.now() - startedAt;
  const scrollTop = await locator.evaluate((node) => node.scrollTop);

  assert.ok(scrollTop > 0, `${label} did not move after wheel input`);
  assert.ok(elapsedMs < 3_000, `${label} became unresponsive for ${elapsedMs}ms`);
  console.log(`PASS responsive wheel scroll ${label} (${elapsedMs}ms)`);
}

function assertScrollableMetrics(metrics, label) {
  assert.match(metrics.overflowY, /auto|scroll/, `${label} does not expose vertical scrolling`);
  if (metrics.scrollHeight > metrics.clientHeight + 1) {
    assert.ok(
      metrics.scrollTopAfterJump > 0,
      `${label} has overflow but cannot scroll: ${metrics.scrollHeight}px > ${metrics.clientHeight}px`
    );
  }
  console.log(`PASS scroll container ${label}`);
}

function pushServerLog(chunk) {
  serverLogs.push(chunk.toString());
  if (serverLogs.length > 40) serverLogs.shift();
}

function serverLogTail() {
  return serverLogs.join('').trim();
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function cleanup() {
  if (browser) await browser.close().catch(() => {});
  if (!server) return;
  server.kill('SIGTERM');
}

try {
  await main();
} catch (error) {
  console.error('MVP visual smoke failed.');
  console.error(error instanceof Error ? error.message : String(error));
  const tail = serverLogTail();
  if (tail) {
    console.error('\nServer log tail:');
    console.error(tail);
  }
  process.exitCode = 1;
} finally {
  await cleanup();
}
