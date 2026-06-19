import assert from 'node:assert/strict';
import test from 'node:test';

test('scraperApi routes metadata commands through preview runtime', async () => {
  const previousNodeEnv = process.env.NODE_ENV;
  process.env.NODE_ENV = 'development';
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: {
      location: {
        hostname: 'localhost',
        port: '3000'
      }
    }
  });

  const { scraperApi } = await import('./scraperApi.ts');

  let status = await scraperApi.getScreenscraperStatus();
  assert.equal(status.configured, false);

  let sgdbStatus = await scraperApi.getSteamgriddbStatus();
  assert.equal(sgdbStatus.configured, false);

  sgdbStatus = await scraperApi.saveSteamgriddbKey('sgdb-preview-key');
  assert.equal(sgdbStatus.configured, true);
  assert.equal(sgdbStatus.keySource, 'user');

  await scraperApi.scrapeGame('crystal-caverns');
  const artworkOnlyState = await scraperApi.getScrapeState('crystal-caverns');
  assert.equal(artworkOnlyState.status, 'ready');
  assert.equal(artworkOnlyState.matchKind, 'artwork');

  status = await scraperApi.saveScreenscraperCredentials('preview-user', 'secret', 'us');
  assert.equal(status.configured, true);
  assert.equal(status.region, 'us');

  await scraperApi.scrapeGame('crystal-caverns');
  const scrapeState = await scraperApi.getScrapeState('crystal-caverns');
  assert.equal(scrapeState.status, 'ready');
  assert.equal(scrapeState.matchKind, 'hash');

  const batchStatus = await scraperApi.scrapeLibrary();
  assert.equal(batchStatus.running, false);
  assert.equal(batchStatus.pending, 0);

  process.env.NODE_ENV = previousNodeEnv;
});
