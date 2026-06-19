'use client';

import { Database, Download, Home, Library, Settings, type LucideIcon } from 'lucide-react';
import { useI18n } from '../I18nProvider';
import type { LauncherView } from '../../stores/launcherStore.ts';

const NAV_ITEMS: Array<{ id: LauncherView; icon: LucideIcon }> = [
  { id: 'home', icon: Home },
  { id: 'library', icon: Library },
  { id: 'downloads', icon: Download }
];

interface SidebarProps {
  activeView: LauncherView;
  repositoriesCount: number;
  activeDownloadsCount: number;
  onNavigate: (view: LauncherView) => void;
  onOpenSettings: () => void;
  onFocus: (focusId: string) => void;
}

export function Sidebar({
  activeView,
  repositoriesCount,
  activeDownloadsCount,
  onNavigate,
  onOpenSettings,
  onFocus
}: SidebarProps) {
  const { t } = useI18n();

  return (
    <aside className="rh-sidebar">
      <div className="rh-brand-block">
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img src="/fusion/logo-lockup.png" alt="fusion" className="rh-brand-lockup" />
        <div className="rh-sidebar-brand">{t.brand.tagline}</div>
      </div>

      <nav className="rh-nav-list">
        {NAV_ITEMS.map((item) => {
          const Icon = item.icon;
          const count = item.id === 'downloads' ? activeDownloadsCount : 0;
          const focusId = `nav:${item.id}`;

          return (
            <button
              key={item.id}
              data-testid={`nav-${item.id}`}
              data-focus-id={focusId}
              data-focus-zone="sidebar"
              onFocus={() => onFocus(focusId)}
              onClick={() => onNavigate(item.id)}
              className={`rh-nav-item rh-focusable ${activeView === item.id ? 'rh-nav-item-active' : ''}`}
            >
              <Icon className="h-4 w-4 shrink-0" />
              <span className="min-w-0 flex-1 truncate">{t.shell.nav[item.id]}</span>
              {count > 0 && <span className="rounded bg-white/10 px-1.5 py-0.5 text-[10px] text-white/76">{count}</span>}
            </button>
          );
        })}
        <button
          data-testid="nav-settings"
          data-focus-id="settings:open"
          data-focus-zone="sidebar"
          onFocus={() => onFocus('settings:open')}
          onClick={onOpenSettings}
          className="rh-nav-item rh-focusable"
        >
          <Settings className="h-4 w-4 shrink-0" />
          <span className="min-w-0 flex-1 truncate">{t.shell.nav.settings}</span>
        </button>
      </nav>

      <div className="rh-profile-block">
        <div className="grid gap-3">
          <div className="flex items-center justify-between gap-3 text-xs">
            <span className="inline-flex items-center gap-2 font-semibold text-white/48">
              <Database className="h-3.5 w-3.5" />
              {t.shell.stats.sources}
            </span>
            <span className="font-black text-white/84">{repositoriesCount}</span>
          </div>
          <div className="flex items-center justify-between gap-3 text-xs">
            <span className="inline-flex items-center gap-2 font-semibold text-white/48">
              <Download className="h-3.5 w-3.5" />
              {t.shell.stats.activeDownloads}
            </span>
            <span className="font-black text-white/84">{activeDownloadsCount}</span>
          </div>
        </div>
      </div>
    </aside>
  );
}
