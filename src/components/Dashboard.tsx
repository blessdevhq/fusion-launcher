'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { motion } from 'framer-motion';
import {
  Activity,
  AlertTriangle,
  Ban,
  CheckCircle2,
  Clipboard,
  Download,
  FolderOpen,
  Gamepad2,
  HeartPulse,
  Loader2,
  Pause,
  Play,
  RefreshCcw,
  RotateCw,
  Search,
  Settings,
  ShieldAlert
} from 'lucide-react';
import { GameDetailsModal } from '@/components/GameDetailsModal';
import { LaunchErrorModal } from '@/components/LaunchErrorModal';
import { SettingsModal } from '@/components/SettingsModal';
import { AppShell } from '@/components/shell/AppShell';
import {
  CollectionsPanel,
  HeroPanel
} from '@/components/shell/CockpitPanels';
import { GameArt, GamePoster } from '@/components/shell/GamePoster';
import { useGamepad } from '@/hooks/useGamepad';
import {
  buildGameLibraryItems,
  filterLibraryItems,
  type GameLibraryItem,
  type LibraryFilter
} from '@/lib/libraryStatus';
import { api } from '@/lib/api';
import { normalizeLaunchFailure } from '@/lib/launchErrors';
import { isTauriRuntime } from '@/lib/runtime';
import { loadSettings, saveSettings, type AppSettings } from '@/lib/settings';
import { useLauncherStore, type ActivityEvent, type LauncherView } from '@/stores/launcherStore';
import type {
  CatalogGame,
  DownloadProgressEvent,
  HealthReport,
  LaunchFailure,
  RepositorySummary,
  TorrentDownloadRecord,
  TorrentDownloadStatus
} from '@/types/repository';

type BusyAction = string | null;

interface DashboardProps {
  catalog: CatalogGame[];
  repositories: RepositorySummary[];
  message: string | null;
  onDisconnectRepository: (repositoryId: string) => Promise<void>;
  onRefresh: () => Promise<void>;
}

const FILTERS: Array<{ id: LibraryFilter; label: string }> = [
  { id: 'all', label: 'All' },
  { id: 'installed', label: 'Installed' },
  { id: 'downloading', label: 'Downloading' },
  { id: 'missing', label: 'Missing Requirements' }
];

const ACTIVE_DOWNLOAD_STATUSES: TorrentDownloadStatus[] = ['resolving', 'downloading', 'cancelling'];
const RESUMABLE_DOWNLOAD_STATUSES: TorrentDownloadStatus[] = ['paused', 'interrupted', 'error'];

