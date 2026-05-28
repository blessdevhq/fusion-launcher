'use client';

import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import {
  CheckCircle2,
  Download,
  FolderOpen,
  Link2,
  Loader2,
  Play,
  ShieldAlert,
  Wrench
} from 'lucide-react';
import { api } from '@/lib/api';
import { isTauriRuntime } from '@/lib/runtime';
import {
  MVP_PLATFORMS,
  PLATFORM_DEFAULT_LAUNCH_ARGS,
  PLATFORM_EMULATOR_HINTS,
  PLATFORM_LABELS,
  type MvpPlatform
} from '@/types/platform';
import type {
  CatalogGame,
  OnboardingState,
  RecommendedEmulator,
  RepositoryPreview,
  TorrentDownloadRecord
} from '@/types/repository';

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
  const [repoUrl, setRepoUrl] = useState('');
  const [preview, setPreview] = useState<RepositoryPreview | null>(null);
  const [builtInPreview, setBuiltInPreview] = useState<RepositoryPreview | null>(null);
  const [platform, setPlatform] = useState<MvpPlatform>('nes');
  const [emulatorPath, setEmulatorPath] = useState('');
  const [recommendedEmulators, setRecommendedEmulators] = useState<RecommendedEmulator[]>([]);
  const [demoDownload, setDemoDownload] = useState<TorrentDownloadRecord | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(initialMessage);

  useEffect(() => {
    setMessage(initialMessage);
  }, [initialMessage]);

  const demoGame = useMemo(
    () => catalog.find((game) => game.id.includes('retrohydra_nes_smoke')) ?? catalog.find((game) => game.platform === 'nes') ?? catalog[0] ?? null,
    [catalog]
  );
  const mesen = useMemo(
    () => recommendedEmulators.find((emulator) => emulator.platform === 'nes') ?? null,
    [recommendedEmulators]
  );
  const repositoryReady = Boolean(state?.repositoriesConfigured);
  const emulatorReady = mesen?.status === 'installed';
  const demoDownloaded = demoDownload?.status === 'completed';
  const playable = Boolean(repositoryReady && emulatorReady && demoDownloaded && demoGame);
  const primaryBusy = busy === 'setup' || busy === 'play';

  useEffect(() => {
    let cancelled = false;

    async function loadSetupState() {
      try {
        const [emulators, download] = await Promise.all([
          api.getRecommendedEmulators(),
          demoGame ? api.getGameDownload(demoGame.id) : Promise.resolve(null)
        ]);
        if (!cancelled) {
          setRecommendedEmulators(emulators);
          setDemoDownload(download);
        }
      } catch (error) {
        if (!cancelled) setMessage(error instanceof Error ? error.message : String(error));
      }
    }

    void loadSetupState();
    return () => {
      cancelled = true;
    };
  }, [demoGame?.id, state?.repositoriesConfigured, state?.validEmulatorCount]);

  const refreshSetupState = async (gameId?: string) => {
    const [emulators, download] = await Promise.all([
      api.getRecommendedEmulators(),
      gameId ? api.getGameDownload(gameId) : demoGame ? api.getGameDownload(demoGame.id) : Promise.resolve(null)
    ]);
    setRecommendedEmulators(emulators);
    setDemoDownload(download);
  };

  const setupPlayableDemo = async () => {
    setBusy('setup');
    setMessage(null);
    try {
      let games = catalog;
      if (!repositoryReady || !games.some((game) => game.id.includes('retrohydra_nes_smoke'))) {
        await api.connectBuiltInDemoRepository();
        games = await api.getCatalog();
      } else if (games.length === 0) {
        games = await api.getCatalog();
      }

      const targetGame =
        games.find((game) => game.id.includes('retrohydra_nes_smoke')) ??
        games.find((game) => game.platform === 'nes');
      if (!targetGame) {
        throw new Error('Built-in NES demo was not found after repository setup.');
      }

      const emulators = await api.getRecommendedEmulators();
      const currentMesen = emulators.find((emulator) => emulator.platform === 'nes');
      if (currentMesen?.status !== 'installed') {
        await api.installRecommendedEmulator('nes');
      }

      const currentDownload = await api.getGameDownload(targetGame.id);
      if (currentDownload?.status !== 'completed') {
        await api.startGameDownload(targetGame.id);
      }

      await onReload();
      await refreshSetupState(targetGame.id);
      setMessage('Playable demo is ready.');
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const playDemo = async () => {
    setBusy('play');
    setMessage(null);
    try {
      const targetGame = demoGame ?? (await api.getCatalog()).find((game) => game.platform === 'nes');
      if (!targetGame) throw new Error('No playable demo game is connected.');
      await api.launchGame(targetGame.id);
      setMessage(`Launched ${targetGame.title}.`);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

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

  const previewBuiltInRepository = async () => {
    setBusy('builtin-preview');
    setMessage(null);
    try {
      setBuiltInPreview(await api.previewBuiltInDemoRepository());
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const connectBuiltInRepository = async () => {
    setBusy('builtin-connect');
    setMessage(null);
    try {
      await api.connectBuiltInDemoRepository();
      await onReload();
      await refreshSetupState();
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
      await refreshSetupState();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const downloadFirstGame = async () => {
    if (!demoGame) return;
    setBusy('download');
    setMessage(null);
    try {
      await api.startGameDownload(demoGame.id);
      await onReload();
      await refreshSetupState(demoGame.id);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  return (
    <main className="min-h-screen bg-[#0f0f11] px-6 py-10 text-white">
      <section className="mx-auto w-full max-w-5xl">
        <div className="mb-8 max-w-3xl">
          <div className="text-xs font-black uppercase tracking-[0.2em] text-hydra-accent">RetroHydra MVP setup</div>
          <h1 className="mt-3 text-4xl font-black tracking-normal">Playable demo in one pass</h1>
          <p className="mt-3 text-sm leading-6 text-white/52">
            RetroHydra ships only first-party demo content. Commercial ROMs, BIOS files, firmware, and keys are not included.
          </p>
        </div>

        <div className="grid gap-4 lg:grid-cols-[1.15fr_0.85fr]">
          <section className="rounded-lg border border-white/10 bg-white/[0.04] p-5">
            <div className="flex flex-wrap items-start justify-between gap-4">
              <div>
                <h2 className="text-2xl font-black">Built-in NES Demo</h2>
                <div className="mt-1 text-sm text-white/46">
                  {demoGame?.title ?? 'RetroHydra NES Smoke Demo'} with pinned Mesen2 {mesen?.version ?? '2.1.1'}
                </div>
              </div>
              <button
                onClick={playable ? playDemo : setupPlayableDemo}
                disabled={primaryBusy}
                className="rh-primary-action"
              >
                {primaryBusy ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : playable ? (
                  <Play className="h-4 w-4" />
                ) : (
                  <Wrench className="h-4 w-4" />
                )}
                {playable ? 'Play Demo' : 'Set up demo'}
              </button>
            </div>

            <div className="mt-6 grid gap-3 sm:grid-cols-2">
              <SetupItem done={repositoryReady} title="Repository" detail="Built-in demo connected" />
              <SetupItem done={emulatorReady} title="Emulator" detail={mesen?.installedPath ?? 'Mesen2 automatic setup'} />
              <SetupItem done={demoDownloaded} title="Demo ROM" detail={demoDownload?.saveDir ?? 'Bundled smoke ROM'} />
              <SetupItem done={playable} title="Ready" detail="Health check should pass after setup" />
            </div>
          </section>

          <section className="rounded-lg border border-white/10 bg-white/[0.035] p-5">
            <h2 className="text-lg font-black">Manual Controls</h2>
            <div className="mt-4 space-y-3">
              <button onClick={connectBuiltInRepository} disabled={busy !== null} className="rh-mini-action">
                {busy === 'builtin-connect' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Download className="h-3.5 w-3.5" />}
                Use built-in demo
              </button>
              <button onClick={previewBuiltInRepository} disabled={busy !== null} className="rh-mini-action">
                {busy === 'builtin-preview' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldAlert className="h-3.5 w-3.5" />}
                Preview built-in
              </button>
              <button onClick={downloadFirstGame} disabled={busy !== null || !demoGame} className="rh-mini-action">
                {busy === 'download' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Download className="h-3.5 w-3.5" />}
                Download demo
              </button>
            </div>
            {builtInPreview && (
              <div className="mt-4 rounded-md border border-white/10 bg-black/20 p-3 text-xs leading-5 text-white/62">
                <div className="font-bold text-white">{builtInPreview.name}</div>
                <div>{builtInPreview.catalogCount} game / {builtInPreview.systemFileCount} system files</div>
                <div>Trust: {builtInPreview.trustLevel}</div>
              </div>
            )}
          </section>
        </div>

        <details className="mt-4 rounded-lg border border-white/10 bg-white/[0.03] p-5">
          <summary className="cursor-pointer text-sm font-black uppercase text-white/70">Advanced repository and emulator setup</summary>
          <div className="mt-5 grid gap-4 md:grid-cols-2">
            <StepCard title="Community Repository">
              <div className="mb-2 flex items-center gap-2 text-xs font-semibold text-white/52">
                <Link2 className="h-3.5 w-3.5" />
                Repository URL
              </div>
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
                <button onClick={previewRepository} disabled={busy !== null || !repoUrl.trim()} className="rh-mini-action">
                  {busy === 'preview' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldAlert className="h-3.5 w-3.5" />}
                  Preview
                </button>
                <button onClick={connectRepository} disabled={busy !== null || !repoUrl.trim()} className="rh-mini-action">
                  Connect
                </button>
              </div>
            </StepCard>

            <StepCard title="Manual Emulator">
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
          </div>
        </details>

        {message && (
          <div className="mt-5 rounded-md border border-amber-300/24 bg-amber-300/10 px-3 py-2 text-sm text-amber-100">
            {message}
          </div>
        )}
      </section>
    </main>
  );
}

function SetupItem({ done, title, detail }: { done: boolean; title: string; detail: string }) {
  return (
    <div className={`rounded-md border p-4 ${done ? 'border-emerald-300/24 bg-emerald-300/10' : 'border-white/10 bg-black/18'}`}>
      <div className="flex items-center gap-2">
        <CheckCircle2 className={`h-4 w-4 ${done ? 'text-emerald-200' : 'text-white/22'}`} />
        <span className="font-black">{title}</span>
      </div>
      <div className="mt-2 truncate text-xs text-white/46">{detail}</div>
    </div>
  );
}

function StepCard({
  title,
  children
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <section className="rounded-lg border border-white/10 bg-black/16 p-5">
      <h2 className="mb-4 text-lg font-black">{title}</h2>
      {children}
    </section>
  );
}
