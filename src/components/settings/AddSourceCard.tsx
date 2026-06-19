import { useState } from 'react';
import { AlertTriangle, Loader2, PackagePlus, ShieldAlert } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import { api } from '@/lib/api';
import { displayProductText } from '@/lib/brandText';
import { sourceTrustLabel } from '@/lib/sourceTrust';
import type { RepositoryPreview } from '@/types/repository';
import { shortHash, trustBadgeClass } from './utils';

// Unified "Add a source" card. Accepts a community/personal repository URL, a
// repository JSON, or a game manifest (URL or pasted JSON); the backend
// (`preview_source` / `add_source`) auto-detects the format and registers it as a
// browsable library source without installing anything.
export function AddSourceCard({ onAdded }: { onAdded: () => Promise<void> }) {
  const { t } = useI18n();
  const strings = t.settings.sourcesPanel;
  const [input, setInput] = useState('');
  const [checking, setChecking] = useState(false);
  const [adding, setAdding] = useState(false);
  const [preview, setPreview] = useState<RepositoryPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const trimmed = input.trim();
  const busy = checking || adding;

  const check = async () => {
    if (!trimmed) return;
    setChecking(true);
    setError(null);
    setNotice(null);
    setPreview(null);
    try {
      setPreview(await api.previewSource(trimmed));
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setChecking(false);
    }
  };

  const add = async () => {
    if (!trimmed) return;
    setAdding(true);
    setError(null);
    setNotice(null);
    try {
      await api.addSource(trimmed);
      await onAdded();
      setNotice(strings.added);
      setInput('');
      setPreview(null);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setAdding(false);
    }
  };

  return (
    <div className="rounded-sm border border-white/10 bg-black/[0.34] p-4" data-testid="add-source">
      <div className="flex items-center gap-2 text-sm font-semibold text-fusion-accent">
        <PackagePlus className="h-4 w-4" />
        {strings.addTitle}
      </div>
      <p className="mt-2 max-w-2xl text-sm leading-6 text-white/[0.52]">{strings.addCopy}</p>

      <div className="mt-3 grid gap-2 lg:grid-cols-[minmax(0,1fr)_auto_auto]">
        <textarea
          value={input}
          onChange={(event) => setInput(event.target.value)}
          onKeyDown={(event) => {
            if ((event.ctrlKey || event.metaKey) && event.key === 'Enter' && !busy && trimmed) void check();
          }}
          className="min-h-20 min-w-0 resize-y rounded-sm border border-white/10 bg-black/40 px-3 py-2 text-sm text-white/80 outline-none transition placeholder:text-white/25 focus:border-white/60"
          placeholder={strings.manifest.placeholder}
          data-testid="add-source-input"
        />
        <button
          type="button"
          onClick={check}
          disabled={busy || !trimmed}
          className="rh-mini-action h-11 justify-center"
          data-testid="add-source-check"
        >
          {checking ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldAlert className="h-3.5 w-3.5" />}
          {t.common.check}
        </button>
        <button
          type="button"
          onClick={add}
          disabled={busy || !trimmed}
          className="rh-primary-action h-11 justify-center"
          data-testid="add-source-add"
        >
          {adding ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <PackagePlus className="h-3.5 w-3.5" />}
          {adding ? strings.adding : strings.addButton}
        </button>
      </div>

      {error && (
        <div className="mt-3 flex items-center gap-2 rounded-sm border border-red-300/[0.2] bg-red-300/10 px-3 py-2 text-xs font-semibold text-red-100/90">
          <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 break-words select-text whitespace-pre-line">{error}</span>
        </div>
      )}

      {notice && (
        <div className="mt-3 flex items-center gap-2 rounded-sm border border-emerald-200/[0.22] bg-emerald-300/10 px-3 py-2 text-xs font-semibold text-emerald-100/90">
          <PackagePlus className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 break-words">{notice}</span>
        </div>
      )}

      {preview && <SourcePreviewCard preview={preview} />}
    </div>
  );
}

function SourcePreviewCard({ preview }: { preview: RepositoryPreview }) {
  const { locale, t } = useI18n();

  return (
    <div className="mt-4 rounded-sm border border-white/[0.16] bg-white/[0.055] p-4" data-testid="add-source-preview">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="truncate text-sm font-black text-white">{displayProductText(preview.name)}</div>
          <div className="mt-1 truncate select-text text-xs text-white/50">{preview.url}</div>
          <div className="mt-1 text-[10px] font-semibold text-white/[0.36]">{sourceTrustLabel(preview.trustLevel, locale)}</div>
        </div>
        <span className={`shrink-0 rounded-lg border px-2 py-1 text-[10px] font-semibold ${trustBadgeClass(preview.trustLevel)}`}>
          {sourceTrustLabel(preview.trustLevel, locale)}
        </span>
      </div>
      <div className="mt-3 grid gap-2 text-xs text-white/56 sm:grid-cols-2">
        <Fact label={t.common.games} value={String(preview.catalogCount)} />
        <Fact label={t.common.systemFiles} value={String(preview.systemFileCount)} />
        <Fact label={t.common.version} value={preview.version} />
        <Fact label={t.common.hash} value={shortHash(preview.contentHash)} />
        {preview.maintainer && <Fact label={t.common.team} value={displayProductText(preview.maintainer)} />}
        {preview.license && <Fact label={t.common.license} value={preview.license} />}
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

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <div className="text-[10px] font-semibold text-white/[0.30]">{label}</div>
      <div className="mt-0.5 truncate select-text text-white/70">{value}</div>
    </div>
  );
}