export function Dashboard({
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
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsMessage, setSettingsMessage] = useState<string | null>(null);
  const [launcherMessage, setLauncherMessage] = useState<string | null>(null);
  const [busyAction, setBusyAction] = useState<BusyAction>(null);
  const [launchFailure, setLaunchFailure] = useState<LaunchFailure | null>(null);
  const [healthReport, setHealthReport] = useState<HealthReport | null>(null);

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
        if (!cancelled) setSettingsMessage(`Failed to load settings: ${error}`);
      }
    };

    void loadPersistedSettings();
    return () => {
      cancelled = true;
    };
  }, [setSettings]);

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
  const readyItems = useMemo(() => items.filter((item) => item.readyToPlay), [items]);
  const installedItems = useMemo(() => items.filter((item) => item.installed), [items]);
  const activeDownloadItems = useMemo(
    () => items.filter((item) => item.isDownloading || item.isPaused || item.hasError),
    [items]
  );
  const recentItems = useMemo(() => items.slice(0, 14), [items]);
  const heroItem = readyItems[0] ?? installedItems[0] ?? activeDownloadItems[0] ?? items[0] ?? null;
  const selectedGame = selectedGameId ? storeCatalog.find((game) => game.id === selectedGameId) ?? null : null;
  const visibleLibraryItems = useMemo(() => filterLibraryItems(items, libraryFilter), [items, libraryFilter]);

  const persistSettings = async (nextSettings: AppSettings) => {
    const savedSettings = await saveSettings(nextSettings);
    setSettings(savedSettings);
    setSettingsMessage(null);
    await refreshLauncherData();
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
        title: 'Diagnostics copied',
        detail: bundle.logPath,
        tone: 'success'
      });
    });
  };

  const startDownload = async (item: GameLibraryItem) => {
    await runAction(`download:${item.game.id}`, async () => {
      await api.startGameDownload(item.game.id);
      addActivityEvent({
        title: 'Download started',
        detail: item.game.title,
        gameId: item.game.id,
        tone: 'info'
      });
    });
  };

  const launchItem = async (item: GameLibraryItem) => {
    setBusyAction(`play:${item.game.id}`);
    setLauncherMessage(null);
    setLaunchFailure(null);
    try {
      await api.launchGame(item.game.id);
      addActivityEvent({
        title: 'Launch requested',
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
    if (item.primaryAction === 'download') return startDownload(item);
    if (item.primaryAction === 'resume' || item.primaryAction === 'retry') {
      return runAction(`resume:${item.game.id}`, () => api.resumeDownload(item.game.id));
    }
    setSelectedGameId(item.game.id);
  };

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
      if (value === 'search') setLauncherMessage('Search is staged for the next UI pass.');
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
      if (downloadAction === 'resume' || downloadAction === 'retry') void runAction(`resume:${gameId}`, () => api.resumeDownload(gameId));
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
    if (focusId === 'settings:open') {
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
  }, [executePrimaryAction, itemsByGameId, launchItem, refreshAll, runAction, setActiveView, setSelectedGameId]);

  useEffect(() => {
    document.querySelectorAll<HTMLElement>('[data-focus-active="true"]').forEach((element) => {
      element.removeAttribute('data-focus-active');
    });

    if (!focusedItemId) return;
    document
      .querySelector<HTMLElement>(`[data-focus-id="${cssEscape(focusedItemId)}"]`)
      ?.setAttribute('data-focus-active', 'true');
  }, [activeView, downloads.length, focusedItemId, items.length, selectedGameId, settingsOpen]);

  useGamepad({
    focusedItemId,
    setFocusedItemId,
    onActivate: focusActivate,
    onBack: () => {
      if (selectedGameId) setSelectedGameId(null);
      else if (settingsOpen) setSettingsOpen(false);
      else setActiveView('home');
    },
    onSearch: () => setLauncherMessage('Search is staged for the next UI pass.'),
    onMenu: (focusId) => {
      const gameId = focusId?.startsWith('game:') ? safeDecodeURIComponent(focusId.split(':').at(-1) ?? '') : null;
      if (gameId) setSelectedGameId(gameId);
    }
  });

  const bannerMessage = message || settingsMessage || launcherMessage;

  return (
    <AppShell
      activeView={activeView}
      repositoriesCount={storeRepositories.length}
      activeDownloadsCount={activeDownloadItems.length}
      onNavigate={setActiveView}
      onFocus={setFocusedItemId}
    >
      <TopChrome
        onRefresh={refreshAll}
        onSearch={() => setLauncherMessage('Search is staged for the next UI pass.')}
        onFocus={setFocusedItemId}
        refreshing={busyAction === 'refresh'}
      />
      {bannerMessage && <div className="rh-banner">{bannerMessage}</div>}

      {activeView === 'home' && (
        <HomeScreen
          heroItem={heroItem}
          readyItems={readyItems}
          activeDownloadItems={activeDownloadItems}
          recentItems={recentItems}
          busyAction={busyAction}
          onPrimaryAction={(item) => void executePrimaryAction(item)}
          onOpenDetails={(game) => setSelectedGameId(game.id)}
          onFocus={setFocusedItemId}
        />
      )}

      {activeView === 'library' && (
        <LibraryScreen
          items={visibleLibraryItems}
          allItems={items}
          totalCount={items.length}
          filter={libraryFilter}
          busyAction={busyAction}
          onFilterChange={setLibraryFilter}
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
          onResume={(gameId) => runAction(`resume:${gameId}`, () => api.resumeDownload(gameId))}
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
          onOpenLibrary={() => setActiveView('library')}
          onFocus={setFocusedItemId}
        />
      )}

      {activeView === 'settings' && (
        <SettingsScreen
          repositories={storeRepositories}
          settings={settings}
          busyAction={busyAction}
          healthReport={healthReport}
          onOpenSettings={() => setSettingsOpen(true)}
          onDisconnect={disconnect}
          onRefresh={refreshAll}
          onRefreshRepository={refreshRepository}
          onRunHealth={runHealthCheck}
          onCopyDiagnostics={copyDiagnostics}
          onOpenLogs={() => api.openLogsFolder()}
        />
      )}

      {selectedGame && (
        <GameDetailsModal
          game={selectedGame}
          settings={settings}
          onOpenSettings={() => {
            setSelectedGameId(null);
            setActiveView('settings');
            setSettingsOpen(true);
          }}
          onClose={() => setSelectedGameId(null)}
          onRefresh={async () => {
            await refreshLauncherData();
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
            setActiveView('settings');
            setSettingsOpen(true);
          }}
          onOpenDetails={() => {
            if (launchFailure.gameId) setSelectedGameId(launchFailure.gameId);
            setLaunchFailure(null);
          }}
          onRetryDownload={() => {
            const item = launchFailure.gameId ? itemsByGameId.get(launchFailure.gameId) : null;
            setLaunchFailure(null);
            if (item) void startDownload(item);
          }}
        />
      )}

      {settingsOpen && (
        <SettingsModal
          settings={settings}
          onClose={() => setSettingsOpen(false)}
          onSave={persistSettings}
        />
      )}
    </AppShell>
  );
}

function TopChrome({
  onRefresh,
  onSearch,
  onFocus,
  refreshing
}: {
  onRefresh: () => void;
  onSearch: () => void;
  onFocus: (focusId: string) => void;
  refreshing: boolean;
}) {
  return (
    <header className="rh-topbar">
      <div className="flex items-center gap-3 text-xs font-black uppercase tracking-[0.18em] text-white/70">
        <Gamepad2 className="h-4 w-4 text-hydra-accent" />
        Console Shell
      </div>
      <div className="flex items-center gap-3">
        <button
          data-focus-id="top:search"
          data-focus-zone="topbar"
          onFocus={() => onFocus('top:search')}
          onClick={onSearch}
          className="rh-icon-button rh-focusable"
          title="Search"
        >
          <Search className="h-4 w-4" />
        </button>
        <button
          data-focus-id="top:refresh"
          data-focus-zone="topbar"
          onFocus={() => onFocus('top:refresh')}
          onClick={onRefresh}
          disabled={refreshing}
          className="rh-icon-button rh-focusable"
          title="Refresh repository"
        >
          {refreshing ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCcw className="h-4 w-4" />}
        </button>
        <div className="min-w-[74px] text-right text-sm font-black">{formatClock(new Date())}</div>
      </div>
    </header>
  );
}

function HomeScreen({
  heroItem,
  readyItems,
  activeDownloadItems,
  recentItems,
  busyAction,
  onPrimaryAction,
  onOpenDetails,
  onFocus
}: {
  heroItem: GameLibraryItem | null;
  readyItems: GameLibraryItem[];
  activeDownloadItems: GameLibraryItem[];
  recentItems: GameLibraryItem[];
  busyAction: BusyAction;
  onPrimaryAction: (item: GameLibraryItem) => void;
  onOpenDetails: (game: CatalogGame) => void;
  onFocus: (focusId: string) => void;
}) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      className="rh-home-screen"
    >
      <HeroPanel
        heroItem={heroItem}
        continueItems={readyItems.length > 0 ? readyItems : activeDownloadItems}
        recentItems={recentItems}
        busyAction={busyAction}
        onPrimaryAction={onPrimaryAction}
        onOpenDetails={onOpenDetails}
        onFocus={onFocus}
      />
    </motion.div>
  );
}

