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
  <meta name="theme-color" content="#050707">
  <style>
    :root {
      color-scheme: dark;
      --bg: #050707;
      --bg-2: #080b0b;
      --panel: rgba(15, 18, 18, 0.76);
      --panel-strong: rgba(20, 24, 24, 0.92);
      --surface: rgba(255, 255, 255, 0.045);
      --surface-strong: rgba(255, 255, 255, 0.075);
      --line: rgba(255, 255, 255, 0.11);
      --line-soft: rgba(255, 255, 255, 0.065);
      --text: #f7f8f3;
      --muted: rgba(232, 238, 232, 0.68);
      --soft: rgba(232, 238, 232, 0.44);
      --faint: rgba(232, 238, 232, 0.22);
      --accent: #5cff91;
      --accent-2: #3bd6c6;
      --accent-3: #d7ff72;
      --accent-on: #041509;
      --danger: #ff6b6b;
      --radius-xl: 32px;
      --radius-lg: 24px;
      --radius-md: 16px;
      --shadow-deep: 0 34px 120px rgba(0, 0, 0, 0.62);
      --shadow-glow: 0 0 80px rgba(92, 255, 145, 0.18);
    }
    * { box-sizing: border-box; }
    html { scroll-behavior: smooth; }
    body {
      margin: 0;
      min-width: 320px;
      background:
        radial-gradient(circle at 56% 13%, rgba(92, 255, 145, 0.16), transparent 30rem),
        radial-gradient(circle at 8% 72%, rgba(59, 214, 198, 0.12), transparent 26rem),
        linear-gradient(180deg, #06100b 0%, var(--bg) 34%, #050505 100%);
      color: var(--text);
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      font-feature-settings: "cv01", "ss03";
      overflow-x: hidden;
    }
    body::before {
      content: '';
      position: fixed;
      inset: 0;
      z-index: -3;
      background-image:
        linear-gradient(rgba(255,255,255,0.03) 1px, transparent 1px),
        linear-gradient(90deg, rgba(255,255,255,0.03) 1px, transparent 1px);
      background-size: 64px 64px;
      mask-image: radial-gradient(circle at 50% 0%, black 0%, transparent 72%);
      pointer-events: none;
    }
    body::after {
      content: '';
      position: fixed;
      inset: 0;
      z-index: -2;
      background:
        linear-gradient(90deg, rgba(255,255,255,0.018), transparent 18%, transparent 82%, rgba(255,255,255,0.018)),
        repeating-linear-gradient(180deg, rgba(255,255,255,0.018) 0 1px, transparent 1px 5px);
      opacity: 0.32;
      pointer-events: none;
    }
    a { color: inherit; }
    .page { width: min(1220px, calc(100% - 40px)); margin: 0 auto; }
    .nav-wrap {
      position: sticky;
      top: 0;
      z-index: 30;
      padding: 18px 0 8px;
      backdrop-filter: blur(18px);
    }
    header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 24px;
      min-height: 66px;
      border: 1px solid rgba(255,255,255,0.075);
      border-radius: 999px;
      background: linear-gradient(180deg, rgba(14,18,18,0.76), rgba(8,10,10,0.48));
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.08), 0 18px 55px rgba(0,0,0,0.25);
      padding: 10px 12px 10px 14px;
    }
    .brand { display: inline-flex; align-items: center; gap: 12px; min-width: 0; text-decoration: none; font-weight: 840; letter-spacing: -0.035em; }
    .brand img { width: 42px; height: 42px; border-radius: 14px; box-shadow: 0 0 0 1px rgba(255,255,255,0.16), 0 12px 34px rgba(92, 255, 145, 0.22); }
    .brand span { white-space: nowrap; }
    nav { display: flex; align-items: center; gap: 6px; color: var(--muted); font-size: 14px; font-weight: 620; }
    nav a { border-radius: 999px; padding: 10px 13px; text-decoration: none; transition: background .18s ease, color .18s ease, transform .18s ease; }
    nav a:hover { background: rgba(255,255,255,0.07); color: var(--text); transform: translateY(-1px); }
    .hero {
      position: relative;
      display: grid;
      grid-template-columns: minmax(0, 0.98fr) minmax(420px, 1.02fr);
      gap: 54px;
      align-items: center;
      min-height: calc(100vh - 104px);
      padding: 76px 0 94px;
    }
    .hero::before {
      content: '';
      position: absolute;
      left: -14%;
      top: 12%;
      width: 440px;
      height: 440px;
      border-radius: 999px;
      background: radial-gradient(circle, rgba(92,255,145,0.22), transparent 64%);
      filter: blur(24px);
      opacity: 0.62;
      pointer-events: none;
    }
    .hero-copy { position: relative; z-index: 2; }
    .eyebrow {
      display: inline-flex;
      align-items: center;
      gap: 10px;
      border: 1px solid rgba(92, 255, 145, 0.36);
      background: linear-gradient(180deg, rgba(92,255,145,0.13), rgba(92,255,145,0.055));
      color: #caffd8;
      border-radius: 999px;
      padding: 9px 13px;
      font-size: 13px;
      font-weight: 780;
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.1), 0 0 42px rgba(92,255,145,0.13);
    }
    .eyebrow::before {
      content: '';
      width: 8px;
      height: 8px;
      border-radius: 999px;
      background: var(--accent);
      box-shadow: 0 0 20px rgba(92, 255, 145, 0.9);
    }
    h1 {
      margin: 24px 0 20px;
      max-width: 780px;
      font-size: clamp(58px, 7.3vw, 106px);
      line-height: 0.89;
      letter-spacing: -0.082em;
      text-wrap: balance;
    }
    .gradient-word {
      color: transparent;
      background: linear-gradient(90deg, #f7f8f3 0%, #baffcd 42%, #69ffd5 100%);
      -webkit-background-clip: text;
      background-clip: text;
      text-shadow: 0 0 48px rgba(92,255,145,0.08);
    }
    .lead {
      max-width: 660px;
      color: var(--muted);
      font-size: clamp(18px, 1.7vw, 22px);
      line-height: 1.62;
      letter-spacing: -0.018em;
      text-wrap: pretty;
    }
    .actions { display: flex; flex-wrap: wrap; gap: 12px; margin: 36px 0 20px; }
    .button {
      position: relative;
      display: inline-flex;
      min-height: 52px;
      align-items: center;
      justify-content: center;
      gap: 10px;
      border-radius: 999px;
      padding: 0 22px;
      text-decoration: none;
      font-weight: 850;
      letter-spacing: -0.012em;
      transition: transform .2s ease, box-shadow .2s ease, background .2s ease;
    }
    .button.primary {
      background: linear-gradient(180deg, #79ff9d, #43e978);
      color: var(--accent-on);
      box-shadow: 0 18px 58px rgba(92, 255, 145, 0.24), inset 0 1px 0 rgba(255,255,255,0.42);
    }
    .button.primary:hover { transform: translateY(-2px); box-shadow: 0 24px 70px rgba(92, 255, 145, 0.32), inset 0 1px 0 rgba(255,255,255,0.5); }
    .button.secondary {
      border: 1px solid rgba(255,255,255,0.13);
      background: linear-gradient(180deg, rgba(255,255,255,0.075), rgba(255,255,255,0.025));
      color: var(--text);
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.08);
    }
    .button.secondary:hover { transform: translateY(-2px); background: rgba(255,255,255,0.09); }
    .note {
      max-width: 650px;
      color: var(--soft);
      font-size: 13px;
      line-height: 1.72;
    }
    .hero-tags { display: flex; flex-wrap: wrap; gap: 10px; margin-top: 28px; }
    .hero-tags span {
      display: inline-flex;
      align-items: center;
      gap: 8px;
      border: 1px solid rgba(255,255,255,0.09);
      border-radius: 999px;
      background: rgba(255,255,255,0.045);
      color: rgba(247,248,243,0.78);
      padding: 8px 10px;
      font-size: 12px;
      font-weight: 720;
    }
    .hero-tags span::before { content: ''; width: 6px; height: 6px; border-radius: 999px; background: var(--accent-2); box-shadow: 0 0 14px rgba(59,214,198,0.65); }
    .showcase {
      position: relative;
      min-height: 620px;
      perspective: 1400px;
    }
    .halo {
      position: absolute;
      inset: 8% -10% auto 4%;
      height: 430px;
      border-radius: 999px;
      background: radial-gradient(circle, rgba(92,255,145,0.23), rgba(59,214,198,0.1) 44%, transparent 68%);
      filter: blur(34px);
      opacity: .76;
      pointer-events: none;
    }
    .launcher-window {
      position: relative;
      z-index: 2;
      display: grid;
      grid-template-columns: 90px minmax(0, 1fr);
      width: min(100%, 640px);
      min-height: 560px;
      margin-left: auto;
      border: 1px solid rgba(255,255,255,0.115);
      border-radius: 34px;
      background:
        linear-gradient(180deg, rgba(255,255,255,0.085), rgba(255,255,255,0.026)),
        radial-gradient(circle at 80% 10%, rgba(92,255,145,0.16), transparent 28rem),
        rgba(9, 11, 11, 0.84);
      box-shadow: var(--shadow-deep), inset 0 1px 0 rgba(255,255,255,0.11), inset 0 -1px 0 rgba(0,0,0,0.45);
      overflow: hidden;
      transform: rotateX(2deg) rotateY(-4deg);
    }
    .launcher-window::before {
      content: '';
      position: absolute;
      inset: 0;
      background-image: linear-gradient(rgba(255,255,255,0.035) 1px, transparent 1px), linear-gradient(90deg, rgba(255,255,255,0.028) 1px, transparent 1px);
      background-size: 42px 42px;
      mask-image: radial-gradient(circle at 55% 35%, black, transparent 72%);
      pointer-events: none;
    }
    .sidebar {
      position: relative;
      z-index: 2;
      display: grid;
      grid-template-rows: auto 1fr auto;
      gap: 18px;
      border-right: 1px solid rgba(255,255,255,0.08);
      background: rgba(0,0,0,0.22);
      padding: 22px 16px;
    }
    .dot-stack { display: flex; gap: 6px; }
    .dot { width: 9px; height: 9px; border-radius: 50%; background: rgba(255,255,255,0.24); }
    .dot:nth-child(1) { background: #ff6464; } .dot:nth-child(2) { background: #ffd56a; } .dot:nth-child(3) { background: var(--accent); }
    .side-icons { display: grid; gap: 12px; align-content: start; padding-top: 16px; }
    .side-icon {
      display: grid;
      place-items: center;
      width: 48px;
      height: 48px;
      border: 1px solid rgba(255,255,255,0.08);
      border-radius: 16px;
      background: rgba(255,255,255,0.045);
      color: var(--muted);
      font-weight: 860;
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.08);
    }
    .side-icon.active { color: var(--accent-on); background: linear-gradient(180deg, #75ff9c, #42df75); box-shadow: 0 16px 40px rgba(92,255,145,0.22); }
    .side-footer { width: 48px; height: 48px; border-radius: 16px; background: url('./fusion/app-icon.png') center/cover; box-shadow: 0 0 0 1px rgba(255,255,255,0.12); }
    .app-area { position: relative; z-index: 2; padding: 24px; }
    .app-top { display: flex; justify-content: space-between; align-items: flex-start; gap: 16px; margin-bottom: 20px; }
    .app-kicker { color: var(--accent); font: 760 12px/1.2 ui-monospace, SFMono-Regular, Menlo, monospace; letter-spacing: .12em; text-transform: uppercase; }
    .app-title { margin-top: 7px; font-size: 28px; font-weight: 860; letter-spacing: -0.055em; }
    .status-pill { border: 1px solid rgba(92,255,145,0.2); border-radius: 999px; background: rgba(92,255,145,0.08); color: #caffd8; padding: 8px 10px; font-size: 12px; font-weight: 760; white-space: nowrap; }
    .game-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 14px; }
    .game-card {
      position: relative;
      min-height: 164px;
      overflow: hidden;
      border: 1px solid rgba(255,255,255,0.09);
      border-radius: 22px;
      background: linear-gradient(150deg, rgba(255,255,255,0.09), rgba(255,255,255,0.026));
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.08);
      padding: 16px;
    }
    .game-card::after {
      content: '';
      position: absolute;
      right: -34px;
      bottom: -48px;
      width: 150px;
      height: 150px;
      border-radius: 34px;
      background: linear-gradient(135deg, rgba(92,255,145,0.28), rgba(59,214,198,0.06));
      transform: rotate(12deg);
    }
    .platform { color: var(--soft); font-size: 12px; font-weight: 740; text-transform: uppercase; letter-spacing: .1em; }
    .game-card h3 { position: relative; z-index: 1; margin: 46px 0 0; font-size: 19px; letter-spacing: -0.035em; }
    .game-card p { position: relative; z-index: 1; margin: 7px 0 0; color: var(--soft); font-size: 12px; }
    .game-card.featured { grid-row: span 2; min-height: 342px; display: grid; align-content: end; background: radial-gradient(circle at 50% 28%, rgba(92,255,145,0.25), transparent 32%), linear-gradient(150deg, rgba(255,255,255,0.09), rgba(255,255,255,0.026)); }
    .game-card.featured img { position: absolute; width: 78%; left: 50%; top: 43%; transform: translate(-50%, -50%); filter: drop-shadow(0 24px 36px rgba(0,0,0,0.45)); opacity: .9; }
    .download-bar {
      display: grid;
      grid-template-columns: 1fr auto;
      gap: 14px;
      align-items: center;
      margin-top: 14px;
      border: 1px solid rgba(255,255,255,0.09);
      border-radius: 20px;
      background: rgba(0,0,0,0.36);
      padding: 14px;
    }
    .progress { height: 8px; overflow: hidden; border-radius: 999px; background: rgba(255,255,255,0.08); }
    .progress span { display:block; width: 72%; height: 100%; border-radius: inherit; background: linear-gradient(90deg, var(--accent), var(--accent-2)); box-shadow: 0 0 18px rgba(92,255,145,0.46); }
    .download-bar strong { font-size: 13px; }
    .download-bar small { display:block; color: var(--soft); margin-top: 4px; }
    .floating-card {
      position: absolute;
      z-index: 5;
      border: 1px solid rgba(255,255,255,0.11);
      border-radius: 18px;
      background: rgba(9, 10, 10, 0.76);
      backdrop-filter: blur(20px);
      box-shadow: 0 18px 50px rgba(0,0,0,0.42), inset 0 1px 0 rgba(255,255,255,0.08);
      padding: 15px 16px;
      min-width: 210px;
    }
    .floating-card b { display: block; font-size: 13px; margin-bottom: 6px; }
    .floating-card span { display: block; color: var(--soft); font-size: 12px; }
    .floating-card.source { left: -18px; top: 88px; }
    .floating-card.emu { right: -10px; top: 42px; }
    .floating-card.play { right: 18px; bottom: 52px; min-width: 230px; }
    .orbit {
      position: absolute;
      z-index: 1;
      right: 42px;
      top: 86px;
      width: 520px;
      height: 520px;
      border: 1px solid rgba(92,255,145,0.10);
      border-radius: 50%;
      opacity: .55;
      animation: spin 26s linear infinite;
    }
    .orbit::before, .orbit::after { content:''; position:absolute; width:10px; height:10px; border-radius:50%; background:var(--accent); box-shadow:0 0 20px rgba(92,255,145,0.8); }
    .orbit::before { left: 50%; top: -5px; } .orbit::after { right: 10%; bottom: 12%; background: var(--accent-2); }
    @keyframes spin { to { transform: rotate(360deg); } }
    .marquee {
      position: relative;
      overflow: hidden;
      border-block: 1px solid rgba(255,255,255,0.07);
      background: rgba(255,255,255,0.025);
      margin-bottom: 34px;
      -webkit-mask-image: linear-gradient(90deg, transparent, black 8%, black 92%, transparent);
      mask-image: linear-gradient(90deg, transparent, black 8%, black 92%, transparent);
    }
    .marquee-track {
      display: flex;
      gap: 10px;
      width: max-content;
      padding: 14px 0;
      animation: slide 28s linear infinite;
    }
    .marquee span {
      display: inline-flex;
      align-items: center;
      border: 1px solid rgba(255,255,255,0.08);
      border-radius: 999px;
      background: rgba(255,255,255,0.04);
      color: rgba(247,248,243,0.72);
      padding: 9px 13px;
      font-size: 13px;
      font-weight: 720;
      white-space: nowrap;
    }
    @keyframes slide { to { transform: translateX(-50%); } }
    .section { position: relative; padding: 74px 0; }
    .section + .section { border-top: 1px solid rgba(255,255,255,0.07); }
    .section-head { display: flex; align-items: end; justify-content: space-between; gap: 26px; margin-bottom: 30px; }
    .section h2 { margin: 0; max-width: 760px; font-size: clamp(34px, 4.6vw, 64px); line-height: .98; letter-spacing: -0.068em; text-wrap: balance; }
    .section .section-copy { max-width: 460px; margin: 0; color: var(--muted); line-height: 1.68; }
    .flow-grid { display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap: 12px; }
    .step {
      position: relative;
      min-height: 188px;
      overflow: hidden;
      border: 1px solid rgba(92,255,145,0.15);
      border-radius: 24px;
      background: linear-gradient(180deg, rgba(92,255,145,0.08), rgba(255,255,255,0.025));
      padding: 22px;
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.07);
    }
    .step::after { content:''; position:absolute; right:-32px; bottom:-40px; width:110px; height:110px; border-radius:34px; background:rgba(92,255,145,0.08); transform:rotate(18deg); }
    .step b { display: block; color: var(--accent); font: 800 12px/1 ui-monospace, SFMono-Regular, Menlo, monospace; letter-spacing: .14em; margin-bottom: 52px; }
    .step strong { display:block; margin-bottom: 9px; font-size: 18px; letter-spacing: -0.03em; }
    .step span { color: var(--muted); font-size: 14px; line-height: 1.48; }
    .cards { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 16px; }
    .card {
      position: relative;
      min-height: 260px;
      overflow: hidden;
      border: 1px solid rgba(255,255,255,0.09);
      border-radius: 28px;
      background: linear-gradient(180deg, rgba(255,255,255,0.07), rgba(255,255,255,0.027));
      padding: 26px;
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.08);
    }
    .card::before { content:''; position:absolute; inset:auto -20% -35% 22%; height:180px; border-radius:50%; background:radial-gradient(circle, rgba(92,255,145,0.12), transparent 68%); }
    .card .kicker { color: var(--accent); font: 820 12px/1 ui-monospace, SFMono-Regular, Menlo, monospace; letter-spacing: .14em; text-transform: uppercase; }
    .card h3 { margin: 80px 0 12px; font-size: 24px; letter-spacing: -0.045em; }
    .card p { margin: 0; color: var(--muted); line-height: 1.62; }
    .legal {
      display: grid;
      grid-template-columns: minmax(0, .9fr) minmax(320px, 1.1fr);
      gap: 22px;
      align-items: stretch;
    }
    .legal-panel, .code-panel {
      border: 1px solid rgba(255,255,255,0.10);
      border-radius: 30px;
      background: linear-gradient(180deg, rgba(255,255,255,0.07), rgba(255,255,255,0.025));
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.08);
      padding: 30px;
    }
    .legal-panel h2 { font-size: clamp(34px, 4.6vw, 58px); }
    .legal-panel p { color: var(--muted); line-height: 1.7; }
    .code-panel { display: grid; align-content: center; gap: 14px; background: rgba(0,0,0,0.34); }
    .terminal-line { display: flex; align-items: center; gap: 10px; color: var(--soft); font: 500 13px/1.6 ui-monospace, SFMono-Regular, Menlo, monospace; }
    .terminal-line::before { content: '>'; color: var(--accent); }
    .mono { color: #caffd8; overflow-wrap: anywhere; }
    footer { display:flex; justify-content:space-between; gap:18px; color: var(--soft); padding: 54px 0 70px; font-size: 14px; }
    @media (prefers-reduced-motion: reduce) {
      .orbit, .marquee-track { animation: none; }
      html { scroll-behavior: auto; }
    }
    @media (max-width: 1060px) {
      .hero { grid-template-columns: 1fr; min-height: auto; }
      .showcase { min-height: 590px; }
      .launcher-window { margin: 0 auto; transform: none; }
      .section-head { display: block; }
      .section-copy { margin-top: 16px !important; }
      .flow-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
      .cards, .legal { grid-template-columns: 1fr; }
    }
    @media (max-width: 720px) {
      .page { width: min(100% - 28px, 1220px); }
      header { align-items: flex-start; border-radius: 26px; flex-direction: column; padding: 12px; }
      nav { flex-wrap: wrap; }
      .hero { padding-top: 44px; }
      h1 { font-size: clamp(46px, 15vw, 72px); }
      .lead { font-size: 17px; }
      .showcase { min-height: auto; }
      .launcher-window { grid-template-columns: 1fr; min-height: auto; border-radius: 28px; }
      .sidebar { display: none; }
      .game-grid { grid-template-columns: 1fr; }
      .game-card.featured { min-height: 270px; }
      .floating-card, .orbit { display: none; }
      .flow-grid { grid-template-columns: 1fr; }
      .section { padding: 54px 0; }
      footer { flex-direction: column; }
    }
  </style>
</head>
<body>
  <div class="nav-wrap">
    <div class="page">
      <header>
        <a class="brand" href="./" aria-label="Fusion Launcher home">
          <img src="./fusion/app-icon.png" alt="">
          <span>fusion launcher</span>
        </a>
        <nav aria-label="Primary">
          <a href="${templateUrl}">Source template</a>
          <a href="${githubUrl}">GitHub</a>
        </nav>
      </header>
    </div>
  </div>

  <main class="page">
    <section class="hero">
      <div class="hero-copy">
        <div class="eyebrow">Source libraries + guided emulator setup</div>
        <h1>Organize and launch <span class="gradient-word">retro game</span> libraries from one PC launcher.</h1>
        <p class="lead">Fusion Launcher connects sources, tracks allowed install flows, prepares emulator requirements, enriches artwork, and launches everything from a local-first Windows desktop app.</p>
        <div class="actions">
          <a class="button primary" href="${githubUrl}">View on GitHub</a>
          <a class="button secondary" href="${templateUrl}">Open source-library template</a>
        </div>
        <p class="note">Content-neutral by design: Fusion Launcher does not host or distribute commercial games, BIOS files, firmware, keys, or third-party payloads.</p>
        <div class="hero-tags" aria-label="Product traits">
          <span>Source-driven</span>
          <span>Windows-first</span>
          <span>Local storage</span>
          <span>Emulator preflight</span>
        </div>
      </div>

      <div class="showcase" aria-label="Fusion Launcher product showcase">
        <div class="halo"></div>
        <div class="orbit"></div>
        <div class="floating-card source"><b>Connect source</b><span>GitHub Pages, HTTPS, or local JSON</span></div>
        <div class="floating-card emu"><b>Prepare emulator</b><span>Portable profiles and system-file checks</span></div>
        <div class="floating-card play"><b>Ready to play</b><span>Launch after preflight passes</span></div>
        <div class="launcher-window">
          <aside class="sidebar" aria-hidden="true">
            <div class="dot-stack"><span class="dot"></span><span class="dot"></span><span class="dot"></span></div>
            <div class="side-icons"><div class="side-icon active">▶</div><div class="side-icon">◎</div><div class="side-icon">▦</div><div class="side-icon">⚙</div></div>
            <div class="side-footer"></div>
          </aside>
          <div class="app-area">
            <div class="app-top">
              <div><div class="app-kicker">source connected</div><div class="app-title">Retro library</div></div>
              <div class="status-pill">Local-first</div>
            </div>
            <div class="game-grid">
              <article class="game-card featured"><img src="./fusion/hero-mascot.png" alt=""><div class="platform">demo source</div><h3>Fusion smoke test</h3><p>First-party validation content</p></article>
              <article class="game-card"><div class="platform">NES</div><h3>Pixel catalog</h3><p>Metadata + cover ready</p></article>
              <article class="game-card"><div class="platform">PS2</div><h3>Launch profile</h3><p>Requirements checked</p></article>
              <article class="game-card"><div class="platform">PSP</div><h3>Portable setup</h3><p>Emulator path stored locally</p></article>
              <article class="game-card"><div class="platform">Source</div><h3>JSON library</h3><p>Community or personal URL</p></article>
            </div>
            <div class="download-bar"><div><strong>Installing allowed source content</strong><small>Direct, bundled, and torrent-aware state</small></div><div class="progress" aria-hidden="true"><span></span></div></div>
          </div>
        </div>
      </div>
    </section>
  </main>

  <div class="marquee" aria-hidden="true">
    <div class="marquee-track">
      <span>Connect source</span><span>Browse catalog</span><span>Install content</span><span>Prepare emulator</span><span>Scrape artwork</span><span>Run preflight</span><span>Launch game</span>
      <span>Connect source</span><span>Browse catalog</span><span>Install content</span><span>Prepare emulator</span><span>Scrape artwork</span><span>Run preflight</span><span>Launch game</span>
    </div>
  </div>

  <main class="page">
    <section class="section">
      <div class="section-head">
        <h2>Connect source → install → set up → play.</h2>
        <p class="section-copy">Fusion Launcher is built around source libraries: catalogs that describe games, platforms, metadata, artwork, setup profiles, and install requirements.</p>
      </div>
      <div class="flow-grid">
        <div class="step"><b>01</b><strong>Connect</strong><span>Paste or import a source-library URL.</span></div>
        <div class="step"><b>02</b><strong>Browse</strong><span>Explore retro game catalogs in one launcher.</span></div>
        <div class="step"><b>03</b><strong>Install</strong><span>Track files described by the connected source.</span></div>
        <div class="step"><b>04</b><strong>Set up</strong><span>Prepare emulator paths and required system files.</span></div>
        <div class="step"><b>05</b><strong>Launch</strong><span>Start games after readiness checks pass.</span></div>
      </div>
    </section>

    <section class="section">
      <div class="section-head">
        <h2>A launcher layer for source-based game libraries.</h2>
        <p class="section-copy">The pitch is sharp: source-library catalog flow, guided setup assistance, and a Windows desktop launcher shell.</p>
      </div>
      <div class="cards">
        <article class="card"><div class="kicker">Sources</div><h3>Source-library catalog flow</h3><p>Fusion connects user-selected source libraries and turns them into a browsable launcher catalog with download state.</p></article>
        <article class="card"><div class="kicker">Setup</div><h3>Guided emulator setup</h3><p>Supported platforms get portable emulator setup help, local paths, system-file checks, and launch preflight diagnostics.</p></article>
        <article class="card"><div class="kicker">Desktop</div><h3>Windows launcher UX</h3><p>Built with Tauri, Rust, Next.js, and local-first storage for a fast PC launcher experience.</p></article>
      </div>
    </section>

    <section class="section legal" id="content-model">
      <div class="legal-panel">
        <h2>Content-neutral by design.</h2>
        <p>Fusion Launcher provides source connection, download tracking, metadata enrichment, emulator setup assistance, and launch orchestration.</p>
        <p>The official project does not ship commercial ROMs, BIOS files, firmware, keys, or third-party game payloads. Users choose their sources and are responsible for the content they connect.</p>
      </div>
      <div class="code-panel">
        <div class="terminal-line">starter source template</div>
        <div class="terminal-line mono">${templateUrl}</div>
        <div class="terminal-line">template updated ${updatedAt}</div>
        <div class="terminal-line">not affiliated with emulator projects, publishers, or console manufacturers</div>
      </div>
    </section>
  </main>

  <footer class="page">
    <span>Fusion Launcher</span>
    <span><a href="${githubUrl}">GitHub</a> · <a href="${templateUrl}">Source template</a></span>
  </footer>
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
