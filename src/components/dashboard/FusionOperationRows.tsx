import { useState } from 'react';
import { useI18n } from '@/components/I18nProvider';
import { api } from '@/lib/api';
import { fusionText } from '@/lib/fusionText';
import { isFusionOperationTerminal } from '@/stores/fusionSourceStore';
import { fusionErrorMessage, fusionGameKey, type FusionCatalogEntry, type FusionOperationProgress } from '@/types/fusionSource';

export function FusionOperationRows({ operations, games, onOpen, onFocus }: {
  operations: Record<string, FusionOperationProgress>;
  games: FusionCatalogEntry[];
  onOpen: (game: FusionCatalogEntry) => void;
  onFocus: (id: string) => void;
}) {
  const { locale } = useI18n();
  const text = fusionText(locale);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [visible, setVisible] = useState(60);
  const events = Object.values(operations);
  if (!events.length) return null;
  const cancel = async (event: FusionOperationProgress) => {
    setBusy(event.operationId); setError(null);
    try { await api.fusionGameCancel(event.sourceId, event.gameId); }
    catch (caught) { setError(fusionErrorMessage(caught)); }
    finally { setBusy(null); }
  };
  return <section className="mb-4 border-b border-fusion-line pb-4" data-testid="fusion-operations-list">
    <h3 className="mb-2 text-sm font-semibold">{text.progress}</h3>
    {events.slice(0, visible).map((event) => {
      const key = fusionGameKey(event);
      const game = games.find((game) => fusionGameKey(game) === key);
      const focusId = `fusion-game:${encodeURIComponent(key)}`;
      return <div key={key} className="flex min-w-0 flex-wrap items-center gap-3 border-t border-fusion-line py-3">
        <button type="button" disabled={!game} className="rh-focusable min-w-0 flex-1 text-left" onClick={() => game && onOpen(game)} data-focus-id={focusId} data-focus-zone="downloads" onFocus={() => onFocus(focusId)}><span className="block truncate text-sm font-semibold">{game?.title ?? key}</span><span className="mt-1 block break-words text-xs text-white/50">{event.phase}{event.layerId ? ` · ${event.layerId}` : ''} · {event.status}</span>{event.error && <span className="mt-1 block break-words text-xs text-[var(--rh-warning)]">{event.error.message}</span>}</button>
        {!isFusionOperationTerminal(event) && <button type="button" onClick={() => void cancel(event)} disabled={busy === event.operationId} className="rh-mini-action rh-focusable">{text.cancel}</button>}
      </div>;
    })}
    {events.length > visible && <button type="button" onClick={() => setVisible((count) => count + 60)} className="rh-mini-action rh-focusable">{text.more}</button>}
    {error && <p role="alert" className="mt-3 break-words text-xs text-[var(--rh-warning)]">{error}</p>}
  </section>;
}
