'use client';

import { useEffect, useRef, useState } from 'react';
import {
  AlertCircle,
  ArrowLeft,
  ChevronRight,
  Cpu,
  ExternalLink,
  HardDrive,
  ShieldAlert,
  X,
} from 'lucide-react';
import { api } from '@/lib/api';
import { getUiText } from '@/lib/i18n';
import { useLauncherStore } from '@/stores/launcherStore';
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

function extractDownloadHosts(game: ManifestGame, torrentLabel: string, fallbackLabel: string): string[] {
  const hosts: string[] = [];
  if (game.assets.heavy_rom_magnet) {
    hosts.push(torrentLabel);
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
  return hosts.length > 0 ? hosts : [fallbackLabel];
}

export function DeepLinkConfirmModal({ request, onClose }: DeepLinkConfirmModalProps) {
  const language = useLauncherStore((s) => s.settings.language);
  const t = getUiText(language).deepLinkModal;

  const [step, setStep] = useState<1 | 2>(1);
  const [cooldown, setCooldown] = useState(true);
  const [busy, setBusy] = useState(false);
  const [manifest, setManifest] = useState<Manifest | null>(null);
  const [selectedTitleId, setSelectedTitleId] = useState<string>('');
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const activeRequestIdRef = useRef<string>(request.id);

  // Invalidate in-flight operations when request changes
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

    return () => {
      clearTimeout(timer);
    };
  }, [request.id]);

  // Clean up on unmount so any in-flight fetch is completely dropped
  useEffect(() => {
    return () => {
      activeRequestIdRef.current = '';
    };
  }, []);

  // Reset 1.5s countdown on step change
  useEffect(() => {
    setCooldown(true);
    const timer = setTimeout(() => {
      setCooldown(false);
    }, 1500);

    return () => clearTimeout(timer);
  }, [step]);

  const handleClose = () => {
    activeRequestIdRef.current = '';
    onClose();
  };

  const handleConfirmSource = async () => {
    const currentId = request.id;
    if (cooldown || busy) return;
    if (!activeRequestIdRef.current || activeRequestIdRef.current !== currentId) return;
    setBusy(true);
    setErrorMessage(null);
    try {
      if (request.onConfirmSource) {
        await request.onConfirmSource();
      } else if (request.onConfirm) {
        await request.onConfirm();
      }
      handleClose();
    } catch (err) {
      if (activeRequestIdRef.current === currentId) {
        setErrorMessage(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (activeRequestIdRef.current === currentId) {
        setBusy(false);
      }
    }
  };

  const handleFetchManifestStep1 = async () => {
    const currentId = request.id;
    if (cooldown || busy) return;
    if (!activeRequestIdRef.current || activeRequestIdRef.current !== currentId) return;
    setBusy(true);
    setErrorMessage(null);
    try {
      const data = await api.fetchManifest(request.url);
      // Strictly ignore result if dialog was closed or replaced during the network request
      if (!activeRequestIdRef.current || activeRequestIdRef.current !== currentId) return;
      if (!data.games || data.games.length === 0) {
        throw new Error(t.noGamesError);
      }
      setManifest(data);
      setSelectedTitleId(data.games[0].title_id);
      setStep(2);
    } catch (err) {
      if (activeRequestIdRef.current === currentId) {
        setErrorMessage(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (activeRequestIdRef.current === currentId) {
        setBusy(false);
      }
    }
  };

  const handleConfirmInstallStep2 = async () => {
    const currentId = request.id;
    if (cooldown || busy || !selectedTitleId) return;
    if (!activeRequestIdRef.current || activeRequestIdRef.current !== currentId) return;
    setBusy(true);
    setErrorMessage(null);
    try {
      if (request.onConfirmInstall) {
        await request.onConfirmInstall(selectedTitleId);
      } else if (request.onConfirm) {
        await request.onConfirm();
      }
      handleClose();
    } catch (err) {
      if (activeRequestIdRef.current === currentId) {
        setErrorMessage(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (activeRequestIdRef.current === currentId) {
        setBusy(false);
      }
    }
  };

  const selectedGame = manifest?.games.find((g) => g.title_id === selectedTitleId) ?? manifest?.games[0];
  const downloadHosts = selectedGame
    ? extractDownloadHosts(selectedGame, t.torrentMagnet, t.noRemoteSource)
    : [];

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
                  ? t.connectSourceTitle
                  : step === 1
                  ? t.inspectManifestTitle
                  : t.confirmInstallTitle}
              </h2>
              {request.type === 'install' && (
                <span className="rounded-full border border-white/20 px-2 py-0.5 text-[11px] font-medium text-white/60">
                  {t.stepIndicator(step, 2)}
                </span>
              )}
            </div>
            <p className="mt-1 text-sm text-white/70">
              {request.type === 'addSource'
                ? t.connectSourceDesc
                : step === 1
                ? t.inspectManifestDesc
                : t.confirmInstallDesc}
            </p>
          </div>
          <button
            onClick={handleClose}
            className="grid h-9 w-9 place-items-center rounded-md border border-white/10 text-white/60 transition hover:text-white"
            title={t.cancel}
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        {errorMessage && (
          <div className="mt-4 flex items-start gap-2.5 rounded-lg border border-red-500/30 bg-red-500/10 p-3 text-sm text-red-200">
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-red-400" />
            <div className="min-w-0 flex-1 select-text break-words">{errorMessage}</div>
          </div>
        )}

        {/* STEP 1: AddSource OR Initial Manifest URL Verification */}
        {step === 1 && (
          <div className="mt-5 space-y-3">
            <div className="rounded-lg border border-white/10 bg-black/40 p-3">
              <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-white/50">
                <ExternalLink className="h-3.5 w-3.5" />
                {t.targetHost}
              </div>
              <div className="mt-1 text-base font-bold text-white selection:bg-white/20">
                {request.host}
              </div>
            </div>

            <div className="rounded-lg border border-white/10 bg-black/40 p-3">
              <div className="text-xs font-semibold uppercase tracking-wider text-white/50">
                {t.fullResource}
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
                  {t.selectGame(manifest.games.length)}
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
                  {t.gameTitle}
                </div>
                <div className="mt-1 truncate text-sm font-bold text-white" title={selectedGame.title}>
                  {selectedGame.title}
                </div>
              </div>

              <div className="rounded-lg border border-white/10 bg-black/40 p-3">
                <div className="text-xs font-semibold uppercase tracking-wider text-white/50">
                  {t.platform}
                </div>
                <div className="mt-1 text-sm font-bold uppercase text-white">
                  {selectedGame.platform}
                </div>
              </div>
            </div>

            <div className="rounded-lg border border-white/10 bg-black/40 p-3">
              <div className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wider text-white/50">
                <HardDrive className="h-3.5 w-3.5" />
                {t.downloadHosts}
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
                <Cpu className="h-3.5 w-3.5 text-fusion-primary" />
                {t.emulatorNotice}
              </div>
              <div className="mt-1 text-xs text-white/70">
                {t.emulatorNoticeDesc}
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
              {t.back}
            </button>
          ) : (
            <button
              type="button"
              onClick={handleClose}
              className="rounded-lg border border-white/15 px-4 py-2 text-sm font-medium text-white/80 transition hover:bg-white/5 hover:text-white"
            >
              {t.cancel}
            </button>
          )}

          <div className="flex items-center gap-3">
            {step === 2 && (
              <button
                type="button"
                onClick={handleClose}
                className="rounded-lg border border-white/15 px-4 py-2 text-sm font-medium text-white/80 transition hover:bg-white/5 hover:text-white"
              >
                {t.cancel}
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
                {busy ? t.processing : cooldown ? t.confirmCooldown : t.confirmProceed}
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
                {busy ? t.fetching : cooldown ? t.inspectCooldown : t.inspectManifest}
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
                {busy ? t.installing : cooldown ? t.confirmCooldown : t.confirmInstall}
              </button>
            )}
          </div>
        </footer>
      </section>
    </div>
  );
}