function CollectionsScreen({
  items,
  onOpenLibrary,
  onFocus
}: {
  items: GameLibraryItem[];
  onOpenLibrary: () => void;
  onFocus: (focusId: string) => void;
}) {
  return (
    <motion.section initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} className="rh-screen rh-panel">
      <CollectionsPanel items={items} onOpenLibrary={onOpenLibrary} onFocus={onFocus} />
    </motion.section>
  );
}

function ExploreScreen({
  events,
  items,
  onOpenEvent,
  onFocus
}: {
  events: ReturnType<typeof useLauncherStore.getState>['activityEvents'];
  items: GameLibraryItem[];
  onOpenEvent: (event: ActivityEvent) => void;
  onFocus: (focusId: string) => void;
}) {
  const visible = events.length > 0 ? events : buildFallbackEvents(items);

  return (
    <motion.section initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} className="rh-screen rh-panel">
      <ScreenHeader eyebrow="Explore" title="Activity Feed" description="Recent repository, library, and download events" />
      <div className="rh-explore-layout">
        <div className="rh-activity-list">
          {visible.slice(0, 12).map((event, index) => {
            const focusId = `activity:${encodeURIComponent(event.gameId ?? event.id)}`;
            return (
              <button
                key={event.id}
                data-focus-id={focusId}
                data-focus-zone="activity"
                onFocus={() => onFocus(focusId)}
                onClick={() => onOpenEvent(event)}
                className="rh-activity-row rh-focusable"
              >
                <ActivityIcon tone={event.tone} />
                <div className="min-w-0">
                  <div className="truncate text-sm font-bold">{event.title}</div>
                  <div className="truncate text-xs text-white/42">{event.detail}</div>
                </div>
                <div className="ml-auto text-[10px] uppercase text-white/34">{formatEventTime(event.timestamp)}</div>
              </button>
            );
          })}
        </div>
        <div className="rh-explore-stats">
          <div className="text-[10px] font-black uppercase tracking-wide text-white/42">Library Stats</div>
          <StatsLine label="Games" value={String(items.length)} />
          <StatsLine label="Ready" value={String(items.filter((item) => item.readyToPlay).length)} />
          <StatsLine label="Downloading" value={String(items.filter((item) => item.isDownloading || item.isPaused || item.hasError).length)} />
        </div>
      </div>
    </motion.section>
  );
}

