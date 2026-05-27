'use client';

import { ControllerHintBar } from './ControllerHintBar';
import { Sidebar } from './Sidebar';
import type { LauncherView } from '../../stores/launcherStore.ts';
import type { ReactNode } from 'react';

interface AppShellProps {
  activeView: LauncherView;
  repositoriesCount: number;
  activeDownloadsCount: number;
  onNavigate: (view: LauncherView) => void;
  onFocus: (focusId: string) => void;
  children: ReactNode;
}

export function AppShell({
  activeView,
  repositoriesCount,
  activeDownloadsCount,
  onNavigate,
  onFocus,
  children
}: AppShellProps) {
  return (
    <main className="rh-app">
      <div className="rh-app-grid">
        <Sidebar
          activeView={activeView}
          repositoriesCount={repositoriesCount}
          activeDownloadsCount={activeDownloadsCount}
          onNavigate={onNavigate}
          onFocus={onFocus}
        />
        <section className="rh-main-surface">
          {children}
          <ControllerHintBar activeView={activeView} />
        </section>
      </div>
    </main>
  );
}
