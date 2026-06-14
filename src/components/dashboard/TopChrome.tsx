import { useEffect, useRef, useState } from 'react';
import { AlertTriangle, Bell, CheckCircle2, Download, Loader2, RefreshCcw, Search, Settings } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import type { ActivityEvent } from '@/stores/launcherStore';
import type { UpdatePanelPhase, UpdatePanelState } from './types';
import { ActivityIcon } from './shared';
import { displayActivityTitle, formatClock, formatEventTime, updateNotificationText } from './utils';

export function TopChrome({
  onRefresh,
  onOpenSettings,
  onFocus,
  refreshing,
  notificationsOpen,
  hasNotificationAlert,
  updatePanel,
  activityEvents,
  onNotificationsOpenChange,
  onCheckAppUpdate,
  onInstallAppUpdate
}: {
  onRefresh: () => void;
  onOpenSettings: () => void;
  onFocus: (focusId: string) => void;
  refreshing: boolean;
  notificationsOpen: boolean;
  hasNotificationAlert: boolean;
  updatePanel: UpdatePanelState;
  activityEvents: ActivityEvent[];
  onNotificationsOpenChange: (open: boolean) => void;
  onCheckAppUpdate: () => Promise<void>;
  onInstallAppUpdate: () => Promise<void>;
}) {
  const [now, setNow] = useState(() => new Date());
  const [searchValue, setSearchValue] = useState('');
  const actionsRef = useRef<HTMLDivElement>(null);
  const updateCheckedFromPopoverRef = useRef(false);
  const { locale, t } = useI18n();

  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 30_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!notificationsOpen || updatePanel.phase !== 'idle' || updateCheckedFromPopoverRef.current) return;
    updateCheckedFromPopoverRef.current = true;
    void onCheckAppUpdate();
  }, [notificationsOpen, onCheckAppUpdate, updatePanel.phase]);

  useEffect(() => {
    if (!notificationsOpen) return undefined;

    const handlePointerDown = (event: PointerEvent) => {
      if (!(event.target instanceof Node)) return;
      if (!actionsRef.current?.contains(event.target)) onNotificationsOpenChange(false);
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onNotificationsOpenChange(false);
      }
    };

    document.addEventListener('pointerdown', handlePointerDown);
    window.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown);
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, [notificationsOpen, onNotificationsOpenChange]);

  return (
    <header className="rh-topbar">
      <label className="rh-global-search" aria-label={t.dashboard.topbar.searchAria}>
        <Search className="h-4 w-4 text-white/42" />
        <input
          value={searchValue}
          onChange={(event) => setSearchValue(event.target.value)}
          placeholder={t.dashboard.topbar.searchPlaceholder}
        />
      </label>
      <div ref={actionsRef} className="rh-topbar-actions">
        <button
          data-focus-id="top:refresh"
          data-focus-zone="topbar"
          onFocus={() => onFocus('top:refresh')}
          onClick={onRefresh}
          disabled={refreshing}
          className="rh-icon-button rh-focusable"
          title={t.dashboard.topbar.refreshTitle}
        >
          {refreshing ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCcw className="h-4 w-4" />}
        </button>
        <div className="rh-notification-root">
          <button
            id="rh-notifications-trigger"
            data-testid="top-notifications"
            data-focus-id="top:notifications"
            data-focus-zone="topbar"
            onFocus={() => onFocus('top:notifications')}
            onClick={() => onNotificationsOpenChange(!notificationsOpen)}
            className="rh-icon-button rh-notification-button rh-focusable"
            title={t.dashboard.topbar.notificationsTitle}
            aria-label={t.dashboard.topbar.notificationsTitle}
            aria-expanded={notificationsOpen}
            aria-controls="rh-notifications-popover"
          >
            <Bell className="h-4 w-4" />
            {hasNotificationAlert && (
              <span className="rh-notification-dot">
                <span className="sr-only">{t.dashboard.topbar.notifications.unread}</span>
              </span>
            )}
          </button>
          {notificationsOpen && (
            <TopNotificationsPopover
              state={updatePanel}
              events={activityEvents}
              onFocus={onFocus}
              onCheck={onCheckAppUpdate}
              onInstall={onInstallAppUpdate}
            />
          )}
        </div>
        <button
          data-testid="top-settings"
          data-focus-id="top:settings"
          data-focus-zone="topbar"
          onFocus={() => onFocus('top:settings')}
          onClick={onOpenSettings}
          className="rh-icon-button rh-focusable"
          title={t.dashboard.topbar.settingsTitle}
        >
          <Settings className="h-4 w-4" />
        </button>
        <div className="rh-clock">{formatClock(now, locale)}</div>
      </div>
    </header>
  );
}

