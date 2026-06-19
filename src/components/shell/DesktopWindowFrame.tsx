'use client';

import { getCurrentWindow, type Window as TauriWindow } from '@tauri-apps/api/window';
import { Minus, Square, X } from 'lucide-react';
import { useCallback, useEffect, useState, type MouseEvent, type ReactNode } from 'react';
import { isTauriRuntime } from '@/lib/runtime';

export function DesktopWindowFrame({ children }: { children: ReactNode }) {
  const [appWindow, setAppWindow] = useState<TauriWindow | null>(null);

  useEffect(() => {
    if (isTauriRuntime()) {
      setAppWindow(getCurrentWindow());
    }
  }, []);

  const runWindowAction = useCallback((action: (window: TauriWindow) => Promise<void>) => {
    if (!appWindow) return;
    void action(appWindow).catch((error) => {
      console.error('Fusion Launcher window action failed', error);
    });
  }, [appWindow]);

  const handleTitlebarMouseDown = useCallback((event: MouseEvent<HTMLElement>) => {
    if (!appWindow || event.button !== 0) return;

    if (event.detail === 2) {
      event.preventDefault();
      runWindowAction((window) => window.toggleMaximize());
      return;
    }

    void appWindow.startDragging().catch((error) => {
      console.error('Fusion Launcher window drag failed', error);
    });
  }, [appWindow, runWindowAction]);

  if (!appWindow) {
    return <>{children}</>;
  }

  return (
    <div className="rh-desktop-frame">
      <header
        className="rh-desktop-titlebar"
        data-testid="desktop-titlebar"
        onMouseDown={handleTitlebarMouseDown}
      >
        <div className="rh-desktop-titlebar-brand">
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src="/fusion/app-icon.png" alt="" className="rh-desktop-titlebar-icon" draggable={false} />
          <span>Fusion Launcher</span>
        </div>
        <div className="rh-desktop-window-controls" onMouseDown={(event) => event.stopPropagation()}>
          <button
            type="button"
            className="rh-desktop-window-button"
            data-testid="desktop-window-minimize"
            aria-label="Minimize Fusion Launcher"
            title="Minimize"
            onClick={() => runWindowAction((window) => window.minimize())}
          >
            <Minus className="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            className="rh-desktop-window-button"
            data-testid="desktop-window-maximize"
            aria-label="Maximize Fusion Launcher"
            title="Maximize"
            onClick={() => runWindowAction((window) => window.toggleMaximize())}
          >
            <Square className="h-3 w-3" />
          </button>
          <button
            type="button"
            className="rh-desktop-window-button rh-desktop-window-close"
            data-testid="desktop-window-close"
            aria-label="Close Fusion Launcher"
            title="Close"
            onClick={() => runWindowAction((window) => window.close())}
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      </header>
      <div className="rh-desktop-content">
        {children}
      </div>
    </div>
  );
}
