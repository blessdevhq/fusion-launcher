import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';

const root = process.cwd();
const excludedDirs = new Set([
  '.git',
  '.next',
  '.npm-cache',
  '.tmp',
  'node_modules',
  'out',
  'src-tauri/target'
]);

const patterns = [
  /YOUR_CDN_URL/,
  /TODO_.*MAGNET/,
  /systemFilesBaseUrl/,
  /seedGames/,
  /emulator_download_url/,
  /trailer_url: 'YOUR_CDN_URL'/
];

const textExtensions = new Set([
  '.cjs',
  '.css',
  '.html',
  '.js',
  '.json',
  '.jsx',
  '.lock',
  '.md',
  '.mjs',
  '.rs',
  '.toml',
  '.ts',
  '.tsx',
  '.yml'
]);

const findings = [];

function normalizedRelative(filePath) {
  return path.relative(root, filePath).replaceAll(path.sep, '/');
}

function isExcluded(filePath) {
  const relative = normalizedRelative(filePath);
  if (relative === 'scripts/static-check.mjs') return true;
  return Array.from(excludedDirs).some((dir) => relative === dir || relative.startsWith(`${dir}/`));
}

async function walk(dir) {
  if (isExcluded(dir)) return;

  const entries = await readdir(dir, { withFileTypes: true });
  for (const entry of entries) {
    const filePath = path.join(dir, entry.name);
    if (isExcluded(filePath)) continue;
    if (entry.isDirectory()) {
      await walk(filePath);
      continue;
    }
    if (!entry.isFile() || !textExtensions.has(path.extname(entry.name))) continue;

    const content = await readFile(filePath, 'utf8');
    const lines = content.split(/\r?\n/);
    lines.forEach((line, index) => {
      if (patterns.some((pattern) => pattern.test(line))) {
        findings.push(`${normalizedRelative(filePath)}:${index + 1}: ${line.trim()}`);
      }
    });
  }
}

await walk(root);

if (findings.length > 0) {
  console.error('Forbidden legacy/CDN markers found:');
  for (const finding of findings) console.error(finding);
  process.exit(1);
}

console.log('Static check passed: no forbidden legacy/CDN markers found.');
