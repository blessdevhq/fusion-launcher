'use client';

import { getCurrentWindow, type Window as TauriWindow } from '@tauri-apps/api/window';
import { MotionConfig } from 'framer-motion';
import { Copy, Minus, Square, X } from 'lucide-react';
import { useCallback, useEffect, useState, type MouseEvent, type ReactNode } from 'react';
import { isTauriRuntime } from '@/lib/runtime';

export function DesktopWindowFrame({ children }: { children: ReactNode }) {
  const [appWindow, setAppWindow] = useState<TauriWindow | null>(null);
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    if (!isTauriRuntime()) return undefined;
    const win = getCurrentWindow();
    setAppWindow(win);

    let active = true;
    const syncMaximized = () => {
      void win.isMaximized().then((maximized) => {
        if (active) setIsMaximized(maximized);
      }).catch(() => {});
    };

    syncMaximized();
    const unlistenPromise = win.onResized(syncMaximized);
    return () => {
      active = false;
      void unlistenPromise.then((unlisten) => unlisten()).catch(() => {});
    };
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
    return <MotionConfig reducedMotion="user">{children}</MotionConfig>;
  }

  return (
    <MotionConfig reducedMotion="user">
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
              aria-label={isMaximized ? 'Restore Fusion Launcher' : 'Maximize Fusion Launcher'}
              title={isMaximized ? 'Restore' : 'Maximize'}
              onClick={() => runWindowAction((window) => window.toggleMaximize())}
            >
              {isMaximized ? <Copy className="h-3 w-3" /> : <Square className="h-3 w-3" />}
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
    </MotionConfig>
  );
}
