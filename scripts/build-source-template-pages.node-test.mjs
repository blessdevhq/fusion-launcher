import assert from 'node:assert/strict';
import { access, mkdir, readFile, writeFile } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import path from 'node:path';
import { promisify } from 'node:util';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';

const execFileAsync = promisify(execFile);
const repoRoot = path.resolve(fileURLToPath(new URL('..', import.meta.url)));
const checkRoot = path.join(repoRoot, '.tmp', 'source-library-template-pages-check');
const scriptPath = path.join(repoRoot, 'scripts', 'build-source-template-pages.mjs');

describe('source template Pages artifact builder', () => {
  it('generates a clean public template artifact', async () => {
    const stalePath = path.join(checkRoot, 'stale.txt');
    await mkdir(checkRoot, { recursive: true });
    await writeFile(stalePath, 'stale');

    await execFileAsync(process.execPath, ['--experimental-strip-types', scriptPath, '--check'], {
      cwd: repoRoot
    });

    await assert.rejects(() => access(stalePath));

    const manifestPath = path.join(checkRoot, 'source-library-template', 'manifest.json');
    const repositoryPath = path.join(checkRoot, 'source-library-template', 'repository.json');
    const indexPath = path.join(checkRoot, 'source-library-template', 'index.html');
    const landingPath = path.join(checkRoot, 'index.html');
    const appIconPath = path.join(checkRoot, 'fusion', 'app-icon.png');

    const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
    const repository = JSON.parse(await readFile(repositoryPath, 'utf8'));
    const indexHtml = await readFile(indexPath, 'utf8');
    const landingHtml = await readFile(landingPath, 'utf8');

    await access(appIconPath);
    assert.equal(manifest.publicUrl, 'https://mrbeastie.github.io/fusion-launcher/source-library-template/repository.json');
    assert.equal(manifest.trustLevel, 'community');
    assert.equal(manifest.templateId, repository.metadata.id);
    assert.equal(manifest.catalogCount, repository.catalog.length);
    assert.match(indexHtml, /repository\.json<\/a> \| <a href="\.\/README\.md">README\.md<\/a> \|/);
    assert.match(landingHtml, /retro and console games on PC/i);
    assert.match(landingHtml, /Content-neutral by design/);
    assert.match(landingHtml, /\.\/fusion\/app-icon\.png/);
    assert.doesNotMatch(indexHtml, /В/);
  });
});
