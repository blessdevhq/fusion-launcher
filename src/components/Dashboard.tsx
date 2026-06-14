'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { I18nProvider } from '@/components/I18nProvider';
import { GameDetailsModal } from '@/components/GameDetailsModal';
import { LaunchErrorModal } from '@/components/LaunchErrorModal';
import { SettingsModal } from '@/components/SettingsModal';
import { CollectionsScreen } from '@/components/dashboard/CollectionsScreen';
import { DownloadsScreen } from '@/components/dashboard/DownloadsScreen';
import { ExploreScreen } from '@/components/dashboard/ExploreScreen';
import { HomeScreen } from '@/components/dashboard/HomeScreen';
import { LibraryScreen } from '@/components/dashboard/LibraryScreen';
import { TopChrome } from '@/components/dashboard/TopChrome';
import type { BusyAction, UpdatePanelState } from '@/components/dashboard/types';
import { composeHomeRails, cssEscape, normalizeUpdateCheckError, safeDecodeURIComponent } from '@/components/dashboard/utils';
import { AppShell } from '@/components/shell/AppShell';
import { collectionTargetForId, type CollectionTarget } from '@/components/shell/CockpitPanels';
import { useGamepad } from '@/hooks/useGamepad';
import { buildGameLibraryItems, searchAndSortLibraryItems, type GameLibraryItem, type LibraryFilter, type LibrarySort } from '@/lib/libraryStatus';
import { api } from '@/lib/api';
import { isDirectGameDownload } from '@/lib/downloadActions';
import { getUiText } from '@/lib/i18n';
import { normalizeLaunchFailure } from '@/lib/launchErrors';
import { isTauriRuntime } from '@/lib/runtime';
import { loadSettings, saveSettings, type AppSettings } from '@/lib/settings';
import { unknownSourcePrompt } from '@/lib/sourceTrust';
import { useLauncherStore, type LauncherView } from '@/stores/launcherStore';
import type {
  CatalogGame,
  DownloadProgressEvent,
  HealthReport,
  LaunchFailure,
  RepositoryPreview,
  RepositorySummary
} from '@/types/repository';

interface DashboardProps {
  initialSettings: AppSettings;
  catalog: CatalogGame[];
  repositories: RepositorySummary[];
  message: string | null;
  onDisconnectRepository: (repositoryId: string) => Promise<void>;
  onRefresh: () => Promise<void>;
}