function LibraryScreen({
  items,
  allItems,
  totalCount,
  filter,
  busyAction,
  onFilterChange,
  onPrimaryAction,
  onOpenDetails,
  onFocus
}: {
  items: GameLibraryItem[];
  allItems: GameLibraryItem[];
  totalCount: number;
  filter: LibraryFilter;
  busyAction: BusyAction;
  onFilterChange: (filter: LibraryFilter) => void;
  onPrimaryAction: (item: GameLibraryItem) => void;
  onOpenDetails: (game: CatalogGame) => void;
  onFocus: (focusId: string) => void;
}) {
  return (
    <motion.section initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} className="rh-screen rh-panel">
      <ScreenHeader eyebrow="Library" title="Installed Games & Catalog" description={`${items.length} visible / ${totalCount} total games`} />
      <div className="mb-4 flex flex-wrap gap-2">
        {FILTERS.map((item) => (
          <button
            key={item.id}
            data-focus-id={`filter:${item.id}`}
            data-focus-zone="library-filters"
            onFocus={() => onFocus(`filter:${item.id}`)}
            onClick={() => onFilterChange(item.id)}
            className={`rh-filter-chip rh-focusable ${filter === item.id ? 'rh-filter-chip-active' : ''}`}
          >
            {item.label}
            <span>{filterCountLabel(item.id, totalCount, allItems)}</span>
          </button>
        ))}
      </div>
      <div className="rh-library-grid">
        {items.map((item) => (
          <GamePoster
            key={item.game.id}
            item={item}
            focusId={`game:library:${encodeURIComponent(item.game.id)}`}
            zone="library"
            selected={busyAction?.endsWith(item.game.id)}
            onOpen={onOpenDetails}
            onAction={onPrimaryAction}
            onFocus={onFocus}
          />
        ))}
      </div>
    </motion.section>
  );
}

