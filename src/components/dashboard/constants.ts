import type { LibraryFilter, LibrarySort } from '@/lib/libraryStatus';
import type { TorrentDownloadStatus } from '@/types/repository';

export const FILTERS: LibraryFilter[] = ['all', 'installed', 'downloading', 'missing'];
export const SORTS: LibrarySort[] = ['title', 'status', 'platform', 'repository'];

export const ACTIVE_DOWNLOAD_STATUSES: TorrentDownloadStatus[] = ['resolving', 'downloading', 'cancelling'];
export const RESUMABLE_DOWNLOAD_STATUSES: TorrentDownloadStatus[] = ['paused', 'interrupted', 'error'];
