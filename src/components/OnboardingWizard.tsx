'use client';

import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { CheckCircle2, Download, FolderOpen, Loader2, ShieldAlert } from 'lucide-react';
import { api } from '@/lib/api';
import { isTauriRuntime } from '@/lib/runtime';
import {
  MVP_PLATFORMS,
  PLATFORM_DEFAULT_LAUNCH_ARGS,
  PLATFORM_EMULATOR_HINTS,
  PLATFORM_LABELS,
  type MvpPlatform
} from '@/types/platform';
import type { CatalogGame, OnboardingState, RepositoryPreview } from '@/types/repository';

const OFFICIAL_DEMO_REPO =
  process.env.NODE_ENV === 'production'
    ? 'https://cdn.retrohydra.app/repos/demo-v1.json'
    : 'http://localhost:3000/demo-repository.json';

interface OnboardingWizardProps {
  state: OnboardingState | null;
  catalog: CatalogGame[];
  initialMessage: string | null;
  onReload: () => Promise<void>;
}

export function OnboardingWizard({
  state,
  catalog,
  initialMessage,
  onReload
}: OnboardingWizardProps) {
  const [repoUrl, setRepoUrl] = useState(OFFICIAL_DEMO_REPO);
  const [preview, setPreview] = useState<RepositoryPreview | null>(null);
  const [platform, setPlatform] = useState<MvpPlatform>('ps1');
  const [emulatorPath, setEmulatorPath] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(initialMessage);

  useEffect(() => {
    setMessage(initialMessage);
  }, [initialMessage]);

  const currentStep = state?.step ?? 'addRepository';
  const firstGame = useMemo(() => catalog[0] ?? null, [catalog]);

  const previewRepository = async () => {
    setBusy('preview');
    setMessage(null);
    try {
      const nextPreview = await api.previewRepository(repoUrl.trim());
      setPreview(nextPreview);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const connectRepository = async () => {
    setBusy('connect');
    setMessage(null);
    try {
      const nextPreview = preview ?? await api.previewRepository(repoUrl.trim());
      if (nextPreview.trustLevel === 'unknown') {
        const confirmed = window.confirm(
          `Connect unknown repository?\n\n${nextPreview.name}\n${nextPreview.url}\n${nextPreview.catalogCount} games, ${nextPreview.systemFileCount} system files.`
        );
        if (!confirmed) return;
      }
      await api.connectRepository(repoUrl.trim());
      await onReload();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const browseForEmulator = async () => {
    if (!isTauriRuntime()) {
      setMessage('File browsing is available in the desktop build. Paste a path for preview.');
      return;
    }
    setBusy('browse');
    setMessage(null);
    try {
      const selected = await open({
        title: `Select ${PLATFORM_LABELS[platform]} emulator`,
        multiple: false,
        directory: false,
        defaultPath: emulatorPath || undefined,
        filters: [{ name: 'Executable', extensions: ['exe'] }]
      });
      if (typeof selected === 'string') setEmulatorPath(selected);
    } catch (error) {
      setMessage(`Failed to browse for emulator: ${error}`);
    } finally {
      setBusy(null);
    }
  };

  const saveEmulator = async () => {
    if (!emulatorPath.trim()) return;
    setBusy('emulator');
    setMessage(null);
    try {
      await api.saveEmulatorConfig(platform, emulatorPath.trim(), PLATFORM_DEFAULT_LAUNCH_ARGS[platform]);
      await onReload();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const downloadFirstGame = async () => {
    if (!firstGame) return;
    setBusy('download');
    setMessage(null);
    try {
      await api.startGameDownload(firstGame.id);
      await onReload();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  return (
    <main className="min-h-screen bg-[#0f0f11] px-6 py-10 text-white">
      <section className="mx-auto w-full max-w-4xl">
        <div className="mb-8">
          <div className="text-xs font-black uppercase tracking-[0.2em] text-hydra-accent">RetroHydra MVP setup</div>
          <h1 className="mt-3 text-4xl font-black tracking-normal">Connect. Configure. Download.</h1>
        </div>

        <div className="grid gap-4 md:grid-cols-3">
          <StepCard active={currentStep === 'addRepository'} done={Boolean(state?.repositoriesConfigured)} title="1. Repository">
            <input
              value={repoUrl}
              onChange={(event) => {
                setRepoUrl(event.target.value);
                setPreview(null);
              }}
              className="h-10 w-full rounded-md border border-white/12 bg-black/24 px-3 text-sm outline-none focus:border-hydra-accent/70"
              placeholder="https://example.com/repo.json"
            />
            {preview && (
              <div className="mt-3 rounded-md border border-white/10 bg-white/[0.04] p-3 text-xs leading-5 text-white/62">
                <div className="font-bold text-white">{preview.name}</div>
                <div>{preview.catalogCount} games / {preview.systemFileCount} system files</div>
                <div>Trust: {preview.trustLevel}</div>
                {preview.hasExecutableAssets && <div className="text-amber-100">Contains executable assets</div>}
              </div>
            )}
            <div className="mt-3 flex gap-2">
              <button onClick={previewRepository} disabled={busy !== null} className="rh-mini-action">
                {busy === 'preview' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldAlert className="h-3.5 w-3.5" />}
                Preview
              </button>
              <button onClick={connectRepository} disabled={busy !== null || !repoUrl.trim()} className="rh-primary-action">
                Connect
              </button>
            </div>
          </StepCard>

          <StepCard active={currentStep === 'configureEmulator'} done={Boolean(state?.emulatorsConfigured)} title="2. Emulator">
            <select
              value={platform}
              onChange={(event) => setPlatform(event.target.value as MvpPlatform)}
              className="h-10 w-full rounded-md border border-white/12 bg-black/24 px-3 text-sm outline-none"
            >
              {MVP_PLATFORMS.map((item) => (
                <option key={item} value={item}>{PLATFORM_LABELS[item]}</option>
              ))}
            </select>
            <div className="mt-2 text-xs text-white/42">Expected: {PLATFORM_EMULATOR_HINTS[platform]}</div>
            <div className="mt-3 flex gap-2">
              <input
                value={emulatorPath}
                onChange={(event) => setEmulatorPath(event.target.value)}
                className="h-10 min-w-0 flex-1 rounded-md border border-white/12 bg-black/24 px-3 text-sm outline-none"
                placeholder="C:\\Emulators\\..."
              />
              <button onClick={browseForEmulator} disabled={busy !== null} className="rh-icon-button">
                <FolderOpen className="h-4 w-4" />
              </button>
            </div>
            <button onClick={saveEmulator} disabled={busy !== null || !emulatorPath.trim()} className="rh-primary-action mt-3">
              Save emulator
            </button>
          </StepCard>

          <StepCard active={currentStep === 'complete'} done={false} title="3. First game">
            <div className="text-sm font-bold">{firstGame?.title ?? 'Waiting for catalog'}</div>
            <div className="mt-1 text-xs text-white/46">{firstGame ? PLATFORM_LABELS[firstGame.platform] : 'Connect a repository first'}</div>
            <button onClick={downloadFirstGame} disabled={busy !== null || !firstGame} className="rh-primary-action mt-4">
              {busy === 'download' ? <Loader2 className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
              Download
            </button>
          </StepCard>
        </div>

        {message && (
          <div className="mt-5 rounded-md border border-amber-300/24 bg-amber-300/10 px-3 py-2 text-sm text-amber-100">
            {message}
          </div>
        )}
      </section>
    </main>
  );
}

function StepCard({
  active,
  done,
  title,
  children
}: {
  active: boolean;
  done: boolean;
  title: string;
  children: ReactNode;
}) {
  return (
    <section className={`rounded-lg border p-5 ${active ? 'border-hydra-accent bg-white/[0.06]' : 'border-white/10 bg-white/[0.035]'}`}>
      <div className="mb-4 flex items-center justify-between gap-3">
        <h2 className="text-lg font-black">{title}</h2>
        {done && <CheckCircle2 className="h-5 w-5 text-hydra-green" />}
      </div>
      {children}
    </section>
  );
}