export function Dashboard({
  initialSettings,
  catalog,
  repositories,
  message,
  onDisconnectRepository,
  onRefresh
}: DashboardProps) {
  const storeCatalog = useLauncherStore((state) => state.catalog);
  const storeRepositories = useLauncherStore((state) => state.repositories);
  const libraryStatuses = useLauncherStore((state) => state.libraryStatuses);
  const downloads = useLauncherStore((state) => state.downloads);
  const settings = useLauncherStore((state) => state.settings);
  const activeView = useLauncherStore((state) => state.activeView);
  const focusedItemId = useLauncherStore((state) => state.focusedItemId);
  const selectedGameId = useLauncherStore((state) => state.selectedGameId);
  const activityEvents = useLauncherStore((state) => state.activityEvents);
  const setCatalog = useLauncherStore((state) => state.setCatalog);
  const setRepositories = useLauncherStore((state) => state.setRepositories);
  const setLibraryStatuses = useLauncherStore((state) => state.setLibraryStatuses);
  const setDownloads = useLauncherStore((state) => state.setDownloads);
  const setSettings = useLauncherStore((state) => state.setSettings);
  const setActiveView = useLauncherStore((state) => state.setActiveView);
  const setFocusedItemId = useLauncherStore((state) => state.setFocusedItemId);
  const setSelectedGameId = useLauncherStore((state) => state.setSelectedGameId);
  const mergeDownloadEvent = useLauncherStore((state) => state.mergeDownloadEvent);
  const addActivityEvent = useLauncherStore((state) => state.addActivityEvent);

  const [libraryFilter, setLibraryFilter] = useState<LibraryFilter>('all');
  const [librarySearch, setLibrarySearch] = useState('');
  const [librarySort, setLibrarySort] = useState<LibrarySort>('title');
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsMessage, setSettingsMessage] = useState<string | null>(null);
  const [launcherMessage, setLauncherMessage] = useState<string | null>(null);
  const [sourceUrl, setSourceUrl] = useState('');
  const [sourcePreview, setSourcePreview] = useState<RepositoryPreview | null>(null);
  const [busyAction, setBusyAction] = useState<BusyAction>(null);
  const [dataReady, setDataReady] = useState(false);
  const [launchFailure, setLaunchFailure] = useState<LaunchFailure | null>(null);
  const [healthReport, setHealthReport] = useState<HealthReport | null>(null);
  const [notificationsOpen, setNotificationsOpen] = useState(false);
  const [seenNotificationKey, setSeenNotificationKey] = useState('');
  const [updatePanel, setUpdatePanel] = useState<UpdatePanelState>({
    phase: 'idle',
    report: null,
    error: null
  });
  const locale = settings.language;
  const t = getUiText(locale);

  useEffect(() => {
    setSettings(initialSettings);
  }, [initialSettings, setSettings]);

  useEffect(() => {
    setCatalog(catalog);
    setRepositories(repositories);
  }, [catalog, repositories, setCatalog, setRepositories]);

  const refreshLauncherData = useCallback(async () => {
    try {
      const [nextLibraryStatuses, nextDownloads] = await Promise.all([
        api.getLibraryStatuses(),
        api.listTorrentDownloads()
      ]);
      setLibraryStatuses(nextLibraryStatuses);
      setDownloads(nextDownloads);
      setLauncherMessage(null);
    } catch (error) {
      setLauncherMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setDataReady(true);
    }
  }, [setDownloads, setLibraryStatuses]);

  useEffect(() => {
    let cancelled = false;
    const loadPersistedSettings = async () => {
      try {
        const persistedSettings = await loadSettings();
        if (!cancelled) {
          setSettings(persistedSettings);
          setSettingsMessage(null);
        }
      } catch (error) {
        if (!cancelled) setSettingsMessage(t.dashboard.messages.settingsLoadError(error));
      }
    };

    void loadPersistedSettings();
    return () => {
      cancelled = true;
    };
  }, [setSettings, t]);

  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  useEffect(() => {
    void refreshLauncherData();
  }, [storeCatalog.length, refreshLauncherData]);

  useEffect(() => {
    if (!isTauriRuntime()) return;

    let active = true;
    const unlistenPromise = listen<DownloadProgressEvent>('download:progress', (event) => {
      if (active) mergeDownloadEvent(event.payload);
    });

    return () => {
      active = false;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [mergeDownloadEvent]);

  const items = useMemo(
    () => buildGameLibraryItems(storeCatalog, libraryStatuses, settings),
    [libraryStatuses, settings, storeCatalog]
  );
  const itemsByGameId = useMemo(() => new Map(items.map((item) => [item.game.id, item])), [items]);
  const activeDownloadItems = useMemo(
    () => items.filter((item) => item.isDownloading || item.isPaused || item.hasError),
    [items]
  );
  const selectedGame = selectedGameId ? storeCatalog.find((game) => game.id === selectedGameId) ?? null : null;
  const visibleLibraryItems = useMemo(
    () => searchAndSortLibraryItems(items, libraryFilter, librarySearch, librarySort),
    [items, libraryFilter, librarySearch, librarySort]
  );
  const homeRails = useMemo(() => composeHomeRails(items, t), [items, t]);
  const notificationAlertKey = useMemo(() => {
    const updateKey = updatePanel.phase === 'available'
      ? `update:${updatePanel.report?.version ?? 'available'}`
      : '';
    const eventKey = activityEvents[0]?.id ? `event:${activityEvents[0].id}` : '';
    return [updateKey, eventKey].filter(Boolean).join('|');
  }, [activityEvents, updatePanel.phase, updatePanel.report?.version]);
  const hasNotificationAlert = Boolean(notificationAlertKey && notificationAlertKey !== seenNotificationKey);

  useEffect(() => {
    if (notificationsOpen) setSeenNotificationKey(notificationAlertKey);
  }, [notificationAlertKey, notificationsOpen]);

  const persistSettings = async (nextSettings: AppSettings) => {
    const savedSettings = await saveSettings(nextSettings);
    setSettings(savedSettings);
    setSettingsMessage(null);
    await refreshLauncherData();
    return savedSettings;
  };

  const runAction = async (label: string, action: () => Promise<unknown>) => {
    setBusyAction(label);
    setLauncherMessage(null);
    try {
      await action();
      await refreshLauncherData();
    } catch (error) {
      setLauncherMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusyAction(null);
    }
  };

  const refreshAll = async () => {
    await runAction('refresh', async () => {
      await onRefresh();
      await refreshLauncherData();
    });
  };

  const disconnect = async (repositoryId: string) => {
    await runAction(`repo:${repositoryId}`, async () => onDisconnectRepository(repositoryId));
  };

  const refreshRepository = async (repositoryId: string) => {
    await runAction(`repo-refresh:${repositoryId}`, async () => {
      await api.refreshRepository(repositoryId);
      await onRefresh();
    });
  };

  const updateSourceUrl = (value: string) => {
    setSourceUrl(value);
    setSourcePreview(null);
  };

  const previewRepositoryUrl = async () => {
    const trimmedUrl = sourceUrl.trim();
    if (!trimmedUrl) {
      setLauncherMessage(t.dashboard.messages.sourceUrlRequired);
      return;
    }

    await runAction('repo-preview-url', async () => {
      const preview = await api.previewRepository(trimmedUrl);
      setSourcePreview(preview);
      addActivityEvent({
        title: t.dashboard.messages.sourceChecked,
        detail: preview.name,
        tone: preview.hasExecutableAssets ? 'warning' : 'info'
      });
    });
  };

  const connectRepositoryUrl = async () => {
    const trimmedUrl = sourceUrl.trim();
    if (!trimmedUrl) {
      setLauncherMessage(t.dashboard.messages.sourceUrlRequired);
      return;
    }

    await runAction('repo-connect-url', async () => {
      const preview = sourcePreview?.url === trimmedUrl
        ? sourcePreview
        : await api.previewRepository(trimmedUrl);
      if (preview.trustLevel === 'unknown') {
        const confirmed = window.confirm(unknownSourcePrompt(preview, locale));
        if (!confirmed) return;
      }
      await api.connectRepository(trimmedUrl);
      setSourceUrl('');
      setSourcePreview(null);
      addActivityEvent({
        title: t.dashboard.messages.sourceConnected,
        detail: preview.name,
        tone: preview.hasExecutableAssets ? 'warning' : 'success'
      });
      await onRefresh();
    });
  };

  const connectRepositoryFile = async () => {
    if (!isTauriRuntime()) {
      setLauncherMessage(t.dashboard.messages.localJsonDesktopOnly);
      return;
    }
    await runAction('repo-file', async () => {
      const selected = await open({
        title: t.dashboard.messages.selectSourceJson,
        multiple: false,
        directory: false,
        filters: [{ name: t.dashboard.messages.repositoryJson, extensions: ['json'] }]
      });
      if (typeof selected !== 'string') return;
      const preview = await api.previewRepositoryFile(selected);
      if (preview.trustLevel === 'unknown') {
        const confirmed = window.confirm(unknownSourcePrompt(preview, locale));
        if (!confirmed) return;
      }
      await api.connectRepositoryFile(selected);
      await onRefresh();
    });
  };

  const runHealthCheck = async () => {
    await runAction('health', async () => {
      setHealthReport(await api.runHealthCheck());
    });
  };

  const copyDiagnostics = async () => {
    await runAction('diagnostics', async () => {
      const bundle = await api.getDiagnosticsBundle();
      await navigator.clipboard.writeText(JSON.stringify(bundle, null, 2));
      setHealthReport(bundle.health);
      addActivityEvent({
        title: t.dashboard.messages.diagnosticsCopied,
        detail: bundle.logPath,
        tone: 'success'
      });
    });
  };

  const openLogs = async () => {
    await runAction('logs', () => api.openLogsFolder());
  };

  const checkAppUpdate = async () => {
    setUpdatePanel((current) => ({ ...current, phase: 'checking', error: null }));
    try {
      const report = await api.checkAppUpdate();
      setUpdatePanel({
        phase: report.available ? 'available' : 'up-to-date',
        report,
        error: null
      });
    } catch (error) {
      setUpdatePanel({
        phase: 'error',
        report: null,
        error: normalizeUpdateCheckError(error)
      });
    }
  };

  const installAppUpdate = async () => {
    setUpdatePanel((current) => ({ ...current, phase: 'installing', error: null }));
    try {
      await api.installAppUpdate();
      setUpdatePanel((current) => ({
        phase: 'up-to-date',
        report: current.report,
        error: null
      }));
    } catch (error) {
      setUpdatePanel((current) => ({
        phase: 'error',
        report: current.report,
        error: normalizeUpdateCheckError(error)
      }));
    }
  };

  const installItem = async (item: GameLibraryItem) => {
    setBusyAction(`download:${item.game.id}`);
    setLauncherMessage(null);
    try {
      const result = await api.installGame(item.game.id);
      setSettings(await loadSettings());
      addActivityEvent({
        title: result.status === 'ready' ? t.dashboard.messages.installComplete : t.dashboard.messages.installNeedsAttention,
        detail: item.game.title,
        gameId: item.game.id,
        tone: result.status === 'ready' ? 'success' : 'warning'
      });
      if (result.status !== 'ready') {
        setLauncherMessage(result.message ?? result.errorCode ?? t.dashboard.messages.installNeedsAttentionDetail);
        setSelectedGameId(item.game.id);
      }
      await refreshLauncherData();
    } catch (error) {
      setLauncherMessage(error instanceof Error ? error.message : String(error));
      setSelectedGameId(item.game.id);
    } finally {
      setBusyAction(null);
    }
  };

  const launchItem = async (item: GameLibraryItem) => {
    setBusyAction(`play:${item.game.id}`);
    setLauncherMessage(null);
    setLaunchFailure(null);
    try {
      await api.launchGame(item.game.id);
      addActivityEvent({
        title: t.dashboard.messages.launchSent,
        detail: item.game.title,
        gameId: item.game.id,
        tone: 'success'
      });
      await refreshLauncherData();
    } catch (error) {
      setLaunchFailure(normalizeLaunchFailure(error, item.game));
    } finally {
      setBusyAction(null);
    }
  };

  const executePrimaryAction = async (item: GameLibraryItem) => {
    if (item.primaryAction === 'play') return launchItem(item);
    if (item.primaryAction === 'download') return installItem(item);
    if (item.primaryAction === 'resume' || item.primaryAction === 'retry') {
      return runAction(`resume:${item.game.id}`, () => (
        isDirectGameDownload(item.game, item.download)
          ? api.startGameDownload(item.game.id)
          : api.resumeDownload(item.game.id)
      ));
    }
    setSelectedGameId(item.game.id);
  };

  const openLibraryCollection = useCallback((target: CollectionTarget) => {
    setLibraryFilter(target.filter);
    setLibrarySearch(target.query);
    setLibrarySort(target.sort);
    setActiveView('library');
  }, [setActiveView]);

  const focusActivate = useCallback((focusId: string) => {
    const [kind, ...rest] = focusId.split(':');
    const value = rest.join(':');
    const encodedTail = rest[rest.length - 1] ?? '';
    const gameId = safeDecodeURIComponent(encodedTail);

    if (kind === 'nav') {
      setActiveView(value as LauncherView);
      return;
    }
    if (kind === 'top') {
      if (value === 'refresh') void refreshAll();
      if (value === 'notifications') setNotificationsOpen((open) => !open);
      if (value === 'update-check') {
        setNotificationsOpen(true);
        void checkAppUpdate();
      }
      if (value === 'update-install') {
        setNotificationsOpen(true);
        void installAppUpdate();
      }
      if (value === 'settings') {
        setNotificationsOpen(false);
        setSettingsOpen(true);
      }
      return;
    }
    if (kind === 'filter') {
      setLibraryFilter(value as LibraryFilter);
      return;
    }
    if (kind === 'action') {
      const item = itemsByGameId.get(gameId || value);
      if (item) void executePrimaryAction(item);
      return;
    }
    if (kind === 'details' || kind === 'game') {
      if (gameId || value) setSelectedGameId(gameId || value);
      return;
    }
    if (kind === 'download-action') {
      const [downloadAction] = rest;
      if (!gameId) return;
      if (downloadAction === 'pause') void runAction(`pause:${gameId}`, () => api.pauseDownload(gameId));
      if (downloadAction === 'resume' || downloadAction === 'retry') {
        const item = itemsByGameId.get(gameId);
        void runAction(`resume:${gameId}`, () => (
          isDirectGameDownload(item?.game, item?.download)
            ? api.startGameDownload(gameId)
            : api.resumeDownload(gameId)
        ));
      }
      if (downloadAction === 'cancel') void runAction(`cancel:${gameId}`, () => api.cancelDownload(gameId));
      if (downloadAction === 'play') {
        const item = itemsByGameId.get(gameId);
        if (item) void launchItem(item);
      }
      return;
    }
    if (kind === 'activity') {
      if (itemsByGameId.has(gameId)) setSelectedGameId(gameId);
      return;
    }
    if (kind === 'collection') {
      openLibraryCollection(collectionTargetForId(value));
      return;
    }
    if (focusId === 'settings:open') {
      setNotificationsOpen(false);
      setSettingsOpen(true);
      return;
    }
    if (kind === 'downloads' && rest[0] === 'open') {
      setActiveView('downloads');
      return;
    }
    if (kind === 'library' && rest[0] === 'open') {
      setActiveView('library');
      return;
    }
    if (focusId === 'downloads:open') setActiveView('downloads');
    if (focusId === 'library:open') setActiveView('library');
  }, [checkAppUpdate, executePrimaryAction, installAppUpdate, itemsByGameId, launchItem, openLibraryCollection, refreshAll, runAction, setActiveView, setSelectedGameId]);

  useEffect(() => {
    document.querySelectorAll<HTMLElement>('[data-focus-active="true"]').forEach((element) => {
      element.removeAttribute('data-focus-active');
    });

    if (!focusedItemId) return;
    document
      .querySelector<HTMLElement>(`[data-focus-id="${cssEscape(focusedItemId)}"]`)
      ?.setAttribute('data-focus-active', 'true');
  }, [activeView, downloads.length, focusedItemId, items.length, notificationsOpen, selectedGameId, settingsOpen]);

  useGamepad({
    focusedItemId,
    setFocusedItemId,
    onActivate: focusActivate,
    onBack: () => {
      if (notificationsOpen) setNotificationsOpen(false);
      else if (selectedGameId) setSelectedGameId(null);
      else if (settingsOpen) setSettingsOpen(false);
      else setActiveView('home');
    },
    onMenu: (focusId) => {
      const gameId = focusId?.startsWith('game:') ? safeDecodeURIComponent(focusId.split(':').at(-1) ?? '') : null;
      if (gameId) setSelectedGameId(gameId);
    }
  });

  const bannerMessage = message || settingsMessage || launcherMessage;

  return (
    <I18nProvider locale={locale}>
      <AppShell
        activeView={activeView}
        repositoriesCount={storeRepositories.length}
        activeDownloadsCount={activeDownloadItems.length}
        onNavigate={setActiveView}
        onOpenSettings={() => {
          setNotificationsOpen(false);
          setSettingsOpen(true);
        }}
        onFocus={setFocusedItemId}
      >
        <TopChrome
          onRefresh={refreshAll}
          onOpenSettings={() => {
            setNotificationsOpen(false);
            setSettingsOpen(true);
          }}
          onFocus={setFocusedItemId}
          refreshing={busyAction === 'refresh'}
          notificationsOpen={notificationsOpen}
          hasNotificationAlert={hasNotificationAlert}
          updatePanel={updatePanel}
          activityEvents={activityEvents}
          onNotificationsOpenChange={setNotificationsOpen}
          onCheckAppUpdate={checkAppUpdate}
          onInstallAppUpdate={installAppUpdate}
        />
        {bannerMessage && <div className="rh-banner">{bannerMessage}</div>}

        {activeView === 'home' && (
          <HomeScreen
            loading={!dataReady}
            heroItem={homeRails.heroItem}
            rails={homeRails.rails}
            collectionItems={items}
            busyAction={busyAction}
            onPrimaryAction={(item) => void executePrimaryAction(item)}
            onOpenDetails={(game) => setSelectedGameId(game.id)}
            onOpenCollection={openLibraryCollection}
            onOpenSettings={() => setSettingsOpen(true)}
            onFocus={setFocusedItemId}
          />
        )}

        {activeView === 'library' && (
          <LibraryScreen
            items={visibleLibraryItems}
            allItems={items}
            totalCount={items.length}
            filter={libraryFilter}
            query={librarySearch}
            sort={librarySort}
            busyAction={busyAction}
            onFilterChange={setLibraryFilter}
            onQueryChange={setLibrarySearch}
            onSortChange={setLibrarySort}
            onPrimaryAction={(item) => void executePrimaryAction(item)}
            onOpenDetails={(game) => setSelectedGameId(game.id)}
            onFocus={setFocusedItemId}
          />
        )}

        {activeView === 'downloads' && (
          <DownloadsScreen
            downloads={downloads}
            itemsByGameId={itemsByGameId}
            busyAction={busyAction}
            onOpenDetails={(game) => setSelectedGameId(game.id)}
            onPause={(gameId) => runAction(`pause:${gameId}`, () => api.pauseDownload(gameId))}
            onResume={(gameId) => {
              const item = itemsByGameId.get(gameId);
              const record = downloads.find((download) => download.gameId === gameId) ?? item?.download;
              return runAction(`resume:${gameId}`, () => (
                isDirectGameDownload(item?.game, record)
                  ? api.startGameDownload(gameId)
                  : api.resumeDownload(gameId)
              ));
            }}
            onCancel={(gameId) => runAction(`cancel:${gameId}`, () => api.cancelDownload(gameId))}
            onPlay={(item) => void launchItem(item)}
            onFocus={setFocusedItemId}
          />
        )}

        {activeView === 'explore' && (
          <ExploreScreen
            events={activityEvents}
            items={items}
            onOpenEvent={(event) => {
              if (event.gameId) setSelectedGameId(event.gameId);
            }}
            onFocus={setFocusedItemId}
          />
        )}

        {activeView === 'collections' && (
          <CollectionsScreen
            items={items}
            onOpenCollection={openLibraryCollection}
            onFocus={setFocusedItemId}
          />
        )}

        {selectedGame && (
          <GameDetailsModal
            game={selectedGame}
            settings={settings}
            onOpenSettings={() => {
              setSelectedGameId(null);
              setSettingsOpen(true);
            }}
            onClose={() => setSelectedGameId(null)}
            onRefresh={async () => {
              await refreshLauncherData();
              setSettings(await loadSettings());
              await onRefresh();
            }}
          />
        )}

        {launchFailure && (
          <LaunchErrorModal
            failure={launchFailure}
            onClose={() => setLaunchFailure(null)}
            onOpenSettings={() => {
              setLaunchFailure(null);
              setSettingsOpen(true);
            }}
            onOpenDetails={() => {
              if (launchFailure.gameId) setSelectedGameId(launchFailure.gameId);
              setLaunchFailure(null);
            }}
            onRetryDownload={() => {
              const item = launchFailure.gameId ? itemsByGameId.get(launchFailure.gameId) : null;
              setLaunchFailure(null);
              if (item) void installItem(item);
            }}
          />
        )}

        {settingsOpen && (
          <SettingsModal
            settings={settings}
            repositories={storeRepositories}
            downloads={downloads}
            busyAction={busyAction}
            healthReport={healthReport}
            updatePanel={updatePanel}
            sourceUrl={sourceUrl}
            sourcePreview={sourcePreview}
            onClose={() => setSettingsOpen(false)}
            onSave={persistSettings}
            onSourceUrlChange={updateSourceUrl}
            onPreviewRepositoryUrl={previewRepositoryUrl}
            onConnectRepositoryUrl={connectRepositoryUrl}
            onConnectRepositoryFile={connectRepositoryFile}
            onDisconnect={disconnect}
            onRefreshRepository={refreshRepository}
            onRunHealth={runHealthCheck}
            onCopyDiagnostics={copyDiagnostics}
            onOpenLogs={openLogs}
            onCheckAppUpdate={checkAppUpdate}
            onInstallAppUpdate={installAppUpdate}
          />
        )}
      </AppShell>
    </I18nProvider>
  );
}
