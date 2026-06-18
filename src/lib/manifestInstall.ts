import type { InstallResult } from '@/types/emulatorProfile';
import type { ManifestGame } from '@/types/manifest';

export type ManifestInstallOutcome = 'ready' | 'attention';

/**
 * Pull a human-readable message out of any thrown/rejected value. Handles both
 * `Error` instances and Tauri's structured `{ kind, message }` rejections (the
 * shape `fetch_manifest` rejects with).
 */
export function extractErrorMessage(error: unknown): string {
  if (error && typeof error === 'object' && 'message' in error) {
    return String((error as { message: unknown }).message);
  }
  if (error instanceof Error) return error.message;
  return String(error);
}

/** True when the manifest entry ships an emulator download (`core_bundle_url`). */
export function manifestGameHasEmulator(game: ManifestGame): boolean {
  return Boolean(game.assets.core_bundle_url && game.assets.core_bundle_url.trim());
}

/** Map an install result to the two states the manifest card renders. */
export function manifestInstallOutcome(result: InstallResult): ManifestInstallOutcome {
  return result.status === 'ready' ? 'ready' : 'attention';
}
