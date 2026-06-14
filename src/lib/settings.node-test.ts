import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { normalizeSettings } from './settings.ts';

describe('settings normalization', () => {
  it('defaults missing language to English', () => {
    assert.equal(normalizeSettings({ emulators: {}, emulatorConfigs: {} }).language, 'en');
  });

  it('preserves supported language values', () => {
    assert.equal(normalizeSettings({ emulators: {}, emulatorConfigs: {}, language: 'ru' }).language, 'ru');
  });

  it('normalizes unknown language values to English', () => {
    assert.equal(normalizeSettings({
      emulators: {},
      emulatorConfigs: {},
      language: 'fr' as never
    }).language, 'en');
  });

  it('defaults metadata onboarding to incomplete source strategy', () => {
    assert.deepEqual(normalizeSettings({ emulators: {}, emulatorConfigs: {} }).metadataOnboarding, {
      complete: false,
      strategy: 'source'
    });
  });

  it('preserves completed metadata onboarding strategy', () => {
    assert.deepEqual(normalizeSettings({
      emulators: {},
      emulatorConfigs: {},
      metadataOnboarding: {
        complete: true,
        strategy: 'screenscraper'
      }
    }).metadataOnboarding, {
      complete: true,
      strategy: 'screenscraper'
    });
  });

  it('normalizes invalid metadata onboarding strategy to source', () => {
    assert.deepEqual(normalizeSettings({
      emulators: {},
      emulatorConfigs: {},
      metadataOnboarding: {
        complete: true,
        strategy: 'steam' as never
      }
    }).metadataOnboarding, {
      complete: true,
      strategy: 'source'
    });
  });
});
