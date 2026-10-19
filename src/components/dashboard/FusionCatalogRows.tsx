import { useEffect, useState } from 'react';
import { ChevronRight } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import { fusionText } from '@/lib/fusionText';
import { fusionGameKey, type FusionCatalogEntry, type FusionOperationProgress } from '@/types/fusionSource';

export function FusionCatalogRows({ entries, operations, onOpen, onFocus }: {
  entries: FusionCatalogEntry[];
  operations: Record<string, FusionOperationProgress>;
  onOpen: (game: FusionCatalogEntry) => void;
  onFocus: (id: string) => void;
}) {
  const { locale } = useI18n();
  const text = fusionText(locale);
  const [visible, setVisible] = useState(60);
  useEffect(() => setVisible(60), [entries]);
  if (!entries.length) return null;
  return <section className="col-span-full min-w-0 border-b border-fusion-line pb-4" data-testid="fusion-catalog">
    <h3 className="mb-2 text-sm font-semibold text-white/70">{text.catalog} · {entries.length}</h3>
    {entries.slice(0, visible).map((game) => {
      const key = fusionGameKey(game);
      const focusId = `fusion-game:${encodeURIComponent(key)}`;
      const operation = operations[key];
      return <button key={key} type="button" className="rh-focusable flex w-full min-w-0 items-center gap-3 border-t border-fusion-line py-3 text-left" data-testid={`fusion-catalog-${game.gameId}`} data-focus-id={focusId} data-focus-zone="library" onFocus={() => onFocus(focusId)} onClick={() => onOpen(game)}>
        <span className="min-w-0 flex-1"><span className="block truncate text-sm font-semibold">{game.title}</span><span className="mt-1 block truncate text-xs text-white/40">{game.sourceName} · {game.version}</span></span>
        <span className="shrink-0 text-xs text-white/60">{operation?.status === 'running' ? operation.phase : game.installed ? text.installed : text.available}</span><ChevronRight aria-hidden="true" className="h-4 w-4 shrink-0 text-white/40" />
      </button>;
    })}
    {visible < entries.length && <button type="button" className="rh-mini-action rh-focusable mt-3" onClick={() => setVisible((count) => count + 60)}>{text.more}</button>}
  </section>;
}
