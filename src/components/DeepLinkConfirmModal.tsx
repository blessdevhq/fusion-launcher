'use client';

import { useEffect, useRef, useState } from 'react';
import {
  AlertCircle,
  ArrowLeft,
  ChevronRight,
  ExternalLink,
  HardDrive,
  Layers,
  PackageCheck,
  ShieldAlert,
  X,
} from 'lucide-react';
import { api } from '@/lib/api';
import type { Manifest, ManifestGame } from '@/types/manifest';

export interface PendingDeepLinkRequest {
  id: string;
  type: 'addSource' | 'install';
  url: string;
  host: string;
  onConfirm?: () => Promise<void>;
  onConfirmSource?: () => Promise<void>;
  onConfirmInstall?: (titleId: string) => Promise<void>;
}

interface DeepLinkConfirmModalProps {
  request: PendingDeepLinkRequest;
  onClose: () => void;
}

function extractDownloadHosts(game: ManifestGame): string[] {
  const hosts: string[] = [];
  if (game.assets.heavy_rom_magnet) {
    hosts.push('Torrent / Magnet');
  }
  const addHost = (urlStr?: string | null) => {
    if (!urlStr) return;
    try {
      const h = new URL(urlStr).host;
      if (h && !hosts.includes(h)) hosts.push(h);
    } catch {
      // ignore invalid URLs
    }
  };
  addHost(game.assets.core_bundle_url);
  addHost(game.assets.shader_cache_url);
  return hosts.length > 0 ? hosts : ['No remote download source declared'];
}

