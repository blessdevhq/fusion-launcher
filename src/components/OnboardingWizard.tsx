'use client';

import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import {
  CheckCircle2,
  ChevronRight,
  DatabaseZap,
  Download,
  FolderOpen,
  Gamepad2,
  Globe2,
  Link2,
  Loader2,
  Play,
  Rocket,
  ShieldAlert,
  Wrench
} from 'lucide-react';
import { ManifestInstallCard } from '@/components/settings/ManifestInstallCard';
import { useI18n } from '@/components/I18nProvider';
import { api } from '@/lib/api';
import { displayProductText } from '@/lib/brandText';
import { LOCALES, type Locale } from '@/lib/i18n';
import { isTauriRuntime } from '@/lib/runtime';
import { saveSettings, type AppSettings, type MetadataOnboardingStrategy } from '@/lib/settings';
import { sourceTrustLabel, unknownSourcePrompt } from '@/lib/sourceTrust';
import { PLATFORM_LABELS, type Platform } from '@/types/platform';
import type {
  CatalogGame,
  OnboardingState,
  PlatformSetupProfile,
  RepositoryPreview,
  ScreenScraperStatus,
  SteamGridDbStatus,
  TorrentDownloadRecord
} from '@/types/repository';
import type { EmulatorStatus } from '@/types/emulatorProfile';

type WizardStep = 'welcome' | 'source' | 'metadata' | 'emulator' | 'ready';

interface OnboardingWizardProps {
  state: OnboardingState | null;
  catalog: CatalogGame[];
  settings: AppSettings;
  initialMessage: string | null;
  onSettingsChange: (settings: AppSettings) => void;
  onReload: () => Promise<void>;
}

const STEP_ORDER: WizardStep[] = ['welcome', 'source', 'metadata', 'emulator', 'ready'];

