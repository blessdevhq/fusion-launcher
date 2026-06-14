import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  validateSourceLibrary,
  validateSourceLibraryObject
} from './validate-source-library.mjs';

const templateUrl = new URL('../templates/source-library/repository.json', import.meta.url);
const templatePath = fileURLToPath(templateUrl);

async function loadTemplate() {
  return JSON.parse(await readFile(templateUrl, 'utf8'));
}

async function mutateTemplate(mutator) {
  const repository = await loadTemplate();
  mutator(repository);
  return repository;
}

describe('source library validator', () => {
  it('passes the clean source library template', async () => {
    const report = await validateSourceLibrary(templatePath);

    assert.deepEqual(report.errors, []);
    assert.equal(report.repository.metadata.id, 'fusion-launcher-source-template');
  });

  it('fails on duplicate catalog ids', async () => {
    const repository = await mutateTemplate((draft) => {
      draft.catalog[1].id = draft.catalog[0].id;
    });

    const report = validateSourceLibraryObject(repository);

    assert.match(report.errors.join('\n'), /Duplicate catalog game id "template_homebrew_http"/);
  });

  it('fails on invalid expected extensions', async () => {
    const repository = await mutateTemplate((draft) => {
      draft.catalog[0].expectedExtensions = ['nes'];
    });

    const report = validateSourceLibraryObject(repository);

    assert.match(report.errors.join('\n'), /Extensions must start with/);
  });

  it('fails on bundled sources in user templates', async () => {
    const repository = await mutateTemplate((draft) => {
      draft.catalog[0].downloads = [{
        kind: 'bundled',
        path: 'demo-content/example.nes',
        sha256: 'a'.repeat(64)
      }];
    });

    const report = validateSourceLibraryObject(repository);

    assert.match(report.errors.join('\n'), /bundled sources are only allowed/);
  });

  it('warns when BIOS assets expose automatic download sources', async () => {
    const repository = await mutateTemplate((draft) => {
      draft.system_files[0].sources = [{
        kind: 'http',
        url: 'https://example.com/system/scph5501.bin',
        sha256: 'b'.repeat(64)
      }];
    });

    const report = validateSourceLibraryObject(repository);

    assert.deepEqual(report.errors, []);
    assert.match(report.warnings.join('\n'), /bios assets should normally use user_provided/);
  });
});
