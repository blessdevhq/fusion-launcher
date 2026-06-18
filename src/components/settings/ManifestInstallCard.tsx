import { useState } from 'react';
import { AlertTriangle, CheckCircle2, Cpu, Download, Loader2, PackagePlus } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import { api } from '@/lib/api';
import { displayProductText } from '@/lib/brandText';
import {
  extractErrorMessage,
  manifestGameHasEmulator,
  manifestInstallOutcome,
  type ManifestInstallOutcome
} from '@/lib/manifestInstall';
import type { Manifest, ManifestGame } from '@/types/manifest';

export function ManifestInstallCard({ onInstalled }: { onInstalled: () => Promise<void> }) {
  const { t } = useI18n();
  const strings = t.settings.sourcesPanel.manifest;
  const [manifestInput, setManifestInput] = useState('');
  const [loading, setLoading] = useState(false);
  const [manifest, setManifest] = useState<Manifest | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, ManifestInstallOutcome>>({});

  const trimmedInput = manifestInput.trim();
  const busy = loading || installingId !== null;

  const loadManifest = async () => {
    if (!trimmedInput) return;
    setLoading(true);
    setError(null);
    setNotice(null);
    setManifest(null);
    setResults({});
    try {
      setManifest(await api.fetchManifest(trimmedInput));
    } catch (caught) {
      setError(extractErrorMessage(caught));
    } finally {
      setLoading(false);
    }
  };

  const installGame = async (game: ManifestGame) => {
    setInstallingId(game.title_id);
    setError(null);
    setNotice(null);
    try {
      const result = await api.installGameFromManifest(trimmedInput, game.title_id);
      const outcome = manifestInstallOutcome(result);
      setResults((current) => ({ ...current, [game.title_id]: outcome }));
      if (outcome !== 'ready') {
        setNotice(result.message ?? result.errorCode ?? strings.needsAttention);
      }
      await onInstalled();
    } catch (caught) {
      setError(extractErrorMessage(caught));
    } finally {
      setInstallingId(null);
    }
  };

  return (
    <div className="rounded-sm border border-white/10 bg-black/[0.34] p-4" data-testid="manifest-install">
      <div className="flex items-center gap-2 text-sm font-semibold text-fusion-accent">
        <PackagePlus className="h-4 w-4" />
        {strings.title}
      </div>
      <p className="mt-2 max-w-2xl text-sm leading-6 text-white/[0.52]">{strings.copy}</p>

      <div className="mt-3 grid gap-2 lg:grid-cols-[minmax(0,1fr)_auto]">
        <textarea
          value={manifestInput}
          onChange={(event) => setManifestInput(event.target.value)}
          onKeyDown={(event) => {
            if ((event.ctrlKey || event.metaKey) && event.key === 'Enter' && !busy && trimmedInput) void loadManifest();
          }}
          className="min-h-24 min-w-0 resize-y rounded-sm border border-white/10 bg-black/40 px-3 py-2 text-sm text-white/80 outline-none transition placeholder:text-white/25 focus:border-white/60"
          placeholder={strings.placeholder}
          data-testid="manifest-url"
        />
        <button
          type="button"
          onClick={loadManifest}
          disabled={busy || !trimmedInput}
          className="rh-mini-action h-11 justify-center"
          data-testid="manifest-load"
        >
          {loading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Download className="h-3.5 w-3.5" />}
          {strings.load}
        </button>
      </div>

      {error && (
        <div className="mt-3 flex items-center gap-2 rounded-sm border border-red-300/[0.2] bg-red-300/10 px-3 py-2 text-xs font-semibold text-red-100/90">
          <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 break-words">{error}</span>
        </div>
      )}

      {notice && (
        <div className="mt-3 flex items-center gap-2 rounded-sm border border-amber-200/[0.22] bg-amber-300/10 px-3 py-2 text-xs font-semibold text-amber-100/90">
          <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 break-words">{notice}</span>
        </div>
      )}

      {manifest && (
        <div className="mt-4">
          <div className="mb-2 flex items-center justify-between gap-3">
            <div className="truncate text-sm font-bold text-white">{displayProductText(manifest.repository_name)}</div>
            <div className="text-[10px] font-semibold uppercase text-white/[0.36]">{strings.gamesTitle}</div>
          </div>
          {manifest.games.length === 0 ? (
            <div className="rh-empty-compact">{strings.empty}</div>
          ) : (
            <div className="grid gap-2">
              {manifest.games.map((game) => (
                <ManifestGameRow
                  key={game.title_id}
                  game={game}
                  installing={installingId === game.title_id}
                  disabled={busy}
                  result={results[game.title_id]}
                  onInstall={() => installGame(game)}
                />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function ManifestGameRow({
  game,
  installing,
  disabled,
  result,
  onInstall
}: {
  game: ManifestGame;
  installing: boolean;
  disabled: boolean;
  result?: ManifestInstallOutcome;
  onInstall: () => void;
}) {
  const { t } = useI18n();
  const strings = t.settings.sourcesPanel.manifest;
  const hasEmulator = manifestGameHasEmulator(game);

  return (
    <div className="flex flex-wrap items-center justify-between gap-3 rounded-sm border border-white/10 bg-white/[0.04] px-3 py-2.5" data-testid="manifest-game">
      <div className="min-w-0">
        <div className="truncate text-sm font-bold text-white">{displayProductText(game.title)}</div>
        <div className="mt-1 flex flex-wrap items-center gap-2 text-[10px] font-semibold uppercase text-white/[0.40]">
          <span className="rounded-lg border border-white/10 px-2 py-0.5">{game.platform}</span>
          {hasEmulator && (
            <span className="inline-flex items-center gap-1 rounded-lg border border-fusion-accent/30 px-2 py-0.5 text-fusion-accent">
              <Cpu className="h-3 w-3" />
              {strings.emulatorIncluded}
            </span>
          )}
        </div>
      </div>
      <div className="flex items-center gap-2">
        {result === 'ready' && (
          <span className="inline-flex items-center gap-1 text-xs font-semibold text-emerald-200/90">
            <CheckCircle2 className="h-3.5 w-3.5" />
            {strings.installed}
          </span>
        )}
        {result === 'attention' && (
          <span className="inline-flex items-center gap-1 text-xs font-semibold text-amber-100/90">
            <AlertTriangle className="h-3.5 w-3.5" />
            {strings.needsAttention}
          </span>
        )}
        <button
          type="button"
          onClick={onInstall}
          disabled={disabled}
          className="rh-mini-action h-9 justify-center"
          data-testid="manifest-install-game"
        >
          {installing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Download className="h-3.5 w-3.5" />}
          {installing ? strings.installing : strings.install}
        </button>
      </div>
    </div>
  );
}
