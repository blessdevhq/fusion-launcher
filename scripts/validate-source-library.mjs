import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { repositorySchema } from '../src/types/repository.ts';

const BUILT_IN_REPOSITORY_IDS = new Set(['fusion-launcher-demo', 'retrohydra-demo']);
const BUILT_IN_SETUP_PROFILE_IDS = new Set([
  'nes-mesen',
  'snes-mesen',
  'n64-rmg',
  'gba-mgba',
  'ps2-pcsx2',
  'psp-ppsspp',
  'ps1-manual',
  'switch-manual',
  'snes-manual',
  'ps2-manual',
  'psp-manual'
]);
const RISKY_SYSTEM_FILE_KINDS = new Set(['bios', 'firmware', 'keys']);
const DOWNLOAD_SOURCE_KINDS = new Set(['http', 'magnet']);

export async function validateSourceLibrary(filePath, options = {}) {
  const resolvedPath = path.resolve(filePath);
  const report = {
    filePath: resolvedPath,
    errors: [],
    warnings: [],
    repository: null
  };

  let raw;
  try {
    raw = await readFile(resolvedPath, 'utf8');
  } catch (error) {
    report.errors.push(`Failed to read source library: ${error.message}`);
    return report;
  }

  let input;
  try {
    input = JSON.parse(raw);
  } catch (error) {
    report.errors.push(`Invalid JSON: ${error.message}`);
    return report;
  }

  return validateSourceLibraryObject(input, { ...options, filePath: resolvedPath });
}

export function validateSourceLibraryObject(input, options = {}) {
  const report = {
    filePath: options.filePath ?? 'repository.json',
    errors: [],
    warnings: [],
    repository: null
  };

  collectEmptyStringErrors(input, 'repository', report.errors);

  const parsed = repositorySchema.safeParse(input);
  if (!parsed.success) {
    for (const issue of parsed.error.issues) {
      report.errors.push(formatSchemaIssue(issue));
    }
    return report;
  }

  const repository = parsed.data;
  report.repository = repository;

  collectDuplicateIds(
    repository.catalog.map((game) => game.id),
    'catalog game',
    report.errors
  );
  collectDuplicateIds(
    repository.system_files.map((asset) => asset.id),
    'system file',
    report.errors
  );

  const allowBundled = options.allowBundled === true || BUILT_IN_REPOSITORY_IDS.has(repository.metadata.id);
  for (const [assetIndex, asset] of repository.system_files.entries()) {
    for (const [sourceIndex, source] of asset.sources.entries()) {
      const sourcePath = `system_files[${assetIndex}].sources[${sourceIndex}]`;
      validateSourcePolicy(source, sourcePath, allowBundled, report.errors);
      if (RISKY_SYSTEM_FILE_KINDS.has(asset.assetKind) && DOWNLOAD_SOURCE_KINDS.has(source.kind)) {
        report.warnings.push(
          `${sourcePath}: ${asset.assetKind} assets should normally use user_provided sources instead of ${source.kind}.`
        );
      }
    }
  }

  for (const [gameIndex, game] of repository.catalog.entries()) {
    if (game.setupProfileId && !BUILT_IN_SETUP_PROFILE_IDS.has(game.setupProfileId)) {
      report.warnings.push(
        `catalog[${gameIndex}].setupProfileId: unknown built-in profile "${game.setupProfileId}"; the game will remain visible but setup will be unsupported.`
      );
    }
    for (const [sourceIndex, source] of game.downloads.entries()) {
      const sourcePath = `catalog[${gameIndex}].downloads[${sourceIndex}]`;
      validateSourcePolicy(source, sourcePath, allowBundled, report.errors);
      if (game.contentMode === 'metadata_only' && DOWNLOAD_SOURCE_KINDS.has(source.kind)) {
        report.warnings.push(`${sourcePath}: metadata_only entries should not expose automatic download sources.`);
      }
      if (game.contentMode === 'user_provided' && DOWNLOAD_SOURCE_KINDS.has(source.kind)) {
        report.warnings.push(`${sourcePath}: user_provided entries should use user_provided download instructions.`);
      }
    }
  }

  return report;
}

export async function runCli(args = process.argv.slice(2)) {
  const targets = args.filter((arg) => !arg.startsWith('--'));
  if (targets.length === 0) {
    console.error('Usage: npm run source:validate -- <repository.json> [more-repositories.json]');
    process.exitCode = 2;
    return;
  }

  let hasErrors = false;
  for (const target of targets) {
    const report = await validateSourceLibrary(target);
    printReport(report);
    if (report.errors.length > 0) {
      hasErrors = true;
    }
  }

  process.exitCode = hasErrors ? 1 : 0;
}

function validateSourcePolicy(source, sourcePath, allowBundled, errors) {
  if (source.kind === 'bundled' && !allowBundled) {
    errors.push(`${sourcePath}: bundled sources are only allowed for the built-in Fusion Launcher demo repository.`);
  }
}

function collectDuplicateIds(ids, label, errors) {
  const seen = new Set();
  for (const id of ids) {
    if (seen.has(id)) {
      errors.push(`Duplicate ${label} id "${id}".`);
      continue;
    }
    seen.add(id);
  }
}

function collectEmptyStringErrors(value, location, errors) {
  if (typeof value === 'string') {
    if (value.trim() === '') {
      errors.push(`${location}: empty strings are not allowed; omit optional fields instead.`);
    }
    return;
  }

  if (Array.isArray(value)) {
    value.forEach((item, index) => collectEmptyStringErrors(item, `${location}[${index}]`, errors));
    return;
  }

  if (!value || typeof value !== 'object') {
    return;
  }

  for (const [key, item] of Object.entries(value)) {
    collectEmptyStringErrors(item, `${location}.${key}`, errors);
  }
}

function formatSchemaIssue(issue) {
  const location = issue.path.length > 0 ? issue.path.join('.') : 'repository';
  return `${location}: ${issue.message}`;
}

function printReport(report) {
  const relativePath = path.relative(process.cwd(), report.filePath) || report.filePath;
  for (const warning of report.warnings) {
    console.warn(`[warn] ${relativePath}: ${warning}`);
  }
  for (const error of report.errors) {
    console.error(`[error] ${relativePath}: ${error}`);
  }
  if (report.errors.length === 0) {
    console.log(`[ok] ${relativePath}`);
  }
}

function isDirectRun() {
  if (!process.argv[1]) return false;
  return import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href;
}

if (isDirectRun()) {
  await runCli();
}