export function DeepLinkConfirmModal({ request, onClose }: DeepLinkConfirmModalProps) {
  const [step, setStep] = useState<1 | 2>(1);
  const [cooldown, setCooldown] = useState(true);
  const [busy, setBusy] = useState(false);
  const [manifest, setManifest] = useState<Manifest | null>(null);
  const [selectedTitleId, setSelectedTitleId] = useState<string>('');
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const activeRequestIdRef = useRef<string>(request.id);

  // Reset when request changes
  useEffect(() => {
    activeRequestIdRef.current = request.id;
    setStep(1);
    setManifest(null);
    setSelectedTitleId('');
    setErrorMessage(null);
    setBusy(false);
    setCooldown(true);

    const timer = setTimeout(() => {
      setCooldown(false);
    }, 1500);

    return () => clearTimeout(timer);
  }, [request.id]);

  // Reset 1.5s countdown on step change
  useEffect(() => {
    setCooldown(true);
    const timer = setTimeout(() => {
      setCooldown(false);
    }, 1500);

    return () => clearTimeout(timer);
  }, [step]);

  const handleConfirmSource = async () => {
    if (cooldown || busy) return;
    if (activeRequestIdRef.current !== request.id) return;
    setBusy(true);
    setErrorMessage(null);
    try {
      if (request.onConfirmSource) {
        await request.onConfirmSource();
      } else if (request.onConfirm) {
        await request.onConfirm();
      }
      onClose();
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleFetchManifestStep1 = async () => {
    if (cooldown || busy) return;
    if (activeRequestIdRef.current !== request.id) return;
    setBusy(true);
    setErrorMessage(null);
    try {
      const data = await api.fetchManifest(request.url);
      if (activeRequestIdRef.current !== request.id) return;
      if (!data.games || data.games.length === 0) {
        throw new Error('Manifest contains no installable games.');
      }
      setManifest(data);
      setSelectedTitleId(data.games[0].title_id);
      setStep(2);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleConfirmInstallStep2 = async () => {
    if (cooldown || busy || !selectedTitleId) return;
    if (activeRequestIdRef.current !== request.id) return;
    setBusy(true);
    setErrorMessage(null);
    try {
      if (request.onConfirmInstall) {
        await request.onConfirmInstall(selectedTitleId);
      } else if (request.onConfirm) {
        await request.onConfirm();
      }
      onClose();
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const selectedGame = manifest?.games.find((g) => g.title_id === selectedTitleId) ?? manifest?.games[0];
  const downloadHosts = selectedGame ? extractDownloadHosts(selectedGame) : [];
  const hasCoreBundle = Boolean(selectedGame?.assets.core_bundle_url?.trim());

  return (
    <div className="fixed inset-0 z-[70] grid place-items-center bg-black/75 px-5 backdrop-blur-sm">
      <section className="w-full max-w-lg rounded-2xl border border-white/12 bg-fusion-surface/95 p-6 shadow-[0_40px_120px_rgba(0,0,0,0.7)]">
        <header className="flex items-start gap-4">
          <div className="grid h-10 w-10 shrink-0 place-items-center rounded-md bg-amber-400/15 text-amber-200">
            <ShieldAlert className="h-5 w-5" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <h2 className="text-xl font-bold tracking-tight text-white">
                {request.type === 'addSource'
                  ? 'Connect External Repository'
                  : step === 1
                  ? 'Inspect Game Manifest'
                  : 'Confirm Game Installation'}
              </h2>
              {request.type === 'install' && (
                <span className="rounded-full border border-white/20 px-2 py-0.5 text-[11px] font-medium text-white/60">
                  Step {step}/2
                </span>
              )}
            </div>
            <p className="mt-1 text-sm text-white/70">
              {request.type === 'addSource'
                ? 'An external link requested access to add a repository. Verify the target host before proceeding.'
                : step === 1
                ? 'An external link requested installing a game. Fusion Launcher will inspect the manifest before anything is saved.'
                : 'Review the game details and download sources parsed from the remote manifest.'}
            </p>
          </div>
          <button
            onClick={onClose}
            className="grid h-9 w-9 place-items-center rounded-md border border-white/10 text-white/60 transition hover:text-white"
            title="Cancel"
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        {errorMessage && (
          <div className="mt-4 flex items-start gap-2.5 rounded-lg border border-red-500/30 bg-red-500/10 p-3 text-sm text-red-200">
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-red-400" />
            <div className="min-w-0 flex-1 break-words">{errorMessage}</div>
          </div>
        )}

        {/* STEP 1: AddSource OR Initial Manifest URL Verification */}
        {step === 1 && (
          <div className="mt-5 space-y-3">
            <div className="rounded-lg border border-white/10 bg-black/40 p-3">
              <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-white/50">
                <ExternalLink className="h-3.5 w-3.5" />
                Target Host
              </div>
              <div className="mt-1 text-base font-bold text-white selection:bg-white/20">
                {request.host}
              </div>
            </div>

            <div className="rounded-lg border border-white/10 bg-black/40 p-3">
              <div className="text-xs font-semibold uppercase tracking-wider text-white/50">
                Full HTTPS Resource
              </div>
              <div className="mt-1 break-all font-mono text-xs text-white/80 select-all">
                {request.url}
              </div>
            </div>
          </div>
        )}

        {/* STEP 2: Manifest Inspection Details */}
        {step === 2 && selectedGame && (
          <div className="mt-5 space-y-3">
            {manifest && manifest.games.length > 1 && (
              <div className="rounded-lg border border-white/10 bg-black/40 p-3">
                <label
                  htmlFor="deep-link-game-select"
                  className="block text-xs font-semibold uppercase tracking-wider text-white/50"
                >
                  Select Game ({manifest.games.length} available in manifest)
                </label>
                <select
                  id="deep-link-game-select"
                  value={selectedTitleId}
                  onChange={(e) => setSelectedTitleId(e.target.value)}
                  className="mt-1.5 w-full rounded-md border border-white/15 bg-neutral-900 px-3 py-2 text-sm text-white focus:border-fusion-primary focus:outline-none"
                >
                  {manifest.games.map((g) => (
                    <option key={g.title_id} value={g.title_id} className="bg-neutral-900 text-white">
                      {g.title} ({g.platform.toUpperCase()})
                    </option>
                  ))}
                </select>
              </div>
            )}

            <div className="grid grid-cols-2 gap-3">
              <div className="rounded-lg border border-white/10 bg-black/40 p-3">
                <div className="text-xs font-semibold uppercase tracking-wider text-white/50">
                  Game Title
                </div>
                <div className="mt-1 truncate text-sm font-bold text-white" title={selectedGame.title}>
                  {selectedGame.title}
                </div>
              </div>

              <div className="rounded-lg border border-white/10 bg-black/40 p-3">
                <div className="text-xs font-semibold uppercase tracking-wider text-white/50">
                  Platform
                </div>
                <div className="mt-1 text-sm font-bold uppercase text-white">
                  {selectedGame.platform}
                </div>
              </div>
            </div>

            <div className="rounded-lg border border-white/10 bg-black/40 p-3">
              <div className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wider text-white/50">
                <HardDrive className="h-3.5 w-3.5" />
                Download Hosts / Transport
              </div>
              <div className="mt-1 space-y-1">
                {downloadHosts.map((h, i) => (
                  <div key={i} className="font-mono text-xs text-white/80">
                    {h}
                  </div>
                ))}
              </div>
            </div>

            <div className="rounded-lg border border-white/10 bg-black/40 p-3">
              <div className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wider text-white/50">
                <Layers className="h-3.5 w-3.5" />
                Core Bundle (Emulator)
              </div>
              <div className="mt-1 flex items-center gap-2 text-xs">
                {hasCoreBundle ? (
                  <>
                    <span className="inline-flex items-center gap-1 rounded bg-emerald-500/20 px-2 py-0.5 font-medium text-emerald-300">
                      <PackageCheck className="h-3.5 w-3.5" />
                      Included in manifest
                    </span>
                    {selectedGame.assets.core_bundle_sha256 && (
                      <span className="font-mono text-[11px] text-white/50">
                        SHA-256: {selectedGame.assets.core_bundle_sha256.slice(0, 8)}...
                      </span>
                    )}
                  </>
                ) : (
                  <span className="text-white/60">
                    None (relies on user-configured or pre-installed emulator)
                  </span>
                )}
              </div>
            </div>
          </div>
        )}

        <footer className="mt-6 flex items-center justify-between gap-3">
          {step === 2 ? (
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                setStep(1);
                setErrorMessage(null);
              }}
              className="inline-flex items-center gap-1.5 rounded-lg border border-white/15 px-3 py-2 text-sm font-medium text-white/80 transition hover:bg-white/5 hover:text-white"
            >
              <ArrowLeft className="h-4 w-4" />
              Back
            </button>
          ) : (
            <button
              type="button"
              onClick={onClose}
              className="rounded-lg border border-white/15 px-4 py-2 text-sm font-medium text-white/80 transition hover:bg-white/5 hover:text-white"
            >
              Cancel
            </button>
          )}

          <div className="flex items-center gap-3">
            {step === 2 && (
              <button
                type="button"
                onClick={onClose}
                className="rounded-lg border border-white/15 px-4 py-2 text-sm font-medium text-white/80 transition hover:bg-white/5 hover:text-white"
              >
                Cancel
              </button>
            )}

            {request.type === 'addSource' ? (
              <button
                type="button"
                disabled={cooldown || busy}
                onClick={handleConfirmSource}
                className={`rounded-lg px-4 py-2 text-sm font-semibold transition ${
                  cooldown || busy
                    ? 'cursor-not-allowed bg-fusion-primary/40 text-white/50'
                    : 'bg-fusion-primary text-white shadow-lg shadow-fusion-primary/30 hover:bg-fusion-primary/90'
                }`}
              >
                {busy ? 'Processing...' : cooldown ? 'Confirming in 1.5s...' : 'Confirm & Proceed'}
              </button>
            ) : step === 1 ? (
              <button
                type="button"
                disabled={cooldown || busy}
                onClick={handleFetchManifestStep1}
                className={`inline-flex items-center gap-1.5 rounded-lg px-4 py-2 text-sm font-semibold transition ${
                  cooldown || busy
                    ? 'cursor-not-allowed bg-fusion-primary/40 text-white/50'
                    : 'bg-fusion-primary text-white shadow-lg shadow-fusion-primary/30 hover:bg-fusion-primary/90'
                }`}
              >
                {busy ? 'Fetching...' : cooldown ? 'Inspect in 1.5s...' : 'Inspect Manifest'}
                <ChevronRight className="h-4 w-4" />
              </button>
            ) : (
              <button
                type="button"
                disabled={cooldown || busy || !selectedTitleId}
                onClick={handleConfirmInstallStep2}
                className={`rounded-lg px-4 py-2 text-sm font-semibold transition ${
                  cooldown || busy || !selectedTitleId
                    ? 'cursor-not-allowed bg-fusion-primary/40 text-white/50'
                    : 'bg-fusion-primary text-white shadow-lg shadow-fusion-primary/30 hover:bg-fusion-primary/90'
                }`}
              >
                {busy ? 'Installing...' : cooldown ? 'Confirming in 1.5s...' : 'Confirm & Install'}
              </button>
            )}
          </div>
        </footer>
      </section>
    </div>
  );
}
