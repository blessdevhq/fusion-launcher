'use client';

import {
  Compass,
  Download,
  FolderHeart,
  Home,
  Library,
  Settings,
  Wifi,
  type LucideIcon
} from 'lucide-react';
import type { LauncherView } from '../../stores/launcherStore.ts';

const NAV_ITEMS: Array<{ id: LauncherView; label: string; icon: LucideIcon }> = [
  { id: 'home', label: 'Home', icon: Home },
  { id: 'library', label: 'Library', icon: Library },
  { id: 'explore', label: 'Explore', icon: Compass },
  { id: 'downloads', label: 'Downloads', icon: Download },
  { id: 'collections', label: 'Collections', icon: FolderHeart },
  { id: 'settings', label: 'Settings', icon: Settings }
];

interface SidebarProps {
  activeView: LauncherView;
  repositoriesCount: number;
  activeDownloadsCount: number;
  onNavigate: (view: LauncherView) => void;
  onFocus: (focusId: string) => void;
}

export function Sidebar({
  activeView,
  repositoriesCount,
  activeDownloadsCount,
  onNavigate,
  onFocus
}: SidebarProps) {
  return (
    <aside className="rh-sidebar">
      <div>
        <div className="text-xs uppercase tracking-[0.14em] text-white/42">P2P Retro Launcher</div>
        <div className="mt-6 flex items-center gap-3">
          <div className="grid h-10 w-10 place-items-center rounded-md bg-hydra-accent text-xs font-black text-white shadow-glow">RH</div>
          <div className="min-w-0">
            <div className="truncate text-sm font-black">RetroHydra</div>
            <div className="text-xs text-white/46">{repositoriesCount} repositories</div>
          </div>
        </div>
      </div>

      <nav className="mt-10 space-y-2">
        {NAV_ITEMS.map((item) => {
          const Icon = item.icon;
          const count = item.id === 'downloads' ? activeDownloadsCount : 0;
          const focusId = `nav:${item.id}`;

          return (
            <button
              key={item.id}
              data-focus-id={focusId}
              data-focus-zone="sidebar"
              onFocus={() => onFocus(focusId)}
              onClick={() => onNavigate(item.id)}
              className={`rh-nav-item rh-focusable ${activeView === item.id ? 'rh-nav-item-active' : ''}`}
            >
              <Icon className="h-4 w-4 shrink-0" />
              <span className="min-w-0 flex-1 truncate">{item.label}</span>
              {count > 0 && <span className="rounded bg-white/10 px-1.5 py-0.5 text-[10px] text-white/76">{count}</span>}
            </button>
          );
        })}
      </nav>

      <div className="mt-auto border-t border-white/10 pt-5">
        <div className="flex items-center gap-3">
          <div className="grid h-12 w-12 place-items-center rounded-full bg-[radial-gradient(circle_at_35%_25%,#e9dac1,#4a463f_56%,#171717)] text-xs font-black">
            P1
          </div>
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-bold">PlayerOne</div>
            <div className="text-xs text-[#a78bfa]">Level 24</div>
            <div className="mt-2 h-1 overflow-hidden rounded bg-white/12">
              <div className="h-full w-[68%] rounded bg-hydra-accent" />
            </div>
            <div className="mt-1 text-right text-[10px] text-white/38">12,450 XP</div>
          </div>
        </div>
        <div className="mt-5 flex items-center gap-2 text-[11px] font-black uppercase tracking-wide text-hydra-green">
          <Wifi className="h-3 w-3" />
          Online
        </div>
      </div>
    </aside>
  );
}
