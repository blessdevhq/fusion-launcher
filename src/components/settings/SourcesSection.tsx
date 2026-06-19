import { Ban, FolderOpen, Loader2, RefreshCcw, ShieldAlert } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import { displayProductText } from '@/lib/brandText';
import { sourceTrustLabel } from '@/lib/sourceTrust';
import type { RepositoryPreview, RepositorySummary } from '@/types/repository';
import { AddSourceCard } from './AddSourceCard';
import type { BusyAction } from './types';
import { formatDateTime, shortHash, trustBadgeClass } from './utils';

export function SourcesSection({
  repositories,
  busyAction,
  onConnectRepositoryFile,
  onRefreshRepository,
  onDisconnect,
  onManifestInstalled
}: {
  repositories: RepositorySummary[];
  busyAction: BusyAction;
  sourceUrl: string;
  sourcePreview: RepositoryPreview | null;
  onSourceUrlChange: (value: string) => void;
  onPreviewRepositoryUrl: () => Promise<void>;
  onConnectRepositoryUrl: () => Promise<void>;
  onConnectRepositoryFile: () => Promise<void>;
  onRefreshRepository: (repositoryId: string) => Promise<void>;
  onDisconnect: (repositoryId: string) => Promise<void>;
  onManifestInstalled: () => Promise<void>;
}) {
  const { t } = useI18n();
  const sourceBusy = busyAction === 'repo-file';

  return (
    <section className="grid gap-4" data-testid="settings-modal-sources-panel">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="text-sm font-semibold text-fusion-accent">{t.settings.sourcesPanel.title}</div>
          <p className="mt-2 max-w-2xl text-sm leading-6 text-white/[0.52]">
            {t.settings.sourcesPanel.copy}
          </p>
        </div>
        <button
          type="button"
          onClick={onConnectRepositoryFile}
          disabled={busyAction !== null}
          className="rh-icon-button"
          title={t.settings.sourcesPanel.importJsonTitle}
        >
          {busyAction === 'repo-file' ? <Loader2 className="h-4 w-4 animate-spin" /> : <FolderOpen className="h-4 w-4" />}
        </button>
      </div>

      <AddSourceCard onAdded={onManifestInstalled} />

      <div className="rounded-sm border border-white/10 bg-black/[0.28] p-4">
        <div className="mb-3 flex items-center justify-between gap-3">
          <div className="text-sm font-semibold text-white/[0.62]">{t.settings.sourcesPanel.connected}</div>
          {sourceBusy && <div className="text-[10px] font-bold uppercase text-white/[0.36]">{t.settings.sourcesPanel.busy}</div>}
        </div>
        <div className="grid gap-3 lg:grid-cols-2">
          {repositories.length === 0 ? (
            <div className="rh-empty-compact lg:col-span-2">{t.settings.sourcesPanel.empty}</div>
          ) : repositories.map((repository) => (
            <RepositorySourceCard
              key={repository.id}
              repository={repository}
              busyAction={busyAction}
              onRefreshRepository={onRefreshRepository}
              onDisconnect={onDisconnect}
            />
          ))}
        </div>
      </div>
    </section>
  );
}

function RepositorySourceCard({
  repository,
  busyAction,
  onRefreshRepository,
  onDisconnect
}: {
  repository: RepositorySummary;
  busyAction: BusyAction;
  onRefreshRepository: (repositoryId: string) => Promise<void>;
  onDisconnect: (repositoryId: string) => Promise<void>;
}) {
  const { locale, t } = useI18n();
  const refreshing = busyAction === `repo-refresh:${repository.id}`;
  const removing = busyAction === `repo:${repository.id}`;

  return (
    <div className="rounded-sm border border-white/10 bg-white/[0.04] p-4" data-testid="source-card">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="truncate text-sm font-bold">{displayProductText(repository.name)}</div>
          <div className="mt-1 truncate select-text text-xs text-white/[0.36]">{repository.url}</div>
          <div className="mt-1 text-[10px] font-semibold text-white/[0.28]">{sourceTrustLabel(repository.trustLevel, locale)}</div>
        </div>
        <TrustBadge trustLevel={repository.trustLevel} />
      </div>
      <div className="mt-3 grid gap-2 text-xs text-white/[0.46] sm:grid-cols-2">
        <SourceFact label={t.common.games} value={String(repository.catalogCount)} />
        <SourceFact label={t.common.systemFiles} value={String(repository.systemFileCount)} />
        <SourceFact label={t.common.version} value={repository.version} />
        {repository.contentHash && <SourceFact label={t.common.hash} value={shortHash(repository.contentHash)} />}
        {repository.maintainer && <SourceFact label={t.common.team} value={displayProductText(repository.maintainer)} />}
        {repository.license && <SourceFact label={t.common.license} value={repository.license} />}
      </div>
      {repository.homepageUrl && (
        <div className="mt-2 truncate select-text text-xs text-white/[0.36]">{repository.homepageUrl}</div>
      )}
      {repository.lastRefreshedAt && (
        <div className="mt-2 text-[10px] uppercase text-white/[0.28]">{t.common.updated} {formatDateTime(repository.lastRefreshedAt, locale)}</div>
      )}
      {repository.hasExecutableAssets && (
        <div className="mt-3 flex items-center gap-2 rounded-sm border border-amber-200/[0.2] bg-amber-300/10 px-3 py-2 text-xs font-semibold text-amber-100">
          <ShieldAlert className="h-3.5 w-3.5" />
          {t.settings.sourcesPanel.executableRequiresTrust}
        </div>
      )}
      <div className="mt-4 flex flex-wrap gap-2">
        <button
          type="button"
          onClick={() => onRefreshRepository(repository.id)}
          disabled={busyAction !== null}
          className="inline-flex h-8 items-center gap-2 rounded-sm border border-white/10 px-3 text-xs font-bold text-white/70"
        >
          {refreshing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCcw className="h-3.5 w-3.5" />}
          {t.common.refresh}
        </button>
        <button
          type="button"
          onClick={() => onDisconnect(repository.id)}
          disabled={busyAction !== null}
          className="inline-flex h-8 items-center gap-2 rounded-sm border border-red-300/[0.2] px-3 text-xs font-bold text-red-100/80"
        >
          {removing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Ban className="h-3.5 w-3.5" />}
          {t.common.remove}
        </button>
      </div>
    </div>
  );
}

function SourceFact({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <div className="text-[10px] font-semibold text-white/[0.30]">{label}</div>
      <div className="mt-0.5 truncate select-text text-white/70">{value}</div>
    </div>
  );
}

function TrustBadge({ trustLevel }: { trustLevel: string }) {
  const { locale } = useI18n();

  return (
    <span className={`shrink-0 rounded-lg border px-2 py-1 text-[10px] font-semibold ${trustBadgeClass(trustLevel)}`}>
      {sourceTrustLabel(trustLevel, locale)}
    </span>
  );
}