export function OnboardingWizard({
  state,
  catalog,
  settings,
  initialMessage,
  onSettingsChange,
  onReload
}: OnboardingWizardProps) {
  const { locale, t } = useI18n();
  const [activeStep, setActiveStep] = useState<WizardStep>('welcome');
  const [repoUrl, setRepoUrl] = useState('');
  const [preview, setPreview] = useState<RepositoryPreview | null>(null);
  const [builtInPreview, setBuiltInPreview] = useState<RepositoryPreview | null>(null);
  const [profiles, setProfiles] = useState<PlatformSetupProfile[]>([]);
  const [emulatorStatuses, setEmulatorStatuses] = useState<Record<string, EmulatorStatus>>({});
  const [emulatorPaths, setEmulatorPaths] = useState<Record<string, string>>({});
  const [demoDownload, setDemoDownload] = useState<TorrentDownloadRecord | null>(null);
  const [metadataStrategy, setMetadataStrategy] = useState<MetadataOnboardingStrategy>(settings.metadataOnboarding.strategy);
  const [scraperStatus, setScraperStatus] = useState<ScreenScraperStatus | null>(null);
  const [scraperSsid, setScraperSsid] = useState('');
  const [scraperPassword, setScraperPassword] = useState('');
  const [scraperRegion, setScraperRegion] = useState('auto');
  const [steamgriddbStatus, setSteamgriddbStatus] = useState<SteamGridDbStatus | null>(null);
  const [steamgriddbKey, setSteamgriddbKey] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(initialMessage);

  const repositoryReady = Boolean(state?.repositoriesConfigured);
  const catalogReady = catalog.length > 0;
  const metadataReady = settings.metadataOnboarding.complete;

  const demoGame = useMemo(
    () => catalog.find((game) => game.id.includes('fusion_launcher_nes_smoke')) ?? catalog.find((game) => game.platform === 'nes') ?? catalog[0] ?? null,
    [catalog]
  );

  const profileById = useMemo(() => new Map(profiles.map((profile) => [profile.id, profile])), [profiles]);

  const requiredProfiles = useMemo(() => {
    const selected = new Map<string, PlatformSetupProfile>();
    const firstProfileByPlatform = new Map<string, PlatformSetupProfile>();

    for (const profile of profiles) {
      if (!firstProfileByPlatform.has(profile.platform)) {
        firstProfileByPlatform.set(profile.platform, profile);
      }
    }

    for (const game of catalog) {
      const profile = (game.setupProfileId ? profileById.get(game.setupProfileId) : undefined)
        ?? firstProfileByPlatform.get(game.platform);
      if (profile) selected.set(profile.id, profile);
    }

    return Array.from(selected.values());
  }, [catalog, profileById, profiles]);

  const emulatorReady = requiredProfiles.length > 0
    ? requiredProfiles.some((profile) => emulatorStatuses[profile.platform]?.installed)
    : Boolean(state?.emulatorsConfigured);
  const onboardingReady = repositoryReady && catalogReady && metadataReady && emulatorReady;
  const demoDownloaded = demoDownload?.status === 'completed';
  const playable = Boolean(onboardingReady && demoDownloaded && demoGame);

  useEffect(() => {
    setMessage(initialMessage);
  }, [initialMessage]);

  useEffect(() => {
    setMetadataStrategy(settings.metadataOnboarding.strategy);
  }, [settings.metadataOnboarding.strategy]);

  useEffect(() => {
    if (!repositoryReady) {
      setActiveStep((current) => current === 'ready' || current === 'emulator' || current === 'metadata' ? 'source' : current);
      return;
    }
    if (!metadataReady) {
      setActiveStep((current) => current === 'ready' || current === 'emulator' ? 'metadata' : current);
    }
  }, [metadataReady, repositoryReady]);

  useEffect(() => {
    let cancelled = false;

    async function loadProfiles() {
      try {
        const nextProfiles = await api.listPlatformSetupProfiles();
        if (!cancelled) setProfiles(nextProfiles);
      } catch (error) {
        if (!cancelled) setMessage(error instanceof Error ? error.message : String(error));
      }
    }

    void loadProfiles();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;

    async function loadSetupAndMetadataState() {
      try {
        const platforms = Array.from(new Set(requiredProfiles.map((profile) => profile.platform)));
        const [statuses, download, screenStatus, steamStatus] = await Promise.all([
          Promise.all(platforms.map(async (platform) => [platform, await api.getEmulatorStatus(platform)] as const)),
          demoGame ? api.getGameDownload(demoGame.id) : Promise.resolve(null),
          api.getScreenscraperStatus(),
          api.getSteamgriddbStatus()
        ]);

        if (!cancelled) {
          setEmulatorStatuses(Object.fromEntries(statuses));
          setDemoDownload(download);
          setScraperStatus(screenStatus);
          setScraperSsid(screenStatus.ssid ?? '');
          setScraperRegion(screenStatus.region ?? 'auto');
          setSteamgriddbStatus(steamStatus);
        }
      } catch (error) {
        if (!cancelled) setMessage(error instanceof Error ? error.message : String(error));
      }
    }

    void loadSetupAndMetadataState();
    return () => {
      cancelled = true;
    };
  }, [demoGame?.id, requiredProfiles]);

  const refreshSetupState = async (profilesToRefresh = requiredProfiles, gameId = demoGame?.id) => {
    const platforms = Array.from(new Set(profilesToRefresh.map((profile) => profile.platform)));
    const [statuses, download] = await Promise.all([
      Promise.all(platforms.map(async (platform) => [platform, await api.getEmulatorStatus(platform)] as const)),
      gameId ? api.getGameDownload(gameId) : Promise.resolve(null)
    ]);

    setEmulatorStatuses((current) => ({ ...current, ...Object.fromEntries(statuses) }));
    setDemoDownload(download);
  };

  const changeLanguage = async (language: Locale) => {
    setBusy('language');
    setMessage(null);
    try {
      const nextSettings = await saveSettings({ ...settings, language });
      onSettingsChange(nextSettings);
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
      setActiveStep('metadata');
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
        const confirmed = window.confirm(unknownSourcePrompt(nextPreview, locale));
        if (!confirmed) return;
      }
      await api.connectRepository(repoUrl.trim());
      await onReload();
      setActiveStep('metadata');
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const connectRepositoryFile = async () => {
    if (!isTauriRuntime()) {
      setMessage(t.onboarding.messages.localJsonDesktopOnly);
      return;
    }
    setBusy('connect-file');
    setMessage(null);
    try {
      const selected = await open({
        title: t.dashboard.messages.selectSourceJson,
        multiple: false,
        directory: false,
        filters: [{ name: t.dashboard.messages.repositoryJson, extensions: ['json'] }]
      });
      if (typeof selected !== 'string') return;
      const nextPreview = await api.previewRepositoryFile(selected);
      if (nextPreview.trustLevel === 'unknown') {
        const confirmed = window.confirm(unknownSourcePrompt(nextPreview, locale));
        if (!confirmed) return;
      }
      await api.connectRepositoryFile(selected);
      await onReload();
      setActiveStep('metadata');
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const browseForEmulator = async (profile: PlatformSetupProfile) => {
    if (!isTauriRuntime()) {
      setMessage(t.onboarding.messages.fileBrowseDesktopOnly);
      return;
    }
    setBusy(`browse:${profile.id}`);
    setMessage(null);
    try {
      const selected = await open({
        title: t.settings.emulators.pickerTitle(platformLabel(profile.platform)),
        multiple: false,
        directory: false,
        defaultPath: emulatorPaths[profile.id] || emulatorStatuses[profile.platform]?.exePath || undefined,
        filters: [{ name: t.gameDetails.selectExecutable, extensions: ['exe'] }]
      });
      if (typeof selected === 'string') {
        setEmulatorPaths((current) => ({ ...current, [profile.id]: selected }));
      }
    } catch (error) {
      setMessage(t.onboarding.messages.browseError(error));
    } finally {
      setBusy(null);
    }
  };

  const saveManualEmulator = async (profile: PlatformSetupProfile) => {
    const path = emulatorPaths[profile.id]?.trim();
    if (!path) return;
    setBusy(`manual:${profile.id}`);
    setMessage(null);
    try {
      await api.selectProfileEmulator(profile.id, path);
      await refreshSetupState([profile]);
      setActiveStep('ready');
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const saveMetadataOnboarding = async () => {
    setBusy('metadata');
    setMessage(null);
    try {
      if (metadataStrategy === 'screenscraper') {
        if (scraperSsid.trim() || scraperPassword.trim()) {
          const nextStatus = await api.saveScreenscraperCredentials(scraperSsid, scraperPassword, scraperRegion);
          setScraperStatus(nextStatus);
          setScraperSsid(nextStatus.ssid ?? scraperSsid.trim());
          setScraperPassword('');
          setScraperRegion(nextStatus.region ?? 'auto');
        }
        if (steamgriddbKey.trim()) {
          const nextSteamStatus = await api.saveSteamgriddbKey(steamgriddbKey);
          setSteamgriddbStatus(nextSteamStatus);
          setSteamgriddbKey('');
        }
      }

      const nextSettings = await saveSettings({
        ...settings,
        metadataOnboarding: {
          complete: true,
          strategy: metadataStrategy
        }
      });
      onSettingsChange(nextSettings);
      setActiveStep('emulator');
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const installProfileEmulator = async (profile: PlatformSetupProfile) => {
    setBusy(`install:${profile.id}`);
    setMessage(null);
    try {
      await api.installProfileEmulator(profile.id);
      await refreshSetupState([profile]);
      setActiveStep('ready');
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
      await refreshSetupState(requiredProfiles, demoGame.id);
      setMessage(t.onboarding.messages.playableReady);
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
      if (!targetGame) throw new Error(t.onboarding.messages.noPlayableDemo);
      await api.launchGame(targetGame.id);
      setMessage(t.onboarding.messages.launched(targetGame.title));
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const openLauncher = async () => {
    setBusy('open-launcher');
    setMessage(null);
    try {
      await onReload();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const nextStep = () => {
    if (activeStep === 'welcome') {
      setActiveStep(repositoryReady ? metadataReady ? 'emulator' : 'metadata' : 'source');
      return;
    }
    if (activeStep === 'source' && repositoryReady) {
      setActiveStep('metadata');
      return;
    }
    if (activeStep === 'metadata' && metadataReady) {
      setActiveStep('emulator');
      return;
    }
    if (activeStep === 'emulator' && emulatorReady) {
      setActiveStep('ready');
    }
  };

  return (
    <main className="rh-onboarding-screen bg-fusion-bg text-white" data-testid="onboarding-screen">
      <section className="rh-onboarding-shell" data-testid="onboarding-stepper">
        <aside className="rh-onboarding-sidebar">
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src="/fusion/logo-lockup.png" alt="fusion" className="rh-onboarding-logo" />
          <div>
            <div className="rh-onboarding-eyebrow">{t.onboarding.firstRun}</div>
            <h1 className="rh-onboarding-title">{t.onboarding.title}</h1>
            <p className="rh-onboarding-copy">{t.onboarding.copy}</p>
          </div>
          <nav className="rh-onboarding-steps" aria-label={t.onboarding.stepperLabel}>
            {STEP_ORDER.map((step, index) => {
              const active = activeStep === step;
              const done = stepDone(step, repositoryReady, catalogReady, metadataReady, emulatorReady, onboardingReady);
              const disabled = stepDisabled(step, repositoryReady, catalogReady, metadataReady);
              return (
                <button
                  key={step}
                  type="button"
                  onClick={() => setActiveStep(step)}
                  disabled={disabled}
                  className={`rh-onboarding-step ${active ? 'rh-onboarding-step-active' : ''}`}
                  data-testid={`onboarding-nav-${step}`}
                >
                  <span className={`rh-onboarding-step-index ${done ? 'rh-onboarding-step-done' : ''}`}>
                    {done ? <CheckCircle2 className="h-4 w-4" /> : index + 1}
                  </span>
                  <span>
                    <span className="rh-onboarding-step-title">{t.onboarding.steps[step]}</span>
                    <span className="rh-onboarding-step-detail">{stepDetail(step, repositoryReady, catalogReady, metadataReady, emulatorReady, t)}</span>
                  </span>
                </button>
              );
            })}
          </nav>
        </aside>

        <section className="rh-onboarding-panel" data-testid={`onboarding-step-${activeStep}`}>
          {activeStep === 'welcome' && (
            <WelcomeStep
              language={settings.language}
              busy={busy === 'language'}
              onLanguageChange={changeLanguage}
              onNext={nextStep}
            />
          )}

          {activeStep === 'source' && (
            <SourceStep
              repositoryReady={repositoryReady}
              catalogCount={state?.catalogCount ?? catalog.length}
              repoUrl={repoUrl}
              preview={preview}
              builtInPreview={builtInPreview}
              busy={busy}
              onRepoUrlChange={(value) => {
                setRepoUrl(value);
                setPreview(null);
              }}
              onPreviewRepository={previewRepository}
              onPreviewBuiltInRepository={previewBuiltInRepository}
              onConnectBuiltInRepository={connectBuiltInRepository}
              onConnectRepository={connectRepository}
              onConnectRepositoryFile={connectRepositoryFile}
              onManifestInstalled={onReload}
              onNext={nextStep}
            />
          )}

          {activeStep === 'metadata' && (
            <MetadataStep
              strategy={metadataStrategy}
              scraperStatus={scraperStatus}
              scraperSsid={scraperSsid}
              scraperPassword={scraperPassword}
              scraperRegion={scraperRegion}
              steamgriddbStatus={steamgriddbStatus}
              steamgriddbKey={steamgriddbKey}
              busy={busy === 'metadata'}
              onStrategyChange={setMetadataStrategy}
              onSsidChange={setScraperSsid}
              onPasswordChange={setScraperPassword}
              onRegionChange={setScraperRegion}
              onSteamgriddbKeyChange={setSteamgriddbKey}
              onNext={saveMetadataOnboarding}
            />
          )}

          {activeStep === 'emulator' && (
            <EmulatorStep
              repositoryReady={repositoryReady}
              catalogReady={catalogReady}
              profiles={requiredProfiles}
              statuses={emulatorStatuses}
              paths={emulatorPaths}
              busy={busy}
              onPathChange={(profileId, value) => setEmulatorPaths((current) => ({ ...current, [profileId]: value }))}
              onBrowse={browseForEmulator}
              onInstall={installProfileEmulator}
              onSaveManual={saveManualEmulator}
              onNext={nextStep}
            />
          )}

          {activeStep === 'ready' && (
            <ReadyStep
              repositoryReady={repositoryReady}
              catalogReady={catalogReady}
              metadataReady={metadataReady}
              emulatorReady={emulatorReady}
              onboardingReady={onboardingReady}
              demoGame={demoGame}
              demoDownloaded={demoDownloaded}
              playable={playable}
              busy={busy}
              onOpenLauncher={openLauncher}
              onDownloadDemo={downloadFirstGame}
              onPlayDemo={playDemo}
              onBackToSource={() => setActiveStep('source')}
              onBackToMetadata={() => setActiveStep('metadata')}
              onBackToEmulator={() => setActiveStep('emulator')}
            />
          )}

          {message && (
            <div className="rh-onboarding-message">
              {message}
            </div>
          )}
        </section>
      </section>
    </main>
  );
}

function WelcomeStep({
  language,
  busy,
  onLanguageChange,
  onNext
}: {
  language: Locale;
  busy: boolean;
  onLanguageChange: (language: Locale) => Promise<void>;
  onNext: () => void;
}) {
  const { t } = useI18n();

  return (
    <div className="rh-onboarding-content">
      <StepHeader icon={<Rocket className="h-5 w-5" />} title={t.onboarding.welcome.title} copy={t.onboarding.welcome.copy} />
      <div className="rh-onboarding-card">
        <div className="flex items-start gap-3">
          <Globe2 className="mt-1 h-5 w-5 text-fusion-accent" />
          <div className="min-w-0 flex-1">
            <div className="text-sm font-black text-white/90">{t.language.label}</div>
            <p className="mt-1 text-sm leading-6 text-white/52">{t.language.description}</p>
            <select
              value={language}
              onChange={(event) => void onLanguageChange(event.target.value as Locale)}
              disabled={busy}
              className="mt-4 h-11 w-full rounded-sm border border-white/10 bg-black/40 px-3 text-sm font-semibold text-white/80 outline-none transition focus:border-white/60"
              data-testid="onboarding-language"
            >
              {LOCALES.map((item) => (
                <option key={item} value={item}>{t.language.options[item]}</option>
              ))}
            </select>
          </div>
        </div>
      </div>
      <div className="rh-onboarding-actions">
        <button type="button" onClick={onNext} className="rh-primary-action" data-testid="onboarding-next">
          {t.onboarding.continue}
          <ChevronRight className="h-4 w-4" />
        </button>
      </div>
    </div>
  );
}

function SourceStep({
  repositoryReady,
  catalogCount,
  repoUrl,
  preview,
  builtInPreview,
  busy,
  onRepoUrlChange,
  onPreviewRepository,
  onPreviewBuiltInRepository,
  onConnectBuiltInRepository,
  onConnectRepository,
  onConnectRepositoryFile,
  onManifestInstalled,
  onNext
}: {
  repositoryReady: boolean;
  catalogCount: number;
  repoUrl: string;
  preview: RepositoryPreview | null;
  builtInPreview: RepositoryPreview | null;
  busy: string | null;
  onRepoUrlChange: (value: string) => void;
  onPreviewRepository: () => Promise<void>;
  onPreviewBuiltInRepository: () => Promise<void>;
  onConnectBuiltInRepository: () => Promise<void>;
  onConnectRepository: () => Promise<void>;
  onConnectRepositoryFile: () => Promise<void>;
  onManifestInstalled: () => Promise<void>;
  onNext: () => void;
}) {
  const { t } = useI18n();

  return (
    <div className="rh-onboarding-content">
      <StepHeader icon={<Link2 className="h-5 w-5" />} title={t.onboarding.sourceStep.title} copy={t.onboarding.sourceStep.copy} />
      <div className="grid gap-4 lg:grid-cols-[0.95fr_1.05fr]">
        <section className="rh-onboarding-card" data-testid="onboarding-demo-card">
          <div className="flex items-start justify-between gap-3">
            <div>
              <h2 className="text-xl font-black">{t.onboarding.builtInDemo}</h2>
              <p className="mt-2 text-sm leading-6 text-white/52">{t.onboarding.sourceStep.demoCopy}</p>
            </div>
            <StatusPill done={repositoryReady} label={repositoryReady ? t.common.ready : t.common.missing} />
          </div>
          <div className="mt-5 flex flex-wrap gap-2">
            <button type="button" onClick={onConnectBuiltInRepository} disabled={busy !== null} className="rh-primary-action" data-testid="onboarding-use-demo">
              {busy === 'builtin-connect' ? <Loader2 className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
              {t.onboarding.sourceStep.useDemo}
            </button>
            <button type="button" onClick={onPreviewBuiltInRepository} disabled={busy !== null} className="rh-mini-action">
              {busy === 'builtin-preview' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldAlert className="h-3.5 w-3.5" />}
              {t.common.check}
            </button>
          </div>
          {builtInPreview && <OnboardingSourcePreview preview={builtInPreview} compact />}
        </section>

        <section className="rh-onboarding-card" data-testid="onboarding-source-card">
          <h2 className="text-xl font-black">{t.onboarding.sourceCard.title}</h2>
          <p className="mt-2 text-sm leading-6 text-white/52">{t.onboarding.sourceCard.copy}</p>
          <label className="mt-5 block">
            <span className="mb-2 block text-xs font-semibold text-white/52">{t.onboarding.sourceCard.sourceUrl}</span>
            <input
              value={repoUrl}
              onChange={(event) => onRepoUrlChange(event.target.value)}
              className="h-11 w-full rounded-sm border border-white/10 bg-black/40 px-3 text-sm outline-none focus:border-fusion-accent/70"
              placeholder="https://blessdevhq.github.io/fusion-launcher/source-library-template/repository.json"
              data-testid="onboarding-source-url"
            />
          </label>
          {preview && <OnboardingSourcePreview preview={preview} />}
          <div className="mt-4 flex flex-wrap gap-2">
            <button type="button" onClick={onPreviewRepository} disabled={busy !== null || !repoUrl.trim()} className="rh-mini-action">
              {busy === 'preview' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldAlert className="h-3.5 w-3.5" />}
              {t.common.check}
            </button>
            <button type="button" onClick={onConnectRepository} disabled={busy !== null || !repoUrl.trim()} className="rh-mini-action">
              {busy === 'connect' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Link2 className="h-3.5 w-3.5" />}
              {t.common.connect}
            </button>
            <button type="button" onClick={onConnectRepositoryFile} disabled={busy !== null} className="rh-mini-action">
              {busy === 'connect-file' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <FolderOpen className="h-3.5 w-3.5" />}
              {t.onboarding.sourceCard.jsonFile}
            </button>
          </div>
        </section>
      </div>
      <div className="mt-4">
        <ManifestInstallCard onInstalled={onManifestInstalled} />
      </div>
      <div className="rh-onboarding-checkline">
        <SetupItem done={repositoryReady} title={t.onboarding.setup.source} detail={repositoryReady ? t.onboarding.sourceStep.connected(catalogCount) : t.onboarding.sourceStep.notConnected} />
      </div>
      <div className="rh-onboarding-actions">
        <button type="button" onClick={onNext} disabled={!repositoryReady} className="rh-primary-action" data-testid="onboarding-next-source">
          {t.onboarding.continue}
          <ChevronRight className="h-4 w-4" />
        </button>
      </div>
    </div>
  );
}

function MetadataStep({
  strategy,
  scraperStatus,
  scraperSsid,
  scraperPassword,
  scraperRegion,
  steamgriddbStatus,
  steamgriddbKey,
  busy,
  onStrategyChange,
  onSsidChange,
  onPasswordChange,
  onRegionChange,
  onSteamgriddbKeyChange,
  onNext
}: {
  strategy: MetadataOnboardingStrategy;
  scraperStatus: ScreenScraperStatus | null;
  scraperSsid: string;
  scraperPassword: string;
  scraperRegion: string;
  steamgriddbStatus: SteamGridDbStatus | null;
  steamgriddbKey: string;
  busy: boolean;
  onStrategyChange: (strategy: MetadataOnboardingStrategy) => void;
  onSsidChange: (value: string) => void;
  onPasswordChange: (value: string) => void;
  onRegionChange: (value: string) => void;
  onSteamgriddbKeyChange: (value: string) => void;
  onNext: () => Promise<void>;
}) {
  const { t } = useI18n();
  const strategies: MetadataOnboardingStrategy[] = ['source', 'screenscraper', 'manual'];

  return (
    <div className="rh-onboarding-content">
      <StepHeader icon={<DatabaseZap className="h-5 w-5" />} title={t.onboarding.metadataStep.title} copy={t.onboarding.metadataStep.copy} />

      <section className="rh-onboarding-card" data-testid="onboarding-metadata-sources">
        <h2 className="text-lg font-black text-white/90">{t.onboarding.metadataStep.sourcesTitle}</h2>
        <div className="mt-4 grid gap-3 md:grid-cols-2">
          {t.onboarding.metadataStep.sources.map((source) => (
            <div key={source.title} className="rounded-sm border border-white/10 bg-black/20 p-3">
              <div className="text-sm font-black text-white/84">{source.title}</div>
              <p className="mt-1 text-xs leading-5 text-white/46">{source.copy}</p>
            </div>
          ))}
        </div>
      </section>

      <section className="rh-onboarding-card" data-testid="onboarding-metadata-strategy">
        <h2 className="text-lg font-black text-white/90">{t.onboarding.metadataStep.strategyTitle}</h2>
        <div className="mt-4 grid gap-3">
          {strategies.map((item) => {
            const selected = strategy === item;
            const option = t.onboarding.metadataStep.strategies[item];
            return (
              <button
                key={item}
                type="button"
                onClick={() => onStrategyChange(item)}
                className={`rounded-sm border p-4 text-left transition ${selected ? 'border-fusion-accent/45 bg-fusion-accent/10' : 'border-white/10 bg-black/18 hover:bg-white/[0.06]'}`}
                data-testid={`onboarding-metadata-strategy-${item}`}
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span className="text-sm font-black text-white/90">{option.title}</span>
                  {option.badge && (
                    <span className="rounded-lg border border-fusion-accent/24 bg-fusion-accent/10 px-2 py-1 text-[10px] font-semibold text-fusion-accent">
                      {option.badge}
                    </span>
                  )}
                </div>
                <p className="mt-2 text-xs leading-5 text-white/48">{option.copy}</p>
              </button>
            );
          })}
        </div>
      </section>

      {strategy === 'screenscraper' && (
        <section className="rh-onboarding-card" data-testid="onboarding-metadata-credentials">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <h2 className="text-lg font-black text-white/90">{t.onboarding.metadataStep.credentialsTitle}</h2>
              <p className="mt-2 text-sm leading-6 text-white/52">{t.onboarding.metadataStep.credentialsCopy}</p>
            </div>
            <StatusPill done={Boolean(scraperStatus?.configured || steamgriddbStatus?.configured)} label={scraperStatus?.configured || steamgriddbStatus?.configured ? t.common.ready : t.onboarding.metadataStep.optional} />
          </div>
          <div className="mt-4 grid gap-3 lg:grid-cols-2">
            <label className="block text-xs font-semibold text-white/58">
              {t.onboarding.metadataStep.ssid}
              <input
                value={scraperSsid}
                onChange={(event) => onSsidChange(event.target.value)}
                className="mt-2 h-10 w-full rounded-sm border border-white/10 bg-black/40 px-3 text-sm text-white/82 outline-none transition focus:border-fusion-accent/70"
                placeholder="username"
                autoComplete="username"
              />
            </label>
            <label className="block text-xs font-semibold text-white/58">
              {t.onboarding.metadataStep.password}
              <input
                value={scraperPassword}
                onChange={(event) => onPasswordChange(event.target.value)}
                className="mt-2 h-10 w-full rounded-sm border border-white/10 bg-black/40 px-3 text-sm text-white/82 outline-none transition focus:border-fusion-accent/70"
                type="password"
                autoComplete="current-password"
              />
            </label>
            <label className="block text-xs font-semibold text-white/58">
              {t.onboarding.metadataStep.region}
              <select
                value={scraperRegion}
                onChange={(event) => onRegionChange(event.target.value)}
                className="mt-2 h-10 w-full rounded-sm border border-white/10 bg-black/40 px-3 text-sm text-white/82 outline-none transition focus:border-fusion-accent/70"
              >
                <option value="auto">{t.onboarding.metadataStep.regions.auto}</option>
                <option value="eu">{t.onboarding.metadataStep.regions.eu}</option>
                <option value="us">{t.onboarding.metadataStep.regions.us}</option>
                <option value="jp">{t.onboarding.metadataStep.regions.jp}</option>
              </select>
            </label>
            <label className="block text-xs font-semibold text-white/58">
              {t.onboarding.metadataStep.steamgriddbKey}
              <input
                value={steamgriddbKey}
                onChange={(event) => onSteamgriddbKeyChange(event.target.value)}
                className="mt-2 h-10 w-full rounded-sm border border-white/10 bg-black/40 px-3 text-sm text-white/82 outline-none transition focus:border-fusion-accent/70"
                type="password"
                autoComplete="off"
                placeholder={t.onboarding.metadataStep.steamgriddbPlaceholder}
              />
            </label>
          </div>
        </section>
      )}

      <div className="rh-onboarding-actions">
        <button type="button" onClick={() => void onNext()} disabled={busy} className="rh-primary-action" data-testid="onboarding-next-metadata">
          {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <DatabaseZap className="h-4 w-4" />}
          {t.onboarding.metadataStep.save}
        </button>
      </div>
    </div>
  );
}

function EmulatorStep({
  repositoryReady,
  catalogReady,
  profiles,
  statuses,
  paths,
  busy,
  onPathChange,
  onBrowse,
  onInstall,
  onSaveManual,
  onNext
}: {
  repositoryReady: boolean;
  catalogReady: boolean;
  profiles: PlatformSetupProfile[];
  statuses: Record<string, EmulatorStatus>;
  paths: Record<string, string>;
  busy: string | null;
  onPathChange: (profileId: string, value: string) => void;
  onBrowse: (profile: PlatformSetupProfile) => Promise<void>;
  onInstall: (profile: PlatformSetupProfile) => Promise<void>;
  onSaveManual: (profile: PlatformSetupProfile) => Promise<void>;
  onNext: () => void;
}) {
  const { t } = useI18n();
  const anyReady = profiles.some((profile) => statuses[profile.platform]?.installed);

  return (
    <div className="rh-onboarding-content">
      <StepHeader icon={<Gamepad2 className="h-5 w-5" />} title={t.onboarding.emulatorStep.title} copy={t.onboarding.emulatorStep.copy} />
      {!repositoryReady && (
        <div className="rh-onboarding-card rh-onboarding-card-warning">
          {t.onboarding.emulatorStep.needsSource}
        </div>
      )}
      {repositoryReady && !catalogReady && (
        <div className="rh-onboarding-card rh-onboarding-card-warning">
          {t.onboarding.emulatorStep.noCatalog}
        </div>
      )}
      {repositoryReady && catalogReady && profiles.length === 0 && (
        <div className="rh-onboarding-card rh-onboarding-card-warning">
          {t.onboarding.emulatorStep.noProfiles}
        </div>
      )}
      <div className="grid gap-3" data-testid="onboarding-emulator-list">
        {profiles.map((profile) => {
          const status = statuses[profile.platform];
          const installed = Boolean(status?.installed);
          const manual = profile.emulator.installMode === 'manual';
          const rowBusy = busy === `install:${profile.id}` || busy === `manual:${profile.id}` || busy === `browse:${profile.id}`;
          const path = paths[profile.id] ?? status?.exePath ?? '';

          return (
            <article key={profile.id} className={`rh-onboarding-emulator-row ${installed ? 'rh-onboarding-emulator-ready' : ''}`} data-testid={`onboarding-emulator-${profile.platform}`}>
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <h2 className="text-base font-black text-white/90">{profile.displayName}</h2>
                  <StatusPill done={installed} label={installed ? t.common.ready : t.common.missing} />
                </div>
                <p className="mt-2 text-xs leading-5 text-white/46">
                  {manual
                    ? t.onboarding.emulatorStep.manualDetail(profile.emulator.emulatorName)
                    : t.onboarding.emulatorStep.autoDetail(profile.emulator.emulatorName)}
                </p>
                <div className="mt-2 text-xs text-white/34">
                  {t.onboarding.expected(profile.emulator.executableCandidates.join(', ') || profile.emulator.executableName || 'emulator.exe')}
                </div>
              </div>
              <div className="min-w-0">
                {manual ? (
                  <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto_auto]">
                    <input
                      value={path}
                      onChange={(event) => onPathChange(profile.id, event.target.value)}
                      className="h-10 min-w-0 rounded-sm border border-white/10 bg-black/40 px-3 text-sm outline-none focus:border-fusion-accent/70"
                      placeholder="C:\\Emulators\\..."
                    />
                    <button type="button" onClick={() => onBrowse(profile)} disabled={busy !== null} className="rh-icon-button" title={t.common.browse}>
                      {busy === `browse:${profile.id}` ? <Loader2 className="h-4 w-4 animate-spin" /> : <FolderOpen className="h-4 w-4" />}
                    </button>
                    <button type="button" onClick={() => onSaveManual(profile)} disabled={busy !== null || !path.trim()} className="rh-mini-action h-10">
                      {busy === `manual:${profile.id}` ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Wrench className="h-3.5 w-3.5" />}
                      {t.onboarding.saveEmulator}
                    </button>
                  </div>
                ) : (
                  <button type="button" onClick={() => onInstall(profile)} disabled={busy !== null || installed} className="rh-primary-action" data-testid={`onboarding-install-${profile.platform}`}>
                    {rowBusy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
                    {installed ? t.common.ready : t.onboarding.emulatorStep.install(profile.emulator.emulatorName)}
                  </button>
                )}
              </div>
            </article>
          );
        })}
      </div>
      <div className="rh-onboarding-actions">
        <button type="button" onClick={onNext} disabled={!anyReady} className="rh-primary-action" data-testid="onboarding-next-emulator">
          {t.onboarding.continue}
          <ChevronRight className="h-4 w-4" />
        </button>
      </div>
    </div>
  );
}

function ReadyStep({
  repositoryReady,
  catalogReady,
  metadataReady,
  emulatorReady,
  onboardingReady,
  demoGame,
  demoDownloaded,
  playable,
  busy,
  onOpenLauncher,
  onDownloadDemo,
  onPlayDemo,
  onBackToSource,
  onBackToMetadata,
  onBackToEmulator
}: {
  repositoryReady: boolean;
  catalogReady: boolean;
  metadataReady: boolean;
  emulatorReady: boolean;
  onboardingReady: boolean;
  demoGame: CatalogGame | null;
  demoDownloaded: boolean;
  playable: boolean;
  busy: string | null;
  onOpenLauncher: () => Promise<void>;
  onDownloadDemo: () => Promise<void>;
  onPlayDemo: () => Promise<void>;
  onBackToSource: () => void;
  onBackToMetadata: () => void;
  onBackToEmulator: () => void;
}) {
  const { t } = useI18n();

  return (
    <div className="rh-onboarding-content">
      <StepHeader icon={<CheckCircle2 className="h-5 w-5" />} title={t.onboarding.readyStep.title} copy={t.onboarding.readyStep.copy} />
      <div className="rh-onboarding-checkline">
        <SetupItem done={repositoryReady} title={t.onboarding.setup.source} detail={repositoryReady ? t.onboarding.readyStep.sourceReady : t.onboarding.sourceStep.notConnected} />
        <SetupItem done={catalogReady} title={t.onboarding.readyStep.catalog} detail={catalogReady ? t.onboarding.readyStep.catalogReady : t.onboarding.readyStep.catalogMissing} />
        <SetupItem done={metadataReady} title={t.onboarding.setup.metadata} detail={metadataReady ? t.onboarding.readyStep.metadataReady : t.onboarding.readyStep.metadataMissing} />
        <SetupItem done={emulatorReady} title={t.onboarding.setup.emulator} detail={emulatorReady ? t.onboarding.readyStep.emulatorReady : t.onboarding.readyStep.emulatorMissing} />
      </div>

      {demoGame && (
        <section className="rh-onboarding-card">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <h2 className="text-xl font-black">{displayProductText(demoGame.title)}</h2>
              <p className="mt-2 text-sm text-white/52">{demoDownloaded ? t.onboarding.setup.demoRomDetail : t.onboarding.readyStep.demoOptional}</p>
            </div>
            <StatusPill done={demoDownloaded} label={demoDownloaded ? t.common.ready : t.common.download} />
          </div>
          <div className="mt-5 flex flex-wrap gap-2">
            <button type="button" onClick={demoDownloaded ? onPlayDemo : onDownloadDemo} disabled={busy !== null || !onboardingReady} className="rh-mini-action">
              {busy === 'download' || busy === 'play' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : playable ? <Play className="h-3.5 w-3.5" /> : <Download className="h-3.5 w-3.5" />}
              {playable ? t.onboarding.play : t.onboarding.downloadDemo}
            </button>
          </div>
        </section>
      )}

      <div className="rh-onboarding-actions">
        {!repositoryReady && (
          <button type="button" onClick={onBackToSource} className="rh-mini-action">{t.onboarding.readyStep.fixSource}</button>
        )}
        {!metadataReady && (
          <button type="button" onClick={onBackToMetadata} className="rh-mini-action">{t.onboarding.readyStep.fixMetadata}</button>
        )}
        {!emulatorReady && (
          <button type="button" onClick={onBackToEmulator} className="rh-mini-action">{t.onboarding.readyStep.fixEmulator}</button>
        )}
        <button type="button" onClick={onOpenLauncher} disabled={!onboardingReady || busy !== null} className="rh-primary-action" data-testid="onboarding-open-launcher">
          {busy === 'open-launcher' ? <Loader2 className="h-4 w-4 animate-spin" /> : <Rocket className="h-4 w-4" />}
          {t.onboarding.readyStep.openLauncher}
        </button>
      </div>
    </div>
  );
}

function StepHeader({ icon, title, copy }: { icon: ReactNode; title: string; copy: string }) {
  return (
    <header className="rh-onboarding-step-header">
      <div className="rh-onboarding-step-icon">{icon}</div>
      <div>
        <h2>{title}</h2>
        <p>{copy}</p>
      </div>
    </header>
  );
}

function OnboardingSourcePreview({
  preview,
  compact
}: {
  preview: RepositoryPreview;
  compact?: boolean;
}) {
  const { locale, t } = useI18n();

  return (
    <div className={`${compact ? 'mt-4' : 'mt-3'} rounded-sm border border-white/10 bg-white/[0.04] p-3 text-xs leading-5 text-white/62`} data-testid="onboarding-source-preview">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="truncate font-bold text-white">{displayProductText(preview.name)}</div>
          <div className="truncate text-white/38">{preview.url}</div>
          <div className="text-[10px] font-semibold text-white/32">{sourceTrustLabel(preview.trustLevel, locale)}</div>
        </div>
        <span className={`rounded-lg border px-2 py-1 text-[10px] font-semibold ${preview.trustLevel === 'unknown' ? 'border-amber-300/24 bg-amber-300/10 text-amber-100' : 'border-fusion-accent/24 bg-fusion-accent/10 text-fusion-accent'}`}>
          {preview.trustLevel}
        </span>
      </div>
      <div className="mt-2 grid gap-2 sm:grid-cols-2">
        <div>{t.onboarding.preview.games(preview.catalogCount)}</div>
        <div>{t.onboarding.preview.systemFiles(preview.systemFileCount)}</div>
        <div>{t.onboarding.preview.version(preview.version)}</div>
        <div>{shortHash(preview.contentHash)}</div>
      </div>
      {preview.hasExecutableAssets && <div className="mt-2 text-amber-100">{t.onboarding.preview.executableAssets}</div>}
      {preview.trustLevel === 'unknown' && <div className="mt-2 text-amber-100">{t.onboarding.preview.verifyUnknown}</div>}
    </div>
  );
}

function SetupItem({ done, title, detail }: { done: boolean; title: string; detail: string }) {
  return (
    <div className={`rounded-sm border p-4 ${done ? 'border-fusion-accent/24 bg-fusion-accent/10' : 'border-white/10 bg-black/18'}`}>
      <div className="flex items-center gap-2">
        <CheckCircle2 className={`h-4 w-4 ${done ? 'text-emerald-200' : 'text-white/22'}`} />
        <span className="font-black">{title}</span>
      </div>
      <div className="mt-2 truncate text-xs text-white/46">{detail}</div>
    </div>
  );
}

function StatusPill({ done, label }: { done: boolean; label: string }) {
  return (
    <span className={`shrink-0 rounded-lg border px-2 py-1 text-[10px] font-semibold ${done ? 'border-emerald-200/[0.24] bg-emerald-200/10 text-emerald-100' : 'border-amber-200/[0.24] bg-amber-200/10 text-amber-100'}`}>
      {label}
    </span>
  );
}

function stepDone(
  step: WizardStep,
  repositoryReady: boolean,
  catalogReady: boolean,
  metadataReady: boolean,
  emulatorReady: boolean,
  onboardingReady: boolean
) {
  if (step === 'welcome') return true;
  if (step === 'source') return repositoryReady && catalogReady;
  if (step === 'metadata') return metadataReady;
  if (step === 'emulator') return emulatorReady;
  return onboardingReady;
}

function stepDisabled(
  step: WizardStep,
  repositoryReady: boolean,
  catalogReady: boolean,
  metadataReady: boolean
) {
  if (step === 'metadata') return !(repositoryReady && catalogReady);
  if (step === 'emulator') return !(repositoryReady && catalogReady && metadataReady);
  return false;
}

function stepDetail(
  step: WizardStep,
  repositoryReady: boolean,
  catalogReady: boolean,
  metadataReady: boolean,
  emulatorReady: boolean,
  t: ReturnType<typeof useI18n>['t']
) {
  if (step === 'welcome') return t.onboarding.stepDetails.welcome;
  if (step === 'source') return repositoryReady && catalogReady ? t.common.ready : t.onboarding.stepDetails.source;
  if (step === 'metadata') {
    if (!repositoryReady || !catalogReady) return t.onboarding.sourceStep.notConnected;
    return metadataReady ? t.common.ready : t.onboarding.stepDetails.metadata;
  }
  if (step === 'emulator') {
    if (!repositoryReady || !catalogReady) return t.onboarding.emulatorStep.needsSource;
    if (!metadataReady) return t.onboarding.readyStep.metadataMissing;
    return emulatorReady ? t.common.ready : t.onboarding.stepDetails.emulator;
  }
  return repositoryReady && catalogReady && metadataReady && emulatorReady ? t.common.ready : t.onboarding.stepDetails.ready;
}

function platformLabel(platform: string) {
  return PLATFORM_LABELS[platform as Platform] ?? platform;
}

function shortHash(value: string) {
  if (value.length <= 16) return value;
  return `${value.slice(0, 8)}...${value.slice(-8)}`;
}