function TopNotificationsPopover({
  state,
  events,
  onFocus,
  onCheck,
  onInstall
}: {
  state: UpdatePanelState;
  events: ActivityEvent[];
  onFocus: (focusId: string) => void;
  onCheck: () => Promise<void>;
  onInstall: () => Promise<void>;
}) {
  const { locale, t } = useI18n();
  const checking = state.phase === 'checking';
  const installing = state.phase === 'installing';
  const busy = checking || installing;
  const visibleEvents = events.slice(0, 5);

  return (
    <section
      id="rh-notifications-popover"
      className="rh-notifications-popover"
      aria-labelledby="rh-notifications-title"
    >
      <div className="rh-notifications-header">
        <div>
          <div id="rh-notifications-title" className="rh-notifications-title">{t.dashboard.topbar.notifications.title}</div>
          <div className="rh-notifications-subtitle">{t.dashboard.topbar.notifications.subtitle}</div>
        </div>
        {state.phase === 'available' && (
          <span className="rh-notifications-badge">{t.dashboard.topbar.notifications.availableBadge}</span>
        )}
      </div>

      <div className={`rh-notification-update rh-notification-update-${state.phase}`}>
        <div className="rh-notification-update-heading">
          <UpdateStatusIcon phase={state.phase} />
          <div className="min-w-0">
            <div className="rh-notification-section-title">{t.dashboard.topbar.notifications.updateTitle}</div>
            <div className="rh-notification-status-text">{updateNotificationText(state, t, locale)}</div>
          </div>
        </div>
        <div className="rh-notification-actions">
          <button
            type="button"
            data-focus-id="top:update-check"
            data-focus-zone="topbar"
            onFocus={() => onFocus('top:update-check')}
            onClick={() => void onCheck()}
            disabled={busy}
            className="rh-mini-action rh-focusable"
          >
            {checking ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCcw className="h-3.5 w-3.5" />}
            {state.phase === 'error' ? t.settings.updates.retry : t.settings.updates.check}
          </button>
          {state.phase === 'available' && (
            <button
              type="button"
              data-focus-id="top:update-install"
              data-focus-zone="topbar"
              onFocus={() => onFocus('top:update-install')}
              onClick={() => void onInstall()}
              disabled={busy}
              className="rh-mini-action rh-focusable"
            >
              {installing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Download className="h-3.5 w-3.5" />}
              {t.settings.updates.installNow}
            </button>
          )}
        </div>
        {state.phase === 'available' && state.report?.body && (
          <div className="rh-notification-release">
            <div className="rh-notification-release-label">{t.dashboard.topbar.notifications.releaseNotes}</div>
            <div>{state.report.body}</div>
          </div>
        )}
        {state.phase === 'available' && state.report?.date && (
          <div className="rh-notification-published">{t.common.published} {state.report.date}</div>
        )}
      </div>

      <div className="rh-notification-feed-heading">{t.dashboard.topbar.notifications.activityTitle}</div>
      <div className="rh-notification-feed">
        {visibleEvents.length === 0 ? (
          <div className="rh-notification-empty">{t.dashboard.topbar.notifications.empty}</div>
        ) : visibleEvents.map((event) => (
          <div key={event.id} className="rh-notification-event">
            <ActivityIcon tone={event.tone} />
            <div className="min-w-0 flex-1">
              <div className="rh-notification-event-title">{displayActivityTitle(event.title, t)}</div>
              <div className="rh-notification-event-detail">{event.detail}</div>
            </div>
            <div className="rh-notification-event-time">{formatEventTime(event.timestamp, locale)}</div>
          </div>
        ))}
      </div>
    </section>
  );
}

function UpdateStatusIcon({ phase }: { phase: UpdatePanelPhase }) {
  const iconClass = 'h-4 w-4';
  if (phase === 'checking' || phase === 'installing') return <Loader2 className={`${iconClass} animate-spin text-fusion-accent`} />;
  if (phase === 'available') return <Download className={`${iconClass} text-fusion-green`} />;
  if (phase === 'error') return <AlertTriangle className={`${iconClass} text-red-200`} />;
  if (phase === 'up-to-date') return <CheckCircle2 className={`${iconClass} text-fusion-green`} />;
  return <Bell className={`${iconClass} text-white/58`} />;
}
