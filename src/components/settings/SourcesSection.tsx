import { Ban, Clipboard, FolderOpen, Link2, Loader2, RefreshCcw, ShieldAlert } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import { displayProductText } from '@/lib/brandText';
import { sourceTrustLabel } from '@/lib/sourceTrust';
import type { RepositoryPreview, RepositorySummary } from '@/types/repository';
import { PUBLIC_SOURCE_TEMPLATE_URL } from './constants';
import type { BusyAction } from './types';
import { formatDateTime, shortHash, trustBadgeClass } from './utils';

export function SourcesSection({
  repositories,
  busyAction,
  sourceUrl,
  sourcePreview,
  onSourceUrlChange,
  onPreviewRepositoryUrl,
  onConnectRepositoryUrl,
  onConnectRepositoryFile,
  onRefreshRepository,
  onDisconnect
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
}) {
  const { t } = useI18n();
  const sourceBusy = busyAction === 'repo-preview-url' || busyAction === 'repo-connect-url' || busyAction === 'repo-file';

  return (
    <section className="grid gap-4" data-testid="settings-modal-sources-panel">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="text-sm font-semibold text-fusion-accent">{t.settings.sourcesPanel.title}</div>
          <p className="mt-2 max-w-2xl text-sm leading-6 text-white/[0.52]">
            {t.settings.sourcesPanel.copy}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            onClick={() => onSourceUrlChange(PUBLIC_SOURCE_TEMPLATE_URL)}
            disabled={busyAction !== null}
            className="rh-mini-action"
            title={t.settings.sourcesPanel.templateTitle}
          >
            <Clipboard className="h-3.5 w-3.5" />
            {t.settings.sourcesPanel.template}
          </button>
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
      </div>

      <div className="rounded-sm border border-white/10 bg-black/[0.34] p-4">
        <div className="grid gap-2 lg:grid-cols-[minmax(0,1fr)_auto_auto_auto]">
          <input
            value={sourceUrl}
            onChange={(event) => onSourceUrlChange(event.target.value)}
            className="h-11 min-w-0 rounded-sm border border-white/10 bg-black/40 px-3 text-sm text-white/80 outline-none transition placeholder:text-white/25 focus:border-white/60"
            placeholder="https://example.com/fusion-launcher-repository.json"
            data-testid="settings-source-url"
          />
          <button
            type="button"
            onClick={onPreviewRepositoryUrl}
            disabled={busyAction !== null || !sourceUrl.trim()}
            className="rh-mini-action h-11 justify-center"
          >
            {busyAction === 'repo-preview-url' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldAlert className="h-3.5 w-3.5" />}
            {t.common.check}
          </button>
          <button
            type="button"
            onClick={onConnectRepositoryUrl}
            disabled={busyAction !== null || !sourceUrl.trim()}
            className="rh-mini-action h-11 justify-center"
          >
            {busyAction === 'repo-connect-url' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Link2 className="h-3.5 w-3.5" />}
            {t.common.connect}
          </button>
          <button
            type="button"
            onClick={onConnectRepositoryFile}
            disabled={busyAction !== null}
            className="rh-mini-action h-11 justify-center"
          >
            {busyAction === 'repo-file' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <FolderOpen className="h-3.5 w-3.5" />}
            JSON
          </button>
        </div>

        {sourcePreview && <SourcePreviewCard preview={sourcePreview} />}
      </div>

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

function SourcePreviewCard({ preview }: { preview: RepositoryPreview }) {
  const { locale, t } = useI18n();

  return (
    <div className="mt-4 rounded-sm border border-white/[0.16] bg-white/[0.055] p-4" data-testid="source-preview">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="truncate text-sm font-black text-white">{displayProductText(preview.name)}</div>
          <div className="mt-1 truncate text-xs text-white/50">{preview.url}</div>
          <div className="mt-1 text-[10px] font-semibold text-white/[0.36]">{sourceTrustLabel(preview.trustLevel, locale)}</div>
        </div>
        <TrustBadge trustLevel={preview.trustLevel} />
      </div>
      <div className="mt-3 grid gap-2 text-xs text-white/56 sm:grid-cols-2">
        <SourceFact label={t.common.games} value={String(preview.catalogCount)} />
        <SourceFact label={t.common.systemFiles} value={String(preview.systemFileCount)} />
        <SourceFact label={t.common.version} value={preview.version} />
        <SourceFact label={t.common.hash} value={shortHash(preview.contentHash)} />
        {preview.maintainer && <SourceFact label={t.common.team} value={displayProductText(preview.maintainer)} />}
        {preview.license && <SourceFact label={t.common.license} value={preview.license} />}
      </div>
      {preview.hasExecutableAssets && (
        <div className="mt-3 flex items-center gap-2 rounded-sm border border-amber-200/[0.2] bg-amber-300/10 px-3 py-2 text-xs font-semibold text-amber-100">
          <ShieldAlert className="h-3.5 w-3.5" />
          {t.settings.sourcesPanel.executableAssets}
        </div>
      )}
      {preview.trustLevel === 'unknown' && (
        <div className="mt-3 flex items-center gap-2 rounded-sm border border-amber-200/[0.2] bg-amber-300/10 px-3 py-2 text-xs font-semibold text-amber-100">
          <ShieldAlert className="h-3.5 w-3.5" />
          {t.settings.sourcesPanel.unknownSource}
        </div>
      )}
    </div>
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
          <div className="mt-1 truncate text-xs text-white/[0.36]">{repository.url}</div>
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
        <div className="mt-2 truncate text-xs text-white/[0.36]">{repository.homepageUrl}</div>
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
      <div className="mt-0.5 truncate text-white/70">{value}</div>
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
