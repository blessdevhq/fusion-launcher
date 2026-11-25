import { access, readdir, readFile, stat } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import path from 'node:path';

const root = process.cwd();
const repo = 'blessdevhq/fusion-launcher';
const canonicalHomepage = 'https://blessdevhq.github.io/fusion-launcher/';
const strict = process.argv.includes('--strict');

const passes = [];
const warnings = [];
const failures = [];

function rel(filePath) {
  return path.relative(root, filePath).replaceAll(path.sep, '/');
}

function pass(message) {
  passes.push(message);
}

function warn(message) {
  warnings.push(message);
}

function fail(message) {
  failures.push(message);
}

async function exists(filePath) {
  try {
    await access(path.join(root, filePath));
    return true;
  } catch {
    return false;
  }
}

async function readText(filePath) {
  return readFile(path.join(root, filePath), 'utf8');
}

async function readJson(filePath) {
  return JSON.parse(await readText(filePath));
}

async function checkRequiredFiles() {
  const required = [
    'README.md',
    'README.ru.md',
    'LICENSE',
    'CHANGELOG.md',
    'docs/release-checklist.md',
    'public/fusion/hero-key-visual.png',
    'public/fusion/og-image.png',
    'public/source-libraries/official-starter.json',
    'public/demo-content/LICENSE.txt'
  ];

  const missing = [];
  for (const file of required) {
    if (!(await exists(file))) missing.push(file);
  }

  if (missing.length === 0) {
    pass('Minimum public launch files are present.');
  } else {
    fail(`Missing public launch files: ${missing.join(', ')}`);
  }
}

async function checkVersions() {
  const pkg = await readJson('package.json');
  const tauri = await readJson('src-tauri/tauri.conf.json');

  if (pkg.version === tauri.version) {
    pass(`Package and Tauri versions match (${pkg.version}).`);
  } else {
    fail(`package.json version (${pkg.version}) does not match src-tauri/tauri.conf.json version (${tauri.version}).`);
  }
}

async function checkLegalCopy() {
  const readme = await readText('README.md');
  const ruReadme = await readText('README.ru.md');

  const requiredSnippets = [
    'does not host',
    'commercial ROMs',
    'BIOS',
    'firmware',
    'keys',
    'Users are responsible'
  ];
  const missing = requiredSnippets.filter((snippet) => !readme.includes(snippet));

  if (missing.length === 0 && ruReadme.includes('не хостит') && ruReadme.includes('BIOS')) {
    pass('Legal content model is visible in English and Russian READMEs.');
  } else {
    fail(`Legal content model copy is incomplete. Missing/weak snippets: ${missing.join(', ') || 'Russian legal wording'}.`);
  }
}

async function checkSourceLibraries() {
  const official = await readJson('public/source-libraries/official-starter.json');
  const wishlist = await readJson('public/source-libraries/unofficial-player-wishlist.json');

  if (official.metadata?.trustLevel === 'official') {
    pass('Official starter source declares trustLevel=official.');
  } else {
    fail('Official starter source must declare metadata.trustLevel=official.');
  }

  if (Array.isArray(official.system_files) && official.system_files.length === 0) {
    pass('Official starter source contains no BIOS/firmware/key system files.');
  } else {
    fail('Official starter source must not require BIOS, firmware, or keys.');
  }

  const wishlistDownloadIssues = [];
  for (const item of wishlist.catalog ?? []) {
    for (const download of item.downloads ?? []) {
      if (download.kind !== 'user_provided') {
        wishlistDownloadIssues.push(`${item.id}: ${download.kind}`);
      }
    }
  }
  for (const systemFile of wishlist.system_files ?? []) {
    for (const source of systemFile.sources ?? []) {
      if (source.kind !== 'user_provided') {
        wishlistDownloadIssues.push(`${systemFile.id}: ${source.kind}`);
      }
    }
  }

  if (wishlistDownloadIssues.length === 0) {
    pass('Unofficial wishlist is metadata/user-provided only.');
  } else {
    fail(`Unofficial wishlist contains non-user-provided payload sources: ${wishlistDownloadIssues.join(', ')}`);
  }
}

async function walk(dir, files = []) {
  const entries = await readdir(dir, { withFileTypes: true });
  for (const entry of entries) {
    const filePath = path.join(dir, entry.name);
    const relative = rel(filePath);
    if (entry.isDirectory()) {
      if (['.git', '.next', '.npm-cache', 'node_modules', 'out'].includes(entry.name)) continue;
      if (relative.startsWith('src-tauri/target')) continue;
      await walk(filePath, files);
      continue;
    }
    if (!entry.isFile()) continue;
    if (!['.md', '.mdx', '.ts', '.tsx', '.js', '.mjs', '.json', '.html', '.yml', '.yaml'].includes(path.extname(entry.name))) continue;
    files.push(filePath);
  }
  return files;
}

function isPublicLaunchSurface(relative) {
  if (relative === 'README.md' || relative === 'README.ru.md' || relative === 'CHANGELOG.md') return true;
  if (relative === 'catalog.unofficial.json') return true;
  if (relative === 'src/lib/i18n.ts') return true;
  if (relative.startsWith('docs/') && relative.endsWith('.md')) return true;
  if (relative.startsWith('public/') && ['.json', '.html', '.md'].includes(path.extname(relative))) return true;
  return false;
}

function isSafetyDisclaimer(line) {
  return /\b(does not|do not|not|no|without|user-provided|user provided|legally owned|legally obtained)\b/i.test(line);
}