function DownloadsScreen({
  downloads,
  itemsByGameId,
  busyAction,
  onOpenDetails,
  onPause,
  onResume,
  onCancel,
  onPlay,
  onFocus
}: {
  downloads: TorrentDownloadRecord[];
  itemsByGameId: Map<string, GameLibraryItem>;
  busyAction: BusyAction;
  onOpenDetails: (game: CatalogGame) => void;
  onPause: (gameId: string) => Promise<void>;
  onResume: (gameId: string) => Promise<void>;
  onCancel: (gameId: string) => Promise<void>;
  onPlay: (item: GameLibraryItem) => void;
  onFocus: (focusId: string) => void;
}) {
  const summary = summarizeDownloads(downloads);

  return (
    <motion.section initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} className="rh-screen rh-panel">
      <ScreenHeader eyebrow="Downloads" title="Download Center" description={`${downloads.length} torrent records`} />
      <div className="rh-download-summary">
        <DownloadMetric label="Active" value={String(summary.active)} tone="active" />
        <DownloadMetric label="Paused" value={String(summary.paused)} tone="paused" />
        <DownloadMetric label="Errors" value={String(summary.errors)} tone="error" />
        <DownloadMetric label="Downloaded" value={formatBytes(summary.downloadedBytes)} tone="ready" />
      </div>
      <div className="rh-download-list">
        {downloads.length === 0 ? (
          <div className="rh-empty-compact">No downloads yet.</div>
        ) : downloads.map((download) => {
          const item = itemsByGameId.get(download.gameId) ?? null;
          return (
            <DownloadRow
              key={download.gameId}
              download={download}
              item={item}
              busyAction={busyAction}
              onOpenDetails={onOpenDetails}
              onPause={onPause}
              onResume={onResume}
              onCancel={onCancel}
              onPlay={onPlay}
              onFocus={onFocus}
            />
          );
        })}
      </div>
    </motion.section>
  );
}

