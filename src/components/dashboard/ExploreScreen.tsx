import { motion } from 'framer-motion';
import { useI18n } from '@/components/I18nProvider';
import type { GameLibraryItem } from '@/lib/libraryStatus';
import type { ActivityEvent } from '@/stores/launcherStore';
import { ActivityIcon, ScreenHeader, StatsLine } from './shared';
import { displayActivityTitle, formatEventTime } from './utils';

export function ExploreScreen({
  events,
  items,
  onOpenEvent,
  onFocus
}: {
  events: ActivityEvent[];
  items: GameLibraryItem[];
  onOpenEvent: (event: ActivityEvent) => void;
  onFocus: (focusId: string) => void;
}) {
  const { locale, t } = useI18n();
  return (
    <motion.section initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} className="rh-screen rh-panel" data-testid="explore-screen">
      <ScreenHeader eyebrow={t.dashboard.explore.eyebrow} title={t.dashboard.explore.title} description={t.dashboard.explore.description} />
      <div className="rh-explore-layout">
        <div className="rh-activity-list">
          {events.length === 0 ? (
            <div className="rh-empty-compact">{t.dashboard.explore.empty}</div>
          ) : events.slice(0, 12).map((event) => {
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
                  <div className="truncate text-sm font-bold">{displayActivityTitle(event.title, t)}</div>
                  <div className="truncate text-xs text-white/42">{event.detail}</div>
                </div>
                <div className="ml-auto text-[10px] uppercase text-white/34">{formatEventTime(event.timestamp, locale)}</div>
              </button>
            );
          })}
        </div>
        <div className="rh-explore-stats">
          <div className="text-[10px] font-semibold text-white/42">{t.dashboard.explore.libraryStats}</div>
          <StatsLine label={t.dashboard.explore.games} value={String(items.length)} />
          <StatsLine label={t.dashboard.explore.ready} value={String(items.filter((item) => item.readyToPlay).length)} />
          <StatsLine label={t.dashboard.explore.downloads} value={String(items.filter((item) => item.isDownloading || item.isPaused || item.hasError).length)} />
        </div>
      </div>
    </motion.section>
  );
}
