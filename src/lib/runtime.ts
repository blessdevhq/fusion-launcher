type TauriWindow = Window & {
  __TAURI_INTERNALS__?: unknown;
};

const LOCAL_HOSTS = new Set(['localhost', '127.0.0.1', '::1']);

export function isTauriRuntime() {
  return typeof window !== 'undefined' && Boolean((window as TauriWindow).__TAURI_INTERNALS__);
}

export function isPreviewRuntime() {
  if (typeof window === 'undefined' || isTauriRuntime()) {
    return false;
  }

  const nodeEnv = typeof process !== 'undefined' ? process.env.NODE_ENV : undefined;
  const localDevServer = LOCAL_HOSTS.has(window.location.hostname) && Boolean(window.location.port);

  return (nodeEnv === 'development' || localDevServer) && LOCAL_HOSTS.has(window.location.hostname);
}

export function requireDesktopBridge(feature: string): never {
  throw new Error(`${feature} requires the Fusion Launcher desktop bridge. Start the app through Tauri.`);
}
