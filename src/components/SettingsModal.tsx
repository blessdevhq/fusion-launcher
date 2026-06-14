'use client';

import { FormEvent, KeyboardEvent, useEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { Activity, DatabaseZap, Gamepad2, HardDrive, Link2, Loader2, RefreshCcw, Save, Settings, SlidersHorizontal, X } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import { DiagnosticsSection } from '@/components/settings/DiagnosticsSection';
import { EmulatorsSection } from '@/components/settings/EmulatorsSection';
import { GeneralSection } from '@/components/settings/GeneralSection';
import { MetadataSection } from '@/components/settings/MetadataSection';
import { MetricLine } from '@/components/settings/shared';
import { SourcesSection } from '@/components/settings/SourcesSection';
import { StorageSection } from '@/components/settings/StorageSection';
import type { BusyAction, BusyState, SettingsSection, UpdatePanelState } from '@/components/settings/types';
import { getFocusableElements, sectionTitle } from '@/components/settings/utils';
import { UpdatesSection } from '@/components/settings/UpdatesSection';
import { api } from '@/lib/api';
import { isTauriRuntime } from '@/lib/runtime';
import { getEmulatorPath, type AppSettings } from '@/lib/settings';
import type { Locale } from '@/lib/i18n';
import { countConfiguredEmulators, getEmulatorDraftState, hasEmulatorDraftChanges, updateDraftEmulatorPath } from '@/lib/settingsModalState';
import { MVP_PLATFORMS, PLATFORM_LABELS, type MvpPlatform } from '@/types/platform';
import type { HealthReport, LibraryScrapeProgressEvent, PlatformSetupProfile, RepositoryPreview, RepositorySummary, ScreenScraperStatus, SteamGridDbStatus, TorrentDownloadRecord } from '@/types/repository';

interface SettingsModalProps {
  settings: AppSettings;
  repositories: RepositorySummary[];
  downloads: TorrentDownloadRecord[];
  busyAction: BusyAction;
  healthReport: HealthReport | null;
  updatePanel: UpdatePanelState;
  sourceUrl: string;
  sourcePreview: RepositoryPreview | null;
  onClose: () => void;
  onSave: (settings: AppSettings) => Promise<AppSettings>;
  onSourceUrlChange: (value: string) => void;
  onPreviewRepositoryUrl: () => Promise<void>;
  onConnectRepositoryUrl: () => Promise<void>;
  onConnectRepositoryFile: () => Promise<void>;
  onDisconnect: (repositoryId: string) => Promise<void>;
  onRefreshRepository: (repositoryId: string) => Promise<void>;
  onRunHealth: () => Promise<void>;
  onCopyDiagnostics: () => Promise<void>;
  onOpenLogs: () => Promise<void>;
  onCheckAppUpdate: () => Promise<void>;
  onInstallAppUpdate: () => Promise<void>;
}

const SECTIONS: Array<{ id: SettingsSection; icon: typeof Settings }> = [
  { id: 'general', icon: SlidersHorizontal },
  { id: 'emulators', icon: Gamepad2 },
  { id: 'metadata', icon: DatabaseZap },
  { id: 'sources', icon: Link2 },
  { id: 'storage', icon: HardDrive },
  { id: 'diagnostics', icon: Activity },
  { id: 'updates', icon: RefreshCcw }
];

export function SettingsModal({
  settings,
  repositories,
  downloads,
  busyAction,
  healthReport,
  updatePanel,
  sourceUrl,
  sourcePreview,
  onClose,
  onSave,
  onSourceUrlChange,
  onPreviewRepositoryUrl,
  onConnectRepositoryUrl,
  onConnectRepositoryFile,
  onDisconnect,
  onRefreshRepository,
  onRunHealth,
  onCopyDiagnostics,
  onOpenLogs,
  onCheckAppUpdate,
  onInstallAppUpdate
}: SettingsModalProps) {
  const { locale, t } = useI18n();
  const [savedSettings, setSavedSettings] = useState<AppSettings>(settings);
  const [draftSettings, setDraftSettings] = useState<AppSettings>(settings);
  const [activeSection, setActiveSection] = useState<SettingsSection>('emulators');
  const [activePlatform, setActivePlatform] = useState<MvpPlatform>(MVP_PLATFORMS[0]);
  const [busy, setBusy] = useState<BusyState>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [downloadRoot, setDownloadRoot] = useState('');
  const [savedDownloadRoot, setSavedDownloadRoot] = useState('');
  const [profiles, setProfiles] = useState<PlatformSetupProfile[]>([]);
  const [appDataDir, setAppDataDir] = useState('');
  const [logPath, setLogPath] = useState('');
  const [scraperStatus, setScraperStatus] = useState<ScreenScraperStatus | null>(null);
  const [scraperSsid, setScraperSsid] = useState('');
  const [scraperPassword, setScraperPassword] = useState('');
  const [scraperRegion, setScraperRegion] = useState('auto');
  const [metadataBusy, setMetadataBusy] = useState(false);
  const [steamgriddbStatus, setSteamgriddbStatus] = useState<SteamGridDbStatus | null>(null);
  const [steamgriddbKey, setSteamgriddbKey] = useState('');
  const [steamgriddbBusy, setSteamgriddbBusy] = useState(false);
  const [batchBusy, setBatchBusy] = useState(false);
  const [batchProgress, setBatchProgress] = useState<LibraryScrapeProgressEvent | null>(null);
  const modalRef = useRef<HTMLDivElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    setSavedSettings(settings);
    setDraftSettings(settings);
  }, [settings]);

  useEffect(() => {
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const focusTimer = window.setTimeout(() => closeButtonRef.current?.focus(), 0);

    return () => {
      window.clearTimeout(focusTimer);
      previousFocus?.focus();
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    Promise.all([
      api.getDownloadRoot(),
      api.listPlatformSetupProfiles(),
      api.getDiagnosticsPaths(),
      api.getScreenscraperStatus(),
      api.getSteamgriddbStatus()
    ])
      .then(([downloadFolder, setupProfiles, diagnostics, metadataStatus, steamMetadataStatus]) => {
        if (cancelled) return;
        setDownloadRoot(downloadFolder);
        setSavedDownloadRoot(downloadFolder);
        setProfiles(setupProfiles);
        setAppDataDir(diagnostics.dataDir);
        setLogPath(diagnostics.logPath);
        setScraperStatus(metadataStatus);
        setScraperSsid(metadataStatus.ssid ?? '');
        setScraperRegion(metadataStatus.region ?? 'auto');
        setSteamgriddbStatus(steamMetadataStatus);
      })
      .catch((error) => {
        if (!cancelled) setMessage(t.settings.messages.loadDetailsError(error));
      });

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!isTauriRuntime()) return undefined;
    let cleanup: (() => void) | null = null;
    const unlistenPromise = listen<LibraryScrapeProgressEvent>('scrape:batch', (event) => {
      const progress = event.payload;
      setBatchProgress(progress);
      setSteamgriddbStatus((current) => current
        ? {
            ...current,
            pendingBatch: Math.max(progress.total - progress.done, 0),
            batchRunning: progress.done < progress.total
          }
        : current);
    });
    void unlistenPromise.then((unlisten) => {
      cleanup = unlisten;
    });
    return () => {
      if (cleanup) cleanup();
      else void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  const configuredCount = countConfiguredEmulators(draftSettings);
  const readyCount = useMemo(() => (
    MVP_PLATFORMS.filter((platform) => (
      getEmulatorDraftState(draftSettings, savedSettings, platform, locale).tone === 'valid'
    )).length
  ), [draftSettings, locale, savedSettings]);
  const changedEmulators = hasEmulatorDraftChanges(draftSettings, savedSettings);
  const changedStorage = downloadRoot.trim() !== savedDownloadRoot.trim();
  const changedLanguage = draftSettings.language !== savedSettings.language;
  const hasUnsavedChanges = changedEmulators || changedStorage || changedLanguage;
  const activeDownloadsCount = downloads.filter((download) => (
    download.status === 'resolving' || download.status === 'downloading' || download.status === 'cancelling'
  )).length;

  const updateEmulatorPath = (platform: MvpPlatform, emulatorPath: string) => {
    setDraftSettings((currentSettings) => updateDraftEmulatorPath(currentSettings, platform, emulatorPath));
    setActivePlatform(platform);
    setMessage(null);
  };

  const browseForEmulator = async (platform: MvpPlatform) => {
    setBusy(`browse:${platform}`);
    setActivePlatform(platform);
    setMessage(null);
    try {
      if (!isTauriRuntime()) {
        setMessage(t.settings.messages.nativeFilePickerUnavailable);
        return;
      }

      const currentPath = getEmulatorPath(draftSettings, platform);
      const selected = await open({
        title: t.settings.emulators.pickerTitle(PLATFORM_LABELS[platform]),
        multiple: false,
        directory: false,
        defaultPath: currentPath || undefined,
        filters: [
          {
            name: t.settings.emulators.windowsExecutable,
            extensions: ['exe']
          }
        ]
      });

      if (typeof selected === 'string') {
        updateEmulatorPath(platform, selected);
      }
    } catch (error) {
      setMessage(t.settings.messages.browseError(error));
    } finally {
      setBusy(null);
    }
  };

  const saveMetadataSettings = async () => {
    setMetadataBusy(true);
    setMessage(null);
    try {
      const nextStatus = await api.saveScreenscraperCredentials(scraperSsid, scraperPassword, scraperRegion);
      setScraperStatus(nextStatus);
      setScraperSsid(nextStatus.ssid ?? scraperSsid.trim());
      setScraperRegion(nextStatus.region ?? 'auto');
      setScraperPassword('');
      setMessage('ScreenScraper metadata settings saved.');
    } catch (error) {
      setMessage(`Failed to save ScreenScraper settings: ${error}`);
    } finally {
      setMetadataBusy(false);
    }
  };

  const saveSteamgriddbSettings = async () => {
    setSteamgriddbBusy(true);
    setMessage(null);
    try {
      const nextStatus = await api.saveSteamgriddbKey(steamgriddbKey);
      setSteamgriddbStatus(nextStatus);
      setSteamgriddbKey('');
      setMessage('SteamGridDB artwork settings saved.');
    } catch (error) {
      setMessage(`Failed to save SteamGridDB settings: ${error}`);
    } finally {
      setSteamgriddbBusy(false);
    }
  };

  const startLibraryScrape = async () => {
    setBatchBusy(true);
    setMessage(null);
    setBatchProgress(null);
    try {
      const status = await api.scrapeLibrary();
      const nextStatus = await api.getSteamgriddbStatus();
      setSteamgriddbStatus({
        ...nextStatus,
        pendingBatch: status.pending,
        batchRunning: status.running
      });
      setMessage(status.pending > 0 ? 'Library metadata scrape started.' : 'No installed games are queued for metadata scraping.');
    } catch (error) {
      setMessage(`Failed to start library scrape: ${error}`);
    } finally {
      setBatchBusy(false);
    }
  };

  const cancelLibraryScrape = async () => {
    setBatchBusy(true);
    setMessage(null);
    try {
      const status = await api.cancelLibraryScrape();
      setSteamgriddbStatus((current) => current
        ? { ...current, pendingBatch: status.pending, batchRunning: status.running }
        : current);
      setMessage('Library metadata scrape cancellation requested.');
    } catch (error) {
      setMessage(`Failed to cancel library scrape: ${error}`);
    } finally {
      setBatchBusy(false);
    }
  };

  const save = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!hasUnsavedChanges) return;

    setBusy('save');
    setMessage(null);
    try {
      const nextSavedSettings = await onSave(draftSettings);
      if (downloadRoot.trim() && changedStorage) {
        const nextDownloadRoot = await api.setDownloadRoot(downloadRoot.trim());
        setDownloadRoot(nextDownloadRoot);
        setSavedDownloadRoot(nextDownloadRoot);
      }
      setSavedSettings(nextSavedSettings);
      setDraftSettings(nextSavedSettings);
      setMessage(t.settings.messages.saveSuccess);
    } catch (error) {
      setMessage(t.settings.messages.saveError(error));
    } finally {
      setBusy(null);
    }
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      onClose();
      return;
    }

    if (event.key !== 'Tab') return;
    const focusable = getFocusableElements(modalRef.current);
    if (focusable.length === 0) return;

    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const activeElement = document.activeElement;

    if (event.shiftKey && activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 grid place-items-center bg-black/82 px-5 py-5"
      onKeyDown={handleKeyDown}
      data-testid="settings-modal"
    >
      <section
        ref={modalRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-modal-title"
        className="flex h-[min(760px,calc(100vh-40px))] w-[min(1080px,calc(100vw-40px))] overflow-hidden rounded-2xl border border-white/10 bg-fusion-surface/95 text-white shadow-[0_40px_120px_rgba(0,0,0,0.72)] outline-none"
      >
        <aside className="hidden w-60 shrink-0 border-r border-white/10 bg-white/[0.035] p-5 md:flex md:flex-col">
          <div>
            <h2 id="settings-modal-title" className="text-2xl font-bold tracking-normal">{t.settings.title}</h2>
            <p className="mt-2 text-xs leading-5 text-white/[0.46]">{t.settings.description}</p>
          </div>

          <nav className="mt-8 grid gap-2">
            {SECTIONS.map((section) => {
              const Icon = section.icon;
              const active = activeSection === section.id;
              return (
                <button
                  key={section.id}
                  type="button"
                  data-testid={`settings-tab-${section.id}`}
                  onClick={() => setActiveSection(section.id)}
                  className={`flex h-11 items-center gap-3 rounded-lg border px-3 text-left text-sm font-semibold transition ${
                    active
                      ? 'border-fusion-accent/40 bg-fusion-accent/15 text-fusion-accent shadow-glow'
                      : 'border-transparent text-white/[0.48] hover:border-white/[0.14] hover:bg-white/[0.045] hover:text-white/[0.82]'
                  }`}
                >
                  <Icon className="h-4 w-4 shrink-0" />
                  {t.settings.sections[section.id]}
                </button>
              );
            })}
          </nav>

          <div className="mt-auto border-t border-white/10 pt-5">
            <div className="text-xs font-semibold text-white/[0.42]">{t.settings.readiness.title}</div>
            <div className="mt-3 grid gap-2 text-xs text-white/[0.54]">
              <MetricLine label={t.settings.readiness.configured} value={`${configuredCount}/${MVP_PLATFORMS.length}`} />
              <MetricLine label={t.settings.readiness.ready} value={`${readyCount}/${MVP_PLATFORMS.length}`} />
              <MetricLine label={t.settings.readiness.sources} value={String(repositories.length)} />
              <MetricLine label={t.settings.readiness.unsaved} value={hasUnsavedChanges ? t.common.yes : t.common.no} />
            </div>
          </div>
        </aside>

        <form onSubmit={save} className="flex min-h-0 min-w-0 flex-1 flex-col">
          <header className="flex min-h-20 items-start justify-between gap-4 border-b border-white/10 px-5 py-5 md:px-7">
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-3">
                <h2 className="text-2xl font-bold tracking-normal md:hidden">{t.settings.title}</h2>
                <h3 className="text-xl font-bold tracking-normal md:text-2xl">{sectionTitle(activeSection, t)}</h3>
                {hasUnsavedChanges && (
                  <span className="rounded-lg border border-white/[0.18] bg-white/[0.07] px-2 py-1 text-xs font-semibold text-white/[0.78]">
                    {t.settings.unsavedBadge}
                  </span>
                )}
              </div>
            </div>
            <button
              ref={closeButtonRef}
              type="button"
              onClick={onClose}
              className="grid h-9 w-9 shrink-0 place-items-center rounded-lg border border-white/10 bg-white/[0.035] text-white/[0.62] transition hover:border-fusion-accent/40 hover:bg-fusion-accent/10 hover:text-white focus:border-fusion-accent/70 focus:outline-none"
              title={t.settings.closeTitle}
            >
              <X className="h-4 w-4" />
            </button>
          </header>

          <div className="flex border-b border-white/10 md:hidden">
            {SECTIONS.map((section) => (
              <button
                key={section.id}
                type="button"
                data-testid={`settings-mobile-tab-${section.id}`}
                onClick={() => setActiveSection(section.id)}
                className={`min-w-0 flex-1 px-2 py-3 text-xs font-semibold transition ${
                  activeSection === section.id ? 'bg-fusion-accent/14 text-fusion-accent' : 'text-white/[0.46]'
                }`}
              >
                {t.settings.sections[section.id]}
              </button>
            ))}
          </div>

          <div className="min-h-0 flex-1 overscroll-contain overflow-y-auto px-5 py-5 [scrollbar-gutter:stable] md:px-7" data-testid={`settings-modal-${activeSection}`}>
            {activeSection === 'general' && (
              <GeneralSection
                configuredCount={configuredCount}
                readyCount={readyCount}
                repositoriesCount={repositories.length}
                activeDownloadsCount={activeDownloadsCount}
                updatePhase={updatePanel.phase}
                healthReport={healthReport}
                hasUnsavedChanges={hasUnsavedChanges}
                desktopBridge={isTauriRuntime()}
                draftLanguage={draftSettings.language}
                onLanguageChange={(language) => {
                  setDraftSettings((currentSettings) => ({ ...currentSettings, language }));
                  setMessage(null);
                }}
                onOpenSection={setActiveSection}
              />
            )}

            {activeSection === 'emulators' && (
              <EmulatorsSection
                draftSettings={draftSettings}
                savedSettings={savedSettings}
                activePlatform={activePlatform}
                busy={busy}
                locale={locale}
                onFocusPlatform={setActivePlatform}
                onPathChange={updateEmulatorPath}
                onBrowse={browseForEmulator}
              />
            )}

            {activeSection === 'metadata' && (
              <MetadataSection
                status={scraperStatus}
                ssid={scraperSsid}
                password={scraperPassword}
                region={scraperRegion}
                busy={metadataBusy}
                steamStatus={steamgriddbStatus}
                steamKey={steamgriddbKey}
                steamBusy={steamgriddbBusy}
                batchBusy={batchBusy}
                batchProgress={batchProgress}
                onSsidChange={setScraperSsid}
                onPasswordChange={setScraperPassword}
                onRegionChange={setScraperRegion}
                onSave={saveMetadataSettings}
                onSteamKeyChange={setSteamgriddbKey}
                onSaveSteam={saveSteamgriddbSettings}
                onScrapeLibrary={startLibraryScrape}
                onCancelLibraryScrape={cancelLibraryScrape}
              />
            )}

            {activeSection === 'sources' && (
              <SourcesSection
                repositories={repositories}
                busyAction={busyAction}
                sourceUrl={sourceUrl}
                sourcePreview={sourcePreview}
                onSourceUrlChange={onSourceUrlChange}
                onPreviewRepositoryUrl={onPreviewRepositoryUrl}
                onConnectRepositoryUrl={onConnectRepositoryUrl}
                onConnectRepositoryFile={onConnectRepositoryFile}
                onRefreshRepository={onRefreshRepository}
                onDisconnect={onDisconnect}
              />
            )}

            {activeSection === 'storage' && (
              <StorageSection
                downloadRoot={downloadRoot}
                appDataDir={appDataDir}
                logPath={logPath}
                changed={changedStorage}
                onDownloadRootChange={(value) => {
                  setDownloadRoot(value);
                  setMessage(null);
                }}
              />
            )}

            {activeSection === 'diagnostics' && (
              <DiagnosticsSection
                profiles={profiles}
                health={healthReport}
                busyAction={busyAction}
                onRunHealth={onRunHealth}
                onCopyDiagnostics={onCopyDiagnostics}
                onOpenLogs={onOpenLogs}
              />
            )}

            {activeSection === 'updates' && (
              <UpdatesSection
                state={updatePanel}
                onCheck={onCheckAppUpdate}
                onInstall={onInstallAppUpdate}
              />
            )}
          </div>

          {message && (
            <div className="mx-5 mb-4 rounded-sm border border-white/[0.12] bg-white/[0.055] px-3 py-2 text-sm text-white/70 md:mx-7">
              {message}
            </div>
          )}

          <footer className="flex flex-wrap items-center justify-between gap-3 border-t border-white/10 px-5 py-4 md:px-7">
            <div className="text-xs text-white/[0.38]">
              {hasUnsavedChanges ? t.settings.footerDirty : t.settings.footerClean}
            </div>
            <div className="flex gap-2">
              <button
                type="button"
                onClick={onClose}
                disabled={busy !== null}
                className="h-10 rounded-lg border border-white/10 px-4 text-sm font-semibold text-white/[0.66] transition hover:border-white/[0.36] hover:bg-white/[0.065] hover:text-white disabled:opacity-40"
              >
                {t.common.close}
              </button>
              <button
                type="submit"
                disabled={busy !== null || !hasUnsavedChanges}
                className="inline-flex h-10 items-center gap-2 rounded-lg border border-fusion-accent/70 bg-fusion-accent px-4 text-sm font-bold text-fusion-accentOn transition hover:bg-fusion-accentHover disabled:border-white/10 disabled:bg-white/[0.06] disabled:text-white/[0.32]"
              >
                {busy === 'save' ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Save className="h-4 w-4" />
                )}
                {t.common.save}
              </button>
            </div>
          </footer>
        </form>
      </section>
    </div>
  );
}