function SettingsScreen({
  repositories,
  settings,
  busyAction,
  healthReport,
  onOpenSettings,
  onDisconnect,
  onRefresh,
  onRefreshRepository,
  onRunHealth,
  onCopyDiagnostics,
  onOpenLogs
}: {
  repositories: RepositorySummary[];
  settings: AppSettings;
  busyAction: BusyAction;
  healthReport: HealthReport | null;
  onOpenSettings: () => void;
  onDisconnect: (repositoryId: string) => Promise<void>;
  onRefresh: () => Promise<void>;
  onRefreshRepository: (repositoryId: string) => Promise<void>;
  onRunHealth: () => Promise<void>;
  onCopyDiagnostics: () => Promise<void>;
  onOpenLogs: () => Promise<void>;
}) {
  const configured = Object.values(settings.emulators).filter(Boolean).length;

  return (
    <motion.section initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} className="rh-screen rh-panel">
      <ScreenHeader eyebrow="Settings" title="Repository & Emulator Setup" description={`${configured} emulator paths configured`} />
      <div className="grid gap-4 lg:grid-cols-[360px_minmax(0,1fr)]">
        <div className="rounded-md border border-white/10 bg-black/24 p-5">
          <Settings className="h-7 w-7 text-hydra-accent" />
          <div className="mt-4 text-lg font-black">Emulators</div>
          <p className="mt-1 text-sm text-white/46">Per-platform executable paths and readiness checks.</p>
          <button onClick={onOpenSettings} className="rh-primary-action mt-5">Open Emulator Settings</button>
        </div>
        <div className="rounded-md border border-white/10 bg-black/24 p-5">
          <div className="mb-4 flex items-center justify-between">
            <div className="font-black uppercase">Repositories</div>
            <button onClick={onRefresh} disabled={busyAction === 'refresh'} className="rh-icon-button">
              {busyAction === 'refresh' ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCcw className="h-4 w-4" />}
            </button>
          </div>
          <div className="grid gap-3 md:grid-cols-2">
            {repositories.map((repository) => (
              <div key={repository.id} className="rounded-md border border-white/10 bg-white/[0.04] p-4">
                <div className="truncate text-sm font-bold">{repository.name}</div>
                <div className="mt-1 truncate text-xs text-white/36">{repository.url}</div>
                <div className="mt-3 text-xs text-white/46">{repository.catalogCount} games / {repository.systemFileCount} system files</div>
                <div className="mt-4 flex flex-wrap gap-2">
                  <button
                    onClick={() => onRefreshRepository(repository.id)}
                    disabled={busyAction === `repo-refresh:${repository.id}`}
                    className="inline-flex h-8 items-center gap-2 rounded-md border border-white/10 px-3 text-xs font-bold text-white/70"
                  >
                    <RefreshCcw className="h-3.5 w-3.5" />
                    Refresh
                  </button>
                  <button
                    onClick={() => onDisconnect(repository.id)}
                    disabled={busyAction === `repo:${repository.id}`}
                    className="inline-flex h-8 items-center gap-2 rounded-md border border-red-300/20 px-3 text-xs font-bold text-red-100/80"
                  >
                    <Ban className="h-3.5 w-3.5" />
                    Remove
                  </button>
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>
      <div className="mt-4 rounded-md border border-white/10 bg-black/24 p-5">
        <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
          <div>
            <div className="flex items-center gap-2 font-black uppercase">
              <HeartPulse className="h-4 w-4 text-hydra-green" />
              Health Check
            </div>
            <div className="mt-1 text-sm text-white/46">Emulators, files, repositories, and downloader session.</div>
          </div>
          <div className="flex flex-wrap gap-2">
            <button onClick={onRunHealth} disabled={busyAction === 'health'} className="rh-mini-action">
              {busyAction === 'health' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <HeartPulse className="h-3.5 w-3.5" />}
              Run
            </button>
            <button onClick={onCopyDiagnostics} disabled={busyAction === 'diagnostics'} className="rh-mini-action">
              {busyAction === 'diagnostics' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Clipboard className="h-3.5 w-3.5" />}
              Copy diagnostics
            </button>
            <button onClick={onOpenLogs} className="rh-mini-action">
              <FolderOpen className="h-3.5 w-3.5" />
              Open logs
            </button>
          </div>
        </div>
        {healthReport ? (
          <div className="grid gap-3 lg:grid-cols-2">
            <HealthGroup title="Emulators" items={healthReport.emulators} />
            <HealthGroup title="System Files" items={healthReport.systemFiles} />
            <HealthGroup title="Game Files" items={healthReport.gameFiles} />
            <HealthGroup title="Repositories" items={[...healthReport.repositories, healthReport.downloader]} />
          </div>
        ) : (
          <div className="rh-empty-compact">Run diagnostics to inspect MVP readiness.</div>
        )}
      </div>
    </motion.section>
  );
}

function HealthGroup({ title, items }: { title: string; items: HealthReport['emulators'] }) {
  return (
    <div className="rounded-md border border-white/10 bg-white/[0.04] p-4">
      <div className="mb-3 text-xs font-black uppercase tracking-wide text-white/42">{title}</div>
      <div className="space-y-2">
        {items.length === 0 ? (
          <div className="text-xs text-white/36">No records yet.</div>
        ) : items.map((item) => (
          <div key={item.id} className="flex items-start gap-3 rounded border border-white/8 bg-black/16 px-3 py-2 text-xs">
            <span className={`mt-1 h-2 w-2 rounded-full ${healthToneClass(item.status)}`} />
            <div className="min-w-0 flex-1">
              <div className="truncate font-bold text-white/82">{item.label}</div>
              <div className="mt-1 text-white/42">{item.message ?? item.status}</div>
            </div>
            <span className="rounded border border-white/10 px-2 py-1 uppercase text-white/48">{item.status}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function healthToneClass(status: string) {
  if (status === 'ready') return 'bg-hydra-green';
  if (status === 'corrupt' || status === 'error') return 'bg-red-300';
  return 'bg-amber-300';
}

function DownloadRow({
  download,
  item,
  busyAction,
  onOpenDetails,
  onPause,
  onResume,
  onCancel,
  onPlay,
  onFocus
}: {
  download: TorrentDownloadRecord;
  item: GameLibraryItem | null;
  busyAction: BusyAction;
  onOpenDetails: (game: CatalogGame) => void;
  onPause: (gameId: string) => Promise<void>;
  onResume: (gameId: string) => Promise<void>;
  onCancel: (gameId: string) => Promise<void>;
  onPlay: (item: GameLibraryItem) => void;
  onFocus: (focusId: string) => void;
}) {
  const active = ACTIVE_DOWNLOAD_STATUSES.includes(download.status);
  const resumable = RESUMABLE_DOWNLOAD_STATUSES.includes(download.status);
  const cancellable = !['completed', 'cancelled', 'cancelling'].includes(download.status);

  return (
    <article className="rh-download-row">
      <button
        data-focus-id={`details:${encodeURIComponent(download.gameId)}`}
        data-focus-zone="downloads"
        onFocus={() => onFocus(`details:${encodeURIComponent(download.gameId)}`)}
        onClick={() => item && onOpenDetails(item.game)}
        className="rh-download-art rh-focusable"
      >
        {item ? <GameArt game={item.game} className="h-full w-full" /> : null}
      </button>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <div className="truncate text-sm font-black">{item?.game.title ?? download.gameId}</div>
          <span className="rounded border border-white/10 px-2 py-1 text-[10px] uppercase text-white/54">{download.status}</span>
        </div>
        <div className="mt-2 h-1.5 overflow-hidden rounded bg-black/42">
          <div className="h-full rounded bg-hydra-accent" style={{ width: `${download.status === 'completed' ? 100 : download.progressPercent}%` }} />
        </div>
        <div className="mt-2 flex flex-wrap gap-4 text-xs text-white/42">
          <span>{formatBytes(download.downloadedBytes)} / {formatBytes(download.totalBytes)}</span>
          <span>{formatSpeed(download.downloadSpeedBytesPerSec)}</span>
          <span>{download.peersCount} peers</span>
        </div>
        {download.errorMessage && <div className="mt-2 text-xs text-red-100">{download.errorMessage}</div>}
      </div>
      <div className="flex flex-wrap justify-end gap-2">
        {active && download.status !== 'cancelling' && (
          <IconAction
            focusId={`download-action:pause:${encodeURIComponent(download.gameId)}`}
            onFocus={onFocus}
            busy={busyAction === `pause:${download.gameId}`}
            label="Pause"
            icon={<Pause className="h-3.5 w-3.5" />}
            onClick={() => onPause(download.gameId)}
          />
        )}
        {resumable && (
          <IconAction
            focusId={`download-action:${download.status === 'error' ? 'retry' : 'resume'}:${encodeURIComponent(download.gameId)}`}
            onFocus={onFocus}
            busy={busyAction === `resume:${download.gameId}`}
            label={download.status === 'error' ? 'Retry' : 'Resume'}
            icon={<RotateCw className="h-3.5 w-3.5" />}
            onClick={() => onResume(download.gameId)}
          />
        )}
        {item?.readyToPlay && download.status === 'completed' && (
          <IconAction
            focusId={`download-action:play:${encodeURIComponent(download.gameId)}`}
            onFocus={onFocus}
            busy={busyAction === `play:${download.gameId}`}
            label="Play"
            icon={<Play className="h-3.5 w-3.5" />}
            onClick={() => onPlay(item)}
          />
        )}
        {cancellable && (
          <IconAction
            focusId={`download-action:cancel:${encodeURIComponent(download.gameId)}`}
            onFocus={onFocus}
            busy={busyAction === `cancel:${download.gameId}`}
            label="Cancel"
            icon={<Ban className="h-3.5 w-3.5" />}
            onClick={() => onCancel(download.gameId)}
            danger
          />
        )}
      </div>
    </article>
  );
}

function IconAction({
  label,
  icon,
  busy,
  danger,
  focusId,
  onFocus,
  onClick
}: {
  label: string;
  icon: React.ReactNode;
  busy: boolean;
  danger?: boolean;
  focusId: string;
  onFocus: (focusId: string) => void;
  onClick: () => void;
}) {
  return (
    <button
      data-focus-id={focusId}
      data-focus-zone="download-actions"
      onFocus={() => onFocus(focusId)}
      onClick={onClick}
      className={`rh-mini-action rh-focusable ${danger ? 'rh-mini-action-danger' : ''}`}
    >
      {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : icon}
      {label}
    </button>
  );
}

function ScreenHeader({ eyebrow, title, description }: { eyebrow: string; title: string; description: string }) {
  return (
    <header className="mb-5">
      <div className="text-xs font-black uppercase tracking-[0.18em] text-hydra-accent">{eyebrow}</div>
      <h1 className="mt-2 text-3xl font-black uppercase tracking-normal">{title}</h1>
      <p className="mt-1 text-sm text-white/46">{description}</p>
    </header>
  );
}

function ActivityIcon({ tone }: { tone: ActivityEvent['tone'] }) {
  return (
    <div className="grid h-9 w-9 shrink-0 place-items-center rounded-full bg-white/10">
      {tone === 'success' ? (
        <CheckCircle2 className="h-4 w-4 text-hydra-green" />
      ) : tone === 'error' ? (
        <AlertTriangle className="h-4 w-4 text-red-200" />
      ) : tone === 'warning' ? (
        <ShieldAlert className="h-4 w-4 text-amber-200" />
      ) : (
        <Activity className="h-4 w-4 text-white/60" />
      )}
    </div>
  );
}

function StatsLine({ label, value }: { label: string; value: string }) {
  return (
    <div className="mt-3 flex items-center justify-between text-xs">
      <span className="text-white/48">{label}</span>
      <span className="font-bold">{value}</span>
    </div>
  );
}

function buildFallbackEvents(items: GameLibraryItem[]): ActivityEvent[] {
  const first = items[0];
  return [
    {
      id: 'repo-ready',
      title: 'Repository connected',
      detail: `${items.length} games available`,
      timestamp: new Date().toISOString(),
      tone: 'info'
    },
    {
      id: 'library-updated',
      title: 'Library indexed',
      detail: first ? first.game.title : 'Waiting for catalog entries',
      timestamp: new Date().toISOString(),
      gameId: first?.game.id,
      tone: 'success'
    },
    {
      id: 'settings-check',
      title: 'System check available',
      detail: 'Open Settings to configure emulator paths',
      timestamp: new Date().toISOString(),
      tone: 'warning'
    }
  ];
}

function formatEventTime(timestamp: string) {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return '';
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function formatBytes(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 MB';
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function formatSpeed(bytesPerSecond: number) {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) return '0.00 MB/s';
  return `${(bytesPerSecond / 1024 / 1024).toFixed(2)} MB/s`;
}

function formatClock(date: Date) {
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function filterCountLabel(filter: LibraryFilter, totalCount: number, allItems: GameLibraryItem[]) {
  if (filter === 'all') return totalCount;
  return filterLibraryItems(allItems, filter).length;
}

function summarizeDownloads(downloads: TorrentDownloadRecord[]) {
  return downloads.reduce((summary, download) => {
    if (ACTIVE_DOWNLOAD_STATUSES.includes(download.status)) summary.active += 1;
    if (download.status === 'paused' || download.status === 'interrupted') summary.paused += 1;
    if (download.status === 'error') summary.errors += 1;
    summary.downloadedBytes += download.downloadedBytes;
    return summary;
  }, {
    active: 0,
    paused: 0,
    errors: 0,
    downloadedBytes: 0
  });
}

function DownloadMetric({
  label,
  value,
  tone
}: {
  label: string;
  value: string;
  tone: 'active' | 'paused' | 'error' | 'ready';
}) {
  return (
    <div className={`rh-download-metric rh-download-metric-${tone}`}>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function safeDecodeURIComponent(value: string) {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

function cssEscape(value: string) {
  if (typeof CSS !== 'undefined' && CSS.escape) {
    return CSS.escape(value);
  }

  return value.replace(/"/g, '\\"');
}
