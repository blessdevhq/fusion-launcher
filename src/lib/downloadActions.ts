import type { CatalogGame, TorrentDownloadRecord } from '../types/repository.ts';

export function isDirectGameDownload(
  game: CatalogGame | null | undefined,
  record: Pick<TorrentDownloadRecord, 'magnetUri'> | null | undefined
) {
  if (record?.magnetUri.startsWith('direct:')) {
    return true;
  }

  const primarySource = game?.downloads[0];
  return primarySource?.kind === 'http' || primarySource?.kind === 'bundled';
}

export function shouldShowDownloadPeers(
  record: Pick<TorrentDownloadRecord, 'magnetUri'> | null | undefined
) {
  return !record?.magnetUri.startsWith('direct:');
}