async function checkRiskyCopy() {
  const files = await walk(root);
  const riskyPatterns = [
    { label: 'commercial game title as acquisition hook', regex: /\b(TOTK|Tears of the Kingdom)\b/i, allowDisclaimer: false },
    { label: 'deprecated emulator brand in public copy', regex: /\bYuzu\b/i, allowDisclaimer: false },
    { label: 'raw magnet URI in public catalog/copy', regex: /magnet:\?xt=urn:btih/i, allowDisclaimer: false },
    { label: 'ROM magnet phrasing', regex: /\bROM\s+magnet\b/i, allowDisclaimer: false },
    { label: 'manifest ships ROM phrasing', regex: /manifest\s+(can\s+)?include[s]?\s+the\s+ROM/i, allowDisclaimer: false },
    { label: 'bundled proprietary system files', regex: /\b(BIOS|firmware|keys|prod\.keys)\b.{0,80}\b(included|bundled|preconfigured|ships|downloaded)\b/i, allowDisclaimer: true }
  ];

  const findings = [];
  for (const filePath of files) {
    const relative = rel(filePath);
    if (!isPublicLaunchSurface(relative)) continue;
    if (relative.startsWith('docs/marketing/')) continue;
    const content = await readFile(filePath, 'utf8');
    const lines = content.split(/\r?\n/);
    lines.forEach((line, index) => {
      if (line.includes('launch-audit: allow-risk-example')) return;
      for (const pattern of riskyPatterns) {
        if (pattern.allowDisclaimer && isSafetyDisclaimer(line)) continue;
        if (pattern.regex.test(line)) {
          findings.push(`${relative}:${index + 1}: ${pattern.label}: ${line.trim()}`);
        }
      }
    });
  }

  if (findings.length === 0) {
    pass('No high-risk public positioning phrases found.');
  } else {
    warn(`Risky public copy needs review:\n${findings.map((item) => `  - ${item}`).join('\n')}`);
  }
}

function runGhJson(args) {
  try {
    return JSON.parse(execFileSync('gh', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }));
  } catch (error) {
    warn(`GitHub check skipped/failed: ${error.message}`);
    return null;
  }
}

async function checkGitHubSurface() {
  const pkg = await readJson('package.json');
  const repoInfo = runGhJson(['repo', 'view', repo, '--json', 'homepageUrl,description,licenseInfo,repositoryTopics']);
  if (repoInfo) {
    if (repoInfo.homepageUrl === canonicalHomepage) {
      pass(`GitHub homepage points at ${canonicalHomepage}.`);
    } else {
      fail(`GitHub homepage is ${repoInfo.homepageUrl || '(empty)'}, expected ${canonicalHomepage}.`);
    }

    if (repoInfo.licenseInfo?.key === 'gpl-3.0') {
      pass('GitHub detects the GPL-3.0 license.');
    } else {
      warn(`GitHub license detection is ${repoInfo.licenseInfo?.key ?? 'missing'}.`);
    }
  }

  const releaseList = (() => {
    try {
      return execFileSync('gh', ['release', 'list', '--repo', repo, '--limit', '1'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
    } catch (error) {
      warn(`GitHub release check skipped/failed: ${error.message}`);
      return '';
    }
  })();

  if (releaseList) {
    const latestTag = releaseList.split(/\t/)[2];
    const expectedTag = `v${pkg.version}`;
    if (latestTag === expectedTag) {
      pass(`Latest GitHub release matches package version (${expectedTag}).`);
    } else {
      warn(`Latest GitHub release is ${latestTag}; package version expects ${expectedTag}. Publish or adjust before a hard launch.`);
    }
  }
}

async function checkHomepageAvailability() {
  try {
    const response = await fetch(canonicalHomepage, { redirect: 'follow' });
    if (response.ok) {
      pass(`GitHub Pages homepage is reachable (${response.status}).`);
    } else {
      fail(`GitHub Pages homepage returned HTTP ${response.status}.`);
    }
  } catch (error) {
    warn(`Homepage availability check skipped/failed: ${error.message}`);
  }
}

async function checkAssetSizes() {
  const assets = ['public/fusion/hero-key-visual.png', 'public/fusion/og-image.png'];
  for (const asset of assets) {
    try {
      const info = await stat(path.join(root, asset));
      if (info.size > 10_000) {
        pass(`${asset} exists and is non-trivial (${info.size} bytes).`);
      } else {
        warn(`${asset} exists but is very small (${info.size} bytes).`);
      }
    } catch {
      fail(`${asset} is missing.`);
    }
  }
}

await checkRequiredFiles();
await checkVersions();
await checkLegalCopy();
await checkSourceLibraries();
await checkAssetSizes();
await checkRiskyCopy();
await checkGitHubSurface();
await checkHomepageAvailability();

console.log('\nLaunch audit');
console.log('============');
for (const item of passes) console.log(`PASS  ${item}`);
for (const item of warnings) console.warn(`WARN  ${item}`);
for (const item of failures) console.error(`FAIL  ${item}`);

if (failures.length > 0 || (strict && warnings.length > 0)) {
  console.error(`\nLaunch audit failed: ${failures.length} failure(s), ${warnings.length} warning(s).`);
  process.exit(1);
}

console.log(`\nLaunch audit passed with ${warnings.length} warning(s).${strict ? '' : ' Run with --strict to fail on warnings.'}`);
