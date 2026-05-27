import { appLocalDataDir, join } from '@tauri-apps/api/path';
import { isPreviewRuntime, isTauriRuntime, requireDesktopBridge } from './runtime.ts';

export async function defaultSaveDirForGame(gameId: string) {
  const safeGameId = safePathSegment(gameId);

  if (isTauriRuntime()) {
    const appDataDir = await appLocalDataDir();
    return join(appDataDir, 'games', safeGameId);
  }

  if (isPreviewRuntime()) {
    return `preview://games/${safeGameId}`;
  }

  return requireDesktopBridge('Resolving download folders');
}

export function safePathSegment(value: string) {
  const sanitized = value
    .trim()
    .replace(/[<>:"/\\|?*\u0000-\u001F]/g, '-')
    .replace(/[.\s-]+$/g, '')
    .slice(0, 120);

  return sanitized || 'game';
}
