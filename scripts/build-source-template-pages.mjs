import { createHash } from 'node:crypto';
import { cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { validateSourceLibraryObject } from './validate-source-library.mjs';

const PUBLIC_TEMPLATE_URL = 'https://blessdevhq.github.io/fusion-launcher/source-library-template/repository.json';
const PUBLIC_BASE_URL = 'https://blessdevhq.github.io/fusion-launcher';
const GITHUB_REPO_URL = 'https://github.com/blessdevhq/fusion-launcher';
const TEMPLATE_DIR = path.resolve('templates/source-library');
const FUSION_ASSET_DIR = path.resolve('public/fusion');
const DEMO_CONTENT_DIR = path.resolve('public/demo-content');
const DEFAULT_OUTPUT_DIR = path.resolve('out');
const CHECK_OUTPUT_DIR = path.resolve('.tmp/source-library-template-pages-check');

async function main(args = process.argv.slice(2)) {
  const mode = args.includes('--check') ? 'check' : 'build';
  const outputRoot = mode === 'check' ? CHECK_OUTPUT_DIR : DEFAULT_OUTPUT_DIR;
  const artifact = await buildSourceTemplateArtifact(outputRoot);

  if (mode === 'check') {
    assertArtifact(artifact);
    console.log(`[ok] source library template artifact is reproducible at ${path.relative(process.cwd(), outputRoot)}`);
    return;
  }

  console.log(`[ok] source library template published at ${path.relative(process.cwd(), artifact.directory)}`);
  console.log(`[ok] ${PUBLIC_TEMPLATE_URL}`);
}

async function buildSourceTemplateArtifact(outputRoot) {
  const repositoryPath = path.join(TEMPLATE_DIR, 'repository.json');
  const readmePath = path.join(TEMPLATE_DIR, 'README.md');
  const repositoryRaw = await readFile(repositoryPath, 'utf8');
  const readmeRaw = await readFile(readmePath, 'utf8');
  const repository = JSON.parse(repositoryRaw);
  const report = validateSourceLibraryObject(repository, { filePath: repositoryPath });

  if (report.errors.length > 0) {
    throw new Error(`Source template is invalid:\n${report.errors.join('\n')}`);
  }

  const artifactDirectory = path.join(outputRoot, 'source-library-template');
  const repositoryHash = createHash('sha256').update(repositoryRaw).digest('hex');
  const manifest = {
    name: repository.metadata.name,
    templateId: repository.metadata.id,
    version: repository.metadata.version,
    schemaVersion: repository.metadata.schemaVersion,
    trustLevel: repository.metadata.trustLevel,
    publicUrl: PUBLIC_TEMPLATE_URL,
    repositoryJson: './repository.json',
    readme: './README.md',
    repositorySha256: repositoryHash,
    catalogCount: repository.catalog.length,
    systemFileCount: repository.system_files.length,
    updatedAt: repository.metadata.updatedAt
  };

  await rm(outputRoot, { recursive: true, force: true });
  await mkdir(artifactDirectory, { recursive: true });
  await writeFile(path.join(outputRoot, '.nojekyll'), '');
  await writeFile(path.join(artifactDirectory, 'repository.json'), repositoryRaw);
  await writeFile(path.join(artifactDirectory, 'README.md'), readmeRaw);
  await writeFile(path.join(artifactDirectory, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  await writeFile(path.join(artifactDirectory, 'index.html'), renderIndexHtml(manifest));
  await buildLandingPage(outputRoot, manifest);

  return {
    directory: artifactDirectory,
    manifest,
    repository,
    repositoryRaw,
    readmeRaw
  };
}

async function buildLandingPage(outputRoot, manifest) {
  await cp(FUSION_ASSET_DIR, path.join(outputRoot, 'fusion'), { recursive: true, force: true });
  await cp(DEMO_CONTENT_DIR, path.join(outputRoot, 'demo-content'), { recursive: true, force: true });
  await writeFile(path.join(outputRoot, 'index.html'), renderLandingHtml(manifest));
}

function renderLandingHtml(manifest) {
  const templateUrl = escapeHtml(manifest.publicUrl);
  const githubUrl = escapeHtml(GITHUB_REPO_URL);
  const ogImageUrl = escapeHtml(`${PUBLIC_BASE_URL}/fusion/og-image.png`);
  const updatedAt = escapeHtml(manifest.updatedAt);

  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Fusion Launcher - source-driven retro library launcher</title>
  <meta name="description" content="Fusion Launcher is a source-driven Windows launcher for retro game libraries on PC. Connect sources, install allowed content, set up emulators, and play from one desktop app.">
  <meta property="og:title" content="Fusion Launcher">
  <meta property="og:description" content="Source-driven Windows launcher for retro game libraries on PC.">
  <meta property="og:image" content="${ogImageUrl}">
  <meta name="theme-color" content="#070808">
  <style>
    :root {
      color-scheme: dark;
      --bg: #070808;
      --bg-soft: #0b0d0d;
      --panel: rgba(255, 255, 255, 0.035);
      --panel-strong: rgba(255, 255, 255, 0.055);
      --line: rgba(255, 255, 255, 0.10);
      --line-soft: rgba(255, 255, 255, 0.065);
      --text: #f6f7f2;
      --muted: rgba(236, 240, 235, 0.66);
      --soft: rgba(236, 240, 235, 0.42);
      --faint: rgba(236, 240, 235, 0.20);
      --accent: #69e68f;
      --accent-soft: rgba(105, 230, 143, 0.13);
      --accent-line: rgba(105, 230, 143, 0.28);
      --radius-lg: 28px;
      --radius-md: 18px;
      --radius-sm: 12px;
      --shadow-panel: 0 28px 90px rgba(0, 0, 0, 0.42);
      --mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", monospace;
      --sans: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }

    * { box-sizing: border-box; }
    html { scroll-behavior: smooth; }
    body {
      margin: 0;
      min-width: 320px;
      background: linear-gradient(180deg, #0a0b0b 0%, var(--bg) 52%, #050606 100%);
      color: var(--text);
      font-family: var(--sans);
      font-feature-settings: "cv01", "ss03";
      overflow-x: hidden;
    }
    ::selection { background: rgba(105, 230, 143, 0.24); }
    a { color: inherit; }
    code { font-family: var(--mono); }

    .page {
      width: min(1120px, calc(100% - 48px));
      margin: 0 auto;
    }
    .nav-wrap {
      position: sticky;
      top: 0;
      z-index: 20;
      border-bottom: 1px solid var(--line-soft);
      background: rgba(7, 8, 8, 0.78);
      backdrop-filter: blur(18px);
    }
    header {
      display: flex;
      min-height: 76px;
      align-items: center;
      justify-content: space-between;
      gap: 24px;
    }
    .brand {
      display: inline-flex;
      min-width: 0;
      align-items: center;
      gap: 12px;
      color: var(--text);
      text-decoration: none;
    }
    .brand-icon {
      width: 40px;
      height: 40px;
      border-radius: 13px;
      box-shadow: 0 0 0 1px rgba(255,255,255,0.16);
    }
    .brand span {
      font-size: 15px;
      font-weight: 620;
      letter-spacing: -0.025em;
      white-space: nowrap;
    }
    nav {
      display: flex;
      align-items: center;
      gap: 18px;
      color: var(--muted);
      font-size: 13px;
      font-weight: 520;
    }
    nav a {
      text-decoration: none;
      transition: color 160ms ease;
    }
    nav a:hover { color: var(--text); }
    .language-switch {
      display: inline-flex;
      align-items: center;
      gap: 2px;
      border: 1px solid var(--line-soft);
      border-radius: 999px;
      padding: 2px;
      background: rgba(255,255,255,0.025);
    }
    .language-switch button {
      border: 0;
      border-radius: 999px;
      background: transparent;
      color: var(--soft);
      cursor: pointer;
      font: inherit;
      padding: 5px 8px;
      transition: background 160ms ease, color 160ms ease;
    }
    .language-switch button[aria-pressed="true"] {
      background: rgba(255,255,255,0.09);
      color: var(--text);
    }

    .hero {
      display: grid;
      grid-template-columns: minmax(0, 0.88fr) minmax(360px, 1fr);
      gap: 72px;
      align-items: center;
      padding: 116px 0 78px;
    }
    .eyebrow {
      display: inline-flex;
      align-items: center;
      gap: 10px;
      color: var(--accent);
      font: 520 12px/1 var(--mono);
      letter-spacing: 0.12em;
      text-transform: uppercase;
    }
    .eyebrow::before {
      content: "";
      width: 7px;
      height: 7px;
      border-radius: 999px;
      background: var(--accent);
    }
    h1,
    h2,
    h3,
    p { margin: 0; }
    h1 {
      max-width: 680px;
      margin-top: 24px;
      color: var(--text);
      font-size: clamp(50px, 6.9vw, 88px);
      font-weight: 620;
      letter-spacing: -0.072em;
      line-height: 0.94;
      text-wrap: balance;
    }
    .accent-word { color: var(--accent); }
    .lead {
      max-width: 610px;
      margin-top: 26px;
      color: var(--muted);
      font-size: clamp(17px, 1.55vw, 20px);
      line-height: 1.68;
      letter-spacing: -0.018em;
      text-wrap: pretty;
    }
    .actions {
      display: flex;
      flex-wrap: wrap;
      gap: 12px;
      margin-top: 34px;
    }
    .button {
      display: inline-flex;
      min-height: 46px;
      align-items: center;
      justify-content: center;
      border-radius: var(--radius-sm);
      padding: 0 16px;
      font-size: 14px;
      font-weight: 560;
      letter-spacing: -0.012em;
      text-decoration: none;
      transition: background 160ms ease, border-color 160ms ease, color 160ms ease, transform 160ms ease;
    }
    .button:hover { transform: translateY(-1px); }
    .button.primary {
      border: 1px solid rgba(105, 230, 143, 0.44);
      background: var(--accent);
      color: #071109;
    }
    .button.primary:hover { background: #7df2a0; }
    .button.secondary {
      border: 1px solid var(--line);
      background: rgba(255,255,255,0.035);
      color: var(--text);
    }
    .button.secondary:hover {
      border-color: rgba(255,255,255,0.16);
      background: rgba(255,255,255,0.055);
    }
    .note {
      max-width: 610px;
      margin-top: 22px;
      color: var(--soft);
      font-size: 13px;
      line-height: 1.65;
    }
    .hero-facts {
      display: grid;
      grid-template-columns: repeat(4, minmax(0, 1fr));
      gap: 1px;
      margin-top: 38px;
      overflow: hidden;
      border: 1px solid var(--line-soft);
      border-radius: 18px;
      background: var(--line-soft);
    }
    .fact {
      min-height: 84px;
      background: rgba(255,255,255,0.026);
      padding: 16px;
    }
    .fact b {
      display: block;
      color: var(--text);
      font-size: 13px;
      font-weight: 560;
      letter-spacing: -0.01em;
    }
    .fact span {
      display: block;
      margin-top: 8px;
      color: var(--soft);
      font-size: 12px;
      line-height: 1.45;
    }

    .hero-panel {
      position: relative;
      border: 1px solid var(--line);
      border-radius: var(--radius-lg);
      background:
        linear-gradient(180deg, rgba(255,255,255,0.065), rgba(255,255,255,0.024)),
        rgba(7, 9, 9, 0.72);
      box-shadow: var(--shadow-panel), inset 0 1px 0 rgba(255,255,255,0.075);
      overflow: hidden;
      padding: 14px;
    }
    .window-bar,
    .product-frame { position: relative; z-index: 1; }
    .window-bar {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 18px;
      border-bottom: 1px solid var(--line-soft);
      padding: 8px 10px 14px;
      color: var(--soft);
      font: 500 12px/1 var(--mono);
      letter-spacing: 0.08em;
      text-transform: uppercase;
    }
    .traffic { display: flex; gap: 6px; }
    .traffic span {
      width: 8px;
      height: 8px;
      border-radius: 50%;
      background: rgba(255,255,255,0.24);
    }
    .traffic span:nth-child(3) { background: var(--accent); }
    .product-frame {
      display: grid;
      gap: 12px;
      padding-top: 14px;
    }
    .image-drop {
      position: relative;
      display: grid;
      place-items: center;
      min-height: 296px;
      overflow: hidden;
      border: 1px dashed rgba(255,255,255,0.20);
      border-radius: 22px;
      background: rgba(255,255,255,0.026);
      text-align: center;
    }
    .drop-image {
      position: absolute;
      inset: 0;
      width: 100%;
      height: 100%;
      border-radius: inherit;
      object-fit: cover;
      opacity: 0;
      transition: opacity 180ms ease;
    }
    .drop-copy {
      position: relative;
      z-index: 1;
      padding: 28px;
    }
    .image-drop.has-image {
      border-style: solid;
    }
    .image-drop.has-image .drop-image {
      opacity: 1;
    }
    .image-drop.has-image::before,
    .image-drop.has-image .drop-copy {
      opacity: 0;
      pointer-events: none;
    }
    .image-drop::before {
      content: attr(data-slot-label);
      position: absolute;
      left: 16px;
      top: 14px;
      color: var(--soft);
      font: 500 11px/1 var(--mono);
      letter-spacing: 0.14em;
      text-transform: uppercase;
    }
    .image-drop strong {
      display: block;
      color: var(--text);
      font-size: 19px;
      font-weight: 560;
      letter-spacing: -0.025em;
    }
    .image-drop span {
      display: block;
      max-width: 340px;
      margin: 10px auto 0;
      color: var(--soft);
      font-size: 13px;
      line-height: 1.55;
    }
    .image-drop code {
      display: inline-block;
      margin-top: 12px;
      color: rgba(205, 255, 220, 0.82);
      font-size: 12px;
    }
    .mini-grid {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 12px;
    }
    .mini-grid .image-drop { min-height: 154px; border-radius: 18px; }
    .mini-grid .image-drop strong { font-size: 15px; }
    .mini-grid .image-drop span { font-size: 12px; }

    .section {
      padding: 82px 0;
      border-top: 1px solid var(--line-soft);
    }
    .section-head {
      display: grid;
      grid-template-columns: minmax(0, 0.9fr) minmax(260px, 0.7fr);
      gap: 48px;
      align-items: end;
      margin-bottom: 28px;
    }
    .section-kicker {
      margin-bottom: 14px;
      color: var(--accent);
      font: 520 12px/1 var(--mono);
      letter-spacing: 0.12em;
      text-transform: uppercase;
    }
    h2 {
      max-width: 760px;
      color: var(--text);
      font-size: clamp(34px, 4.7vw, 58px);
      font-weight: 610;
      letter-spacing: -0.062em;
      line-height: 1;
      text-wrap: balance;
    }
    .section-copy {
      color: var(--muted);
      font-size: 15px;
      line-height: 1.72;
      text-wrap: pretty;
    }
    .flow-grid,
    .cards,
    .shot-grid {
      display: grid;
      gap: 12px;
    }
    .flow-grid { grid-template-columns: repeat(4, minmax(0, 1fr)); }
    .cards { grid-template-columns: repeat(3, minmax(0, 1fr)); }
    .shot-grid { grid-template-columns: 1.15fr 0.85fr; }
    .stack { display: grid; gap: 12px; }
    .step,
    .card,
    .legal-panel,
    .code-panel {
      border: 1px solid var(--line-soft);
      border-radius: var(--radius-md);
      background: var(--panel);
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.045);
    }
    .step {
      min-height: 174px;
      padding: 18px;
    }
    .step b {
      display: block;
      color: var(--accent);
      font: 520 12px/1 var(--mono);
      letter-spacing: 0.12em;
    }
    .step strong {
      display: block;
      margin-top: 54px;
      color: var(--text);
      font-size: 18px;
      font-weight: 570;
      letter-spacing: -0.025em;
    }
    .step span,
    .card p,
    .legal-panel p {
      color: var(--muted);
      line-height: 1.66;
    }
    .step span {
      display: block;
      margin-top: 8px;
      font-size: 13px;
    }
    .card {
      min-height: 240px;
      padding: 24px;
    }
    .card .kicker {
      color: var(--accent);
      font: 520 12px/1 var(--mono);
      letter-spacing: 0.12em;
      text-transform: uppercase;
    }
    .card h3 {
      margin-top: 74px;
      color: var(--text);
      font-size: 22px;
      font-weight: 590;
      letter-spacing: -0.04em;
      line-height: 1.08;
    }
    .card p {
      margin-top: 14px;
      font-size: 14px;
    }
    .shot-grid .image-drop:first-child { min-height: 440px; }
    .shot-grid .image-drop { min-height: 214px; }

    .legal {
      display: grid;
      grid-template-columns: minmax(0, 0.85fr) minmax(320px, 1.15fr);
      gap: 18px;
      align-items: stretch;
    }
    .legal-panel,
    .code-panel { padding: 28px; }
    .legal-panel h2 { font-size: clamp(34px, 4.2vw, 52px); }
    .legal-panel p { margin-top: 18px; font-size: 15px; }
    .code-panel {
      display: grid;
      align-content: center;
      gap: 14px;
      background: rgba(0,0,0,0.26);
    }
    .terminal-line {
      display: flex;
      min-width: 0;
      align-items: flex-start;
      gap: 10px;
      color: var(--soft);
      font: 500 13px/1.65 var(--mono);
    }
    .terminal-line::before {
      content: ">";
      flex: 0 0 auto;
      color: var(--accent);
    }
    .mono {
      color: rgba(205, 255, 220, 0.88);
      overflow-wrap: anywhere;
    }
    footer {
      display: flex;
      justify-content: space-between;
      gap: 18px;
      padding: 42px 0 64px;
      border-top: 1px solid var(--line-soft);
      color: var(--soft);
      font-size: 13px;
    }
    footer a { color: var(--muted); }

    @media (max-width: 1020px) {
      .hero,
      .section-head,
      .legal,
      .shot-grid { grid-template-columns: 1fr; }
      .hero { gap: 44px; padding-top: 78px; }
      .hero-facts,
      .flow-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
      .cards { grid-template-columns: 1fr; }
      .card { min-height: 210px; }
      .card h3 { margin-top: 56px; }
    }
    @media (max-width: 680px) {
      .page { width: min(100% - 28px, 1120px); }
      header { min-height: 68px; }
      .brand span { display: none; }
      nav { gap: 12px; font-size: 12px; }
      .hero { padding: 58px 0 58px; }
      h1 { font-size: clamp(44px, 14vw, 58px); }
      .lead { font-size: 16px; }
      .hero-facts,
      .flow-grid,
      .mini-grid { grid-template-columns: 1fr; }
      .hero-panel { border-radius: 22px; padding: 10px; }
      .image-drop { min-height: 250px; }
      .section { padding: 58px 0; }
      footer { flex-direction: column; }
    }
  </style>
</head>
<body>
  <div class="nav-wrap">
    <div class="page">
      <header>
        <a class="brand" href="./" aria-label="Fusion Launcher home">
          <img class="brand-icon" src="./fusion/app-icon.png" alt="">
          <span>fusion launcher</span>
        </a>
        <nav aria-label="Primary">
          <a href="#screenshots" data-i18n="nav.images">Image slots</a>
          <a href="${templateUrl}" data-i18n="nav.template">Source template</a>
          <a href="${githubUrl}">GitHub</a>
          <span class="language-switch" aria-label="Language">
            <button type="button" data-lang="en" aria-pressed="true">EN</button>
            <button type="button" data-lang="ru" aria-pressed="false">RU</button>
          </span>
        </nav>
      </header>
    </div>
  </div>

  <main class="page">
    <section class="hero">
      <div class="hero-copy">
        <div class="eyebrow" data-i18n="hero.eyebrow">Local-first Windows launcher</div>
        <h1 data-i18n-html="hero.title">Retro game libraries on PC. <span class="accent-word">Clean source flow.</span></h1>
        <p class="lead" data-i18n="hero.lead">Fusion Launcher connects source-library catalogs, tracks allowed install state, checks emulator requirements, and launches from one quiet desktop shell.</p>
        <div class="actions">
          <a class="button primary" href="${githubUrl}" data-i18n="cta.github">View on GitHub</a>
          <a class="button secondary" href="${templateUrl}" data-i18n="cta.template">Open source-library template</a>
        </div>
        <p class="note" data-i18n="hero.note">Content-neutral by design: Fusion Launcher does not host or distribute commercial games, BIOS files, firmware, keys, or third-party payloads.</p>
        <div class="hero-facts" aria-label="Product traits" data-i18n-aria="aria.traits">
          <div class="fact"><b data-i18n="fact.sources.title">Source-driven</b><span data-i18n="fact.sources.text">Catalogs stay user-owned and portable.</span></div>
          <div class="fact"><b data-i18n="fact.windows.title">Windows-first</b><span data-i18n="fact.windows.text">Tauri desktop shell, local storage.</span></div>
          <div class="fact"><b data-i18n="fact.preflight.title">Preflight</b><span data-i18n="fact.preflight.text">Setup checks before launch.</span></div>
          <div class="fact"><b data-i18n="fact.screens.title">Screenshot-ready</b><span data-i18n="fact.screens.text">Clean frames for real product captures.</span></div>
        </div>
      </div>

      <div class="hero-panel" aria-label="Fusion Launcher screenshot placeholder" data-i18n-aria="aria.heroPanel">
        <div class="window-bar"><span>Fusion Launcher</span><div class="traffic" aria-hidden="true"><span></span><span></span><span></span></div></div>
        <div class="product-frame">
          <div class="image-drop" data-slot-label="image slot" data-slot-label-key="slot.label" data-auto-src="./fusion/screens/hero-library.png" data-auto-alt="Fusion Launcher library screenshot" data-i18n-auto-alt="slot.hero.alt">
            <div class="drop-copy">
              <strong data-i18n="slot.hero.title">Drop your primary app screenshot here.</strong>
              <span data-i18n="slot.hero.text">Best fit: a dark 16:10 library or game-detail screenshot with real covers and the Fusion sidebar.</span>
              <code>public/fusion/screens/hero-library.png</code>
            </div>
          </div>
          <div class="mini-grid">
            <div class="image-drop" data-slot-label="image slot" data-slot-label-key="slot.label" data-auto-src="./fusion/screens/setup-modal.png" data-auto-alt="Fusion Launcher setup modal screenshot" data-i18n-auto-alt="slot.setup.alt">
              <div class="drop-copy"><strong data-i18n="slot.setup.title">Setup modal</strong><span data-i18n="slot.setup.text">Emulator path / preflight checks.</span></div>
            </div>
            <div class="image-drop" data-slot-label="image slot" data-slot-label-key="slot.label" data-auto-src="./fusion/screens/game-detail.png" data-auto-alt="Fusion Launcher game detail screenshot" data-i18n-auto-alt="slot.detail.alt">
              <div class="drop-copy"><strong data-i18n="slot.detail.title">Game detail</strong><span data-i18n="slot.detail.text">Artwork, source, install status.</span></div>
            </div>
          </div>
        </div>
      </div>
    </section>

    <section class="section">
      <div class="section-head">
        <div>
          <div class="section-kicker" data-i18n="flow.kicker">Flow</div>
          <h2 data-i18n="flow.title">Connect source. Verify setup. Launch locally.</h2>
        </div>
        <p class="section-copy" data-i18n="flow.copy">Source libraries describe games, metadata, artwork, setup profiles, and install requirements. Fusion keeps each step explicit before it launches anything.</p>
      </div>
      <div class="flow-grid">
        <div class="step"><b>01</b><strong data-i18n="flow.connect.title">Connect</strong><span data-i18n="flow.connect.text">Paste or import a source-library URL.</span></div>
        <div class="step"><b>02</b><strong data-i18n="flow.browse.title">Browse</strong><span data-i18n="flow.browse.text">Open a local catalog with metadata and artwork.</span></div>
        <div class="step"><b>03</b><strong data-i18n="flow.preflight.title">Preflight</strong><span data-i18n="flow.preflight.text">Check emulator paths and required user-provided files.</span></div>
        <div class="step"><b>04</b><strong data-i18n="flow.launch.title">Launch</strong><span data-i18n="flow.launch.text">Start games after readiness checks pass.</span></div>
      </div>
    </section>

    <section class="section" id="screenshots">
      <div class="section-head">
        <div>
          <div class="section-kicker" data-i18n="screens.kicker">Image slots</div>
          <h2 data-i18n="screens.title">Add real product screenshots when they are ready.</h2>
        </div>
        <p class="section-copy" data-i18n-html="screens.copy">Use these slots for your own product images later. Keep filenames under <code>public/fusion/screens/</code>; the Pages build copies that folder automatically.</p>
      </div>
      <div class="shot-grid">
        <div class="image-drop" data-slot-label="image slot" data-slot-label-key="slot.label" data-auto-src="./fusion/screens/library.png" data-auto-alt="Fusion Launcher library view screenshot" data-i18n-auto-alt="slot.library.alt">
          <div class="drop-copy">
            <strong data-i18n="slot.library.title">Library screenshot</strong>
            <span data-i18n="slot.library.text">Main browsing view with covers, source state, and sidebar.</span>
            <code>public/fusion/screens/library.png</code>
          </div>
        </div>
        <div class="stack">
          <div class="image-drop" data-slot-label="image slot" data-slot-label-key="slot.label" data-auto-src="./fusion/screens/source-setup.png" data-auto-alt="Fusion Launcher source setup screenshot" data-i18n-auto-alt="slot.sourceSetup.alt">
            <div class="drop-copy">
              <strong data-i18n="slot.sourceSetup.title">Source setup</strong>
              <span data-i18n="slot.sourceSetup.text">URL import, manifest details, trust notes.</span>
              <code>public/fusion/screens/source-setup.png</code>
            </div>
          </div>
          <div class="image-drop" data-slot-label="image slot" data-slot-label-key="slot.label" data-auto-src="./fusion/screens/preflight.png" data-auto-alt="Fusion Launcher preflight result screenshot" data-i18n-auto-alt="slot.preflight.alt">
            <div class="drop-copy">
              <strong data-i18n="slot.preflight.title">Preflight result</strong>
              <span data-i18n="slot.preflight.text">Emulator profile and missing-file checks.</span>
              <code>public/fusion/screens/preflight.png</code>
            </div>
          </div>
        </div>
      </div>
    </section>

    <section class="section">
      <div class="section-head">
        <div>
          <div class="section-kicker" data-i18n="position.kicker">Positioning</div>
          <h2 data-i18n="position.title">A launcher layer for source-based game libraries.</h2>
        </div>
        <p class="section-copy" data-i18n="position.copy">Clear source model, setup help, and Windows desktop shell without implying bundled commercial content.</p>
      </div>
      <div class="cards">
        <article class="card"><div class="kicker" data-i18n="card.sources.kicker">Sources</div><h3 data-i18n="card.sources.title">Catalog flow without bundled payloads</h3><p data-i18n="card.sources.text">Fusion connects user-selected source libraries and turns them into a browsable launcher catalog with local download state.</p></article>
        <article class="card"><div class="kicker" data-i18n="card.setup.kicker">Setup</div><h3 data-i18n="card.setup.title">Guided emulator readiness</h3><p data-i18n="card.setup.text">Supported platforms get portable setup help, local paths, system-file checks, and launch diagnostics.</p></article>
        <article class="card"><div class="kicker" data-i18n="card.desktop.kicker">Desktop</div><h3 data-i18n="card.desktop.title">Fast Windows app shell</h3><p data-i18n="card.desktop.text">Built with Tauri, Rust, Next.js, and local-first storage for a responsive PC launcher experience.</p></article>
      </div>
    </section>

    <section class="section legal" id="content-model">
      <div class="legal-panel">
        <h2 data-i18n="legal.title">Content-neutral by design.</h2>
        <p data-i18n="legal.p1">Fusion Launcher provides source connection, download tracking, metadata enrichment, emulator setup assistance, and launch orchestration.</p>
        <p data-i18n="legal.p2">The official project does not ship commercial ROMs, BIOS files, firmware, keys, or third-party game payloads. Users choose their sources and are responsible for the content they connect.</p>
      </div>
      <div class="code-panel">
        <div class="terminal-line" data-i18n="terminal.starter">starter source template</div>
        <div class="terminal-line mono">${templateUrl}</div>
        <div class="terminal-line"><span data-i18n="terminal.updated">template updated</span>&nbsp;${updatedAt}</div>
        <div class="terminal-line" data-i18n="terminal.affiliation">not affiliated with emulator projects, publishers, or console manufacturers</div>
      </div>
    </section>
  </main>

  <footer class="page">
    <span>Fusion Launcher</span>
    <span><a href="${githubUrl}">GitHub</a> · <a href="${templateUrl}" data-i18n="nav.template">Source template</a></span>
  </footer>
  <script>
    (() => {
      const dictionaries = {
        en: {
          'meta.title': 'Fusion Launcher - source-driven retro library launcher',
          'meta.description': 'Fusion Launcher is a source-driven Windows launcher for retro game libraries on PC. Connect sources, install allowed content, set up emulators, and play from one desktop app.',
          'nav.images': 'Image slots',
          'nav.template': 'Source template',
          'hero.eyebrow': 'Local-first Windows launcher',
          'hero.title': 'Retro game libraries on PC. <span class="accent-word">Clean source flow.</span>',
          'hero.lead': 'Fusion Launcher connects source-library catalogs, tracks allowed install state, checks emulator requirements, and launches from one quiet desktop shell.',
          'cta.github': 'View on GitHub',
          'cta.template': 'Open source-library template',
          'hero.note': 'Content-neutral by design: Fusion Launcher does not host or distribute commercial games, BIOS files, firmware, keys, or third-party payloads.',
          'aria.traits': 'Product traits',
          'fact.sources.title': 'Source-driven',
          'fact.sources.text': 'Catalogs stay user-owned and portable.',
          'fact.windows.title': 'Windows-first',
          'fact.windows.text': 'Tauri desktop shell, local storage.',
          'fact.preflight.title': 'Preflight',
          'fact.preflight.text': 'Setup checks before launch.',
          'fact.screens.title': 'Screenshot-ready',
          'fact.screens.text': 'Clean frames for real product captures.',
          'aria.heroPanel': 'Fusion Launcher screenshot placeholder',
          'slot.label': 'image slot',
          'slot.hero.alt': 'Fusion Launcher library screenshot',
          'slot.hero.title': 'Drop your primary app screenshot here.',
          'slot.hero.text': 'Best fit: a dark 16:10 library or game-detail screenshot with real covers and the Fusion sidebar.',
          'slot.setup.alt': 'Fusion Launcher setup modal screenshot',
          'slot.setup.title': 'Setup modal',
          'slot.setup.text': 'Emulator path / preflight checks.',
          'slot.detail.alt': 'Fusion Launcher game detail screenshot',
          'slot.detail.title': 'Game detail',
          'slot.detail.text': 'Artwork, source, install status.',
          'flow.kicker': 'Flow',
          'flow.title': 'Connect source. Verify setup. Launch locally.',
          'flow.copy': 'Source libraries describe games, metadata, artwork, setup profiles, and install requirements. Fusion keeps each step explicit before it launches anything.',
          'flow.connect.title': 'Connect',
          'flow.connect.text': 'Paste or import a source-library URL.',
          'flow.browse.title': 'Browse',
          'flow.browse.text': 'Open a local catalog with metadata and artwork.',
          'flow.preflight.title': 'Preflight',
          'flow.preflight.text': 'Check emulator paths and required user-provided files.',
          'flow.launch.title': 'Launch',
          'flow.launch.text': 'Start games after readiness checks pass.',
          'screens.kicker': 'Image slots',
          'screens.title': 'Add real product screenshots when they are ready.',
          'screens.copy': 'Use these slots for your own product images later. Keep filenames under <code>public/fusion/screens/</code>; the Pages build copies that folder automatically.',
          'slot.library.alt': 'Fusion Launcher library view screenshot',
          'slot.library.title': 'Library screenshot',
          'slot.library.text': 'Main browsing view with covers, source state, and sidebar.',
          'slot.sourceSetup.alt': 'Fusion Launcher source setup screenshot',
          'slot.sourceSetup.title': 'Source setup',
          'slot.sourceSetup.text': 'URL import, manifest details, trust notes.',
          'slot.preflight.alt': 'Fusion Launcher preflight result screenshot',
          'slot.preflight.title': 'Preflight result',
          'slot.preflight.text': 'Emulator profile and missing-file checks.',
          'position.kicker': 'Positioning',
          'position.title': 'A launcher layer for source-based game libraries.',
          'position.copy': 'Clear source model, setup help, and Windows desktop shell without implying bundled commercial content.',
          'card.sources.kicker': 'Sources',
          'card.sources.title': 'Catalog flow without bundled payloads',
          'card.sources.text': 'Fusion connects user-selected source libraries and turns them into a browsable launcher catalog with local download state.',
          'card.setup.kicker': 'Setup',
          'card.setup.title': 'Guided emulator readiness',
          'card.setup.text': 'Supported platforms get portable setup help, local paths, system-file checks, and launch diagnostics.',
          'card.desktop.kicker': 'Desktop',
          'card.desktop.title': 'Fast Windows app shell',
          'card.desktop.text': 'Built with Tauri, Rust, Next.js, and local-first storage for a responsive PC launcher experience.',
          'legal.title': 'Content-neutral by design.',
          'legal.p1': 'Fusion Launcher provides source connection, download tracking, metadata enrichment, emulator setup assistance, and launch orchestration.',
          'legal.p2': 'The official project does not ship commercial ROMs, BIOS files, firmware, keys, or third-party game payloads. Users choose their sources and are responsible for the content they connect.',
          'terminal.starter': 'starter source template',
          'terminal.updated': 'template updated',
          'terminal.affiliation': 'not affiliated with emulator projects, publishers, or console manufacturers'
        },
        ru: {
          'meta.title': 'Fusion Launcher — лаунчер ретро-библиотек из источников',
          'meta.description': 'Fusion Launcher — Windows-лаунчер для ретро-библиотек на ПК. Подключайте источники, устанавливайте разрешённый контент, настраивайте эмуляторы и запускайте игры из одного приложения.',
          'nav.images': 'Слоты для изображений',
          'nav.template': 'Шаблон источника',
          'hero.eyebrow': 'Локальный Windows-лаунчер',
          'hero.title': 'Ретро-библиотеки на ПК. <span class="accent-word">Чистый поток источников.</span>',
          'hero.lead': 'Fusion Launcher подключает каталоги источников, отслеживает разрешённую установку, проверяет требования эмуляторов и запускает всё из спокойной desktop-оболочки.',
          'cta.github': 'Открыть GitHub',
          'cta.template': 'Открыть шаблон источника',
          'hero.note': 'Контент-нейтрально по дизайну: Fusion Launcher не хостит и не распространяет коммерческие игры, BIOS, firmware, ключи или сторонние payload-файлы.',
          'aria.traits': 'Свойства продукта',
          'fact.sources.title': 'От источников',
          'fact.sources.text': 'Каталоги остаются переносимыми и под контролем пользователя.',
          'fact.windows.title': 'Windows-first',
          'fact.windows.text': 'Tauri-оболочка и локальное хранение.',
          'fact.preflight.title': 'Preflight',
          'fact.preflight.text': 'Проверки настройки перед запуском.',
          'fact.screens.title': 'Готово под скриншоты',
          'fact.screens.text': 'Чистые рамки для реальных кадров продукта.',
          'aria.heroPanel': 'Плейсхолдер скриншота Fusion Launcher',
          'slot.label': 'слот изображения',
          'slot.hero.alt': 'Скриншот библиотеки Fusion Launcher',
          'slot.hero.title': 'Добавь сюда главный скриншот приложения.',
          'slot.hero.text': 'Лучше всего подойдёт тёмный 16:10 скрин библиотеки или страницы игры с реальными обложками и сайдбаром Fusion.',
          'slot.setup.alt': 'Скриншот окна настройки Fusion Launcher',
          'slot.setup.title': 'Окно настройки',
          'slot.setup.text': 'Путь к эмулятору / preflight-проверки.',
          'slot.detail.alt': 'Скриншот страницы игры Fusion Launcher',
          'slot.detail.title': 'Страница игры',
          'slot.detail.text': 'Арт, источник, статус установки.',
          'flow.kicker': 'Поток',
          'flow.title': 'Подключи источник. Проверь настройку. Запусти локально.',
          'flow.copy': 'Source libraries описывают игры, метаданные, арт, профили настройки и требования установки. Fusion явно показывает каждый шаг перед запуском.',
          'flow.connect.title': 'Подключение',
          'flow.connect.text': 'Вставь или импортируй URL source-library.',
          'flow.browse.title': 'Каталог',
          'flow.browse.text': 'Открой локальный каталог с метаданными и артом.',
          'flow.preflight.title': 'Preflight',
          'flow.preflight.text': 'Проверь пути эмуляторов и пользовательские системные файлы.',
          'flow.launch.title': 'Запуск',
          'flow.launch.text': 'Запускай игры только после успешных проверок.',
          'screens.kicker': 'Слоты изображений',
          'screens.title': 'Добавь реальные скриншоты продукта, когда они будут готовы.',
          'screens.copy': 'Эти места оставлены под твои изображения. Храни файлы в <code>public/fusion/screens/</code>; Pages-сборка скопирует эту папку автоматически.',
          'slot.library.alt': 'Скриншот библиотеки Fusion Launcher',
          'slot.library.title': 'Скриншот библиотеки',
          'slot.library.text': 'Основной экран с обложками, состоянием источника и сайдбаром.',
          'slot.sourceSetup.alt': 'Скриншот настройки источника Fusion Launcher',
          'slot.sourceSetup.title': 'Настройка источника',
          'slot.sourceSetup.text': 'Импорт URL, детали manifest, trust-заметки.',
          'slot.preflight.alt': 'Скриншот результата preflight Fusion Launcher',
          'slot.preflight.title': 'Результат preflight',
          'slot.preflight.text': 'Профиль эмулятора и проверки недостающих файлов.',
          'position.kicker': 'Позиционирование',
          'position.title': 'Лаунчерный слой для игровых библиотек из источников.',
          'position.copy': 'Понятная модель источников, помощь с настройкой и Windows desktop-оболочка без намёка на поставку коммерческого контента.',
          'card.sources.kicker': 'Источники',
          'card.sources.title': 'Каталоги без встроенных payload-файлов',
          'card.sources.text': 'Fusion подключает выбранные пользователем source libraries и превращает их в просматриваемый каталог с локальным состоянием загрузок.',
          'card.setup.kicker': 'Настройка',
          'card.setup.title': 'Готовность эмулятора',
          'card.setup.text': 'Для поддерживаемых платформ есть помощь с portable-настройкой, локальные пути, проверки системных файлов и диагностика запуска.',
          'card.desktop.kicker': 'Desktop',
          'card.desktop.title': 'Быстрая Windows-оболочка',
          'card.desktop.text': 'Tauri, Rust, Next.js и local-first хранение для отзывчивого ПК-лаунчера.',
          'legal.title': 'Контент-нейтрально по дизайну.',
          'legal.p1': 'Fusion Launcher подключает источники, отслеживает загрузки, обогащает метаданные, помогает с настройкой эмуляторов и оркестрирует запуск.',
          'legal.p2': 'Официальный проект не поставляет коммерческие ROM, BIOS, firmware, ключи или сторонние игровые payload-файлы. Пользователь сам выбирает источники и отвечает за подключаемый контент.',
          'terminal.starter': 'стартовый шаблон источника',
          'terminal.updated': 'шаблон обновлён',
          'terminal.affiliation': 'не связан с проектами эмуляторов, издателями или производителями консолей'
        }
      };

      const isSupported = (locale) => Object.prototype.hasOwnProperty.call(dictionaries, locale);
      const getStoredLocale = () => {
        try { return window.localStorage.getItem('fusionLandingLang'); } catch { return null; }
      };
      const storeLocale = (locale) => {
        try { window.localStorage.setItem('fusionLandingLang', locale); } catch {}
      };
      const params = new URLSearchParams(window.location.search);
      const requested = params.get('lang');
      const stored = getStoredLocale();
      const browserLocale = (navigator.language || '').toLowerCase().startsWith('ru') ? 'ru' : 'en';
      const initialLocale = isSupported(requested) ? requested : (isSupported(stored) ? stored : browserLocale);

      const setLanguage = (locale) => {
        const nextLocale = isSupported(locale) ? locale : 'en';
        const dictionary = dictionaries[nextLocale];
        document.documentElement.lang = nextLocale;
        document.title = dictionary['meta.title'];
        const description = document.querySelector('meta[name="description"]');
        if (description) description.setAttribute('content', dictionary['meta.description']);
        const ogDescription = document.querySelector('meta[property="og:description"]');
        if (ogDescription) ogDescription.setAttribute('content', dictionary['meta.description']);

        document.querySelectorAll('[data-i18n]').forEach((element) => {
          const key = element.getAttribute('data-i18n');
          if (key && dictionary[key]) element.textContent = dictionary[key];
        });
        document.querySelectorAll('[data-i18n-html]').forEach((element) => {
          const key = element.getAttribute('data-i18n-html');
          if (key && dictionary[key]) element.innerHTML = dictionary[key];
        });
        document.querySelectorAll('[data-i18n-aria]').forEach((element) => {
          const key = element.getAttribute('data-i18n-aria');
          if (key && dictionary[key]) element.setAttribute('aria-label', dictionary[key]);
        });
        document.querySelectorAll('[data-slot-label-key]').forEach((element) => {
          const key = element.getAttribute('data-slot-label-key');
          if (key && dictionary[key]) element.setAttribute('data-slot-label', dictionary[key]);
        });
        document.querySelectorAll('[data-i18n-auto-alt]').forEach((element) => {
          const key = element.getAttribute('data-i18n-auto-alt');
          if (key && dictionary[key]) element.setAttribute('data-auto-alt', dictionary[key]);
        });
        document.querySelectorAll('[data-lang]').forEach((button) => {
          button.setAttribute('aria-pressed', String(button.getAttribute('data-lang') === nextLocale));
        });
        storeLocale(nextLocale);
      };

      document.querySelectorAll('[data-lang]').forEach((button) => {
        button.addEventListener('click', () => setLanguage(button.getAttribute('data-lang')));
      });
      setLanguage(initialLocale);
    })();
  </script>
  <script>
    (() => {
      if (window.location.protocol === 'file:') return;
      document.querySelectorAll('.image-drop[data-auto-src]').forEach(async (slot) => {
        const src = slot.getAttribute('data-auto-src');
        if (!src) return;
        try {
          const response = await fetch(src, { method: 'HEAD', cache: 'no-store' });
          if (!response.ok) return;
          const image = new Image();
          image.className = 'drop-image';
          image.alt = slot.getAttribute('data-auto-alt') || '';
          image.loading = 'lazy';
          image.onload = () => slot.classList.add('has-image');
          image.onerror = () => image.remove();
          image.src = src;
          slot.prepend(image);
        } catch {
          // Keep the text placeholder when the optional screenshot is absent.
        }
      });
    })();
  </script>
</body>
</html>
`;
}

function assertArtifact(artifact) {
  if (artifact.manifest.publicUrl !== PUBLIC_TEMPLATE_URL) {
    throw new Error(`Unexpected public URL: ${artifact.manifest.publicUrl}`);
  }
  if (artifact.manifest.templateId !== 'fusion-launcher-source-template') {
    throw new Error(`Unexpected template id: ${artifact.manifest.templateId}`);
  }
  if (artifact.manifest.trustLevel !== 'community') {
    throw new Error(`Template trustLevel must stay community, got ${artifact.manifest.trustLevel}`);
  }
  if (artifact.manifest.catalogCount !== artifact.repository.catalog.length) {
    throw new Error('Manifest catalogCount does not match repository.json');
  }
  if (artifact.manifest.systemFileCount !== artifact.repository.system_files.length) {
    throw new Error('Manifest systemFileCount does not match repository.json');
  }
}

function renderIndexHtml(manifest) {
  const escapedName = escapeHtml(manifest.name);
  const escapedUrl = escapeHtml(manifest.publicUrl);
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${escapedName}</title>
  <style>
    body { background: #050507; color: #f5f5f5; font-family: system-ui, sans-serif; margin: 0; padding: 32px; }
    main { max-width: 760px; }
    a { color: #9ee7ff; }
    code { background: rgba(255,255,255,0.08); border: 1px solid rgba(255,255,255,0.12); padding: 2px 6px; }
  </style>
</head>
<body>
  <main>
    <h1>${escapedName}</h1>
    <p>Paste this source URL into Fusion Launcher Settings &gt; Sources:</p>
    <p><code>${escapedUrl}</code></p>
    <p><a href="./repository.json">repository.json</a> | <a href="./README.md">README.md</a> | <a href="./manifest.json">manifest.json</a></p>
  </main>
</body>
</html>
`;
}

function escapeHtml(value) {
  return String(value)
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;');
}

await main();
