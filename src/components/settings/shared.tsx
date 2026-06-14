import { Download, Loader2, RefreshCcw } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import { updateErrorText } from '@/lib/i18n';
import type { EmulatorDraftTone } from '@/lib/settingsModalState';
import type { HealthCheckItem } from '@/types/repository';
import type { UpdatePanelState } from './types';
import { healthStatusLabel, healthToneClass, statusToneClass } from './utils';

export function UpdateCheckPanel({
  state,
  onCheck,
  onInstall
}: {
  state: UpdatePanelState;
  onCheck: () => Promise<void>;
  onInstall: () => Promise<void>;
}) {
  const { locale, t } = useI18n();
  const checking = state.phase === 'checking';
  const installing = state.phase === 'installing';
  const busy = checking || installing;

  return (
    <div className="rounded-sm border border-white/10 bg-black/[0.34] p-5">
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div>
          <div className="flex items-center gap-2 font-bold">
            <RefreshCcw className="h-4 w-4 text-white/72" />
            {t.settings.updates.panelTitle}
          </div>
          <div className="mt-1 text-sm text-white/[0.46]">{t.settings.updates.panelCopy}</div>
        </div>
        <div className="flex flex-wrap gap-2">
          <button type="button" onClick={onCheck} disabled={busy} className="rh-mini-action">
            {checking ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCcw className="h-3.5 w-3.5" />}
            {state.phase === 'error' ? t.settings.updates.retry : t.settings.updates.check}
          </button>
          {state.phase === 'available' && (
            <button type="button" onClick={onInstall} disabled={busy} className="rh-mini-action">
              {installing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Download className="h-3.5 w-3.5" />}
              {t.settings.updates.installNow}
            </button>
          )}
        </div>
      </div>
      <div className={`rh-update-status rh-update-status-${state.phase}`}>
        {state.phase === 'idle' && t.settings.updates.checkIdle}
        {state.phase === 'checking' && t.settings.updates.checking}
        {state.phase === 'installing' && t.settings.updates.installing}
        {state.phase === 'up-to-date' && t.settings.updates.upToDate(state.report?.currentVersion)}
        {state.phase === 'available' && (
          <div>
            <div className="font-black text-white">{t.settings.updates.available(state.report?.version)}</div>
            {state.report?.body && <div className="mt-1 text-white/50">{state.report.body}</div>}
            {state.report?.date && <div className="mt-1 text-white/[0.36]">{t.common.published} {state.report.date}</div>}
          </div>
        )}
        {state.phase === 'error' && updateErrorText(state.error, locale)}
      </div>
    </div>
  );
}

export function HealthGroup({ title, items }: { title: string; items: HealthCheckItem[] }) {
  const { t } = useI18n();

  return (
    <div className="rounded-sm border border-white/10 bg-white/[0.04] p-4">
      <div className="mb-3 text-sm font-semibold text-white/[0.62]">{title}</div>
      <div className="space-y-2">
        {items.length === 0 ? (
          <div className="text-xs text-white/[0.36]">{t.settings.emptyHealth}</div>
        ) : items.map((item) => (
          <div key={item.id} className="flex items-start gap-3 rounded-sm border border-white/[0.08] bg-black/[0.16] px-3 py-2 text-xs">
            <span className={`mt-1 h-2 w-2 rounded-full ${healthToneClass(item.status)}`} />
            <div className="min-w-0 flex-1">
              <div className="truncate font-bold text-white/[0.82]">{item.label}</div>
              <div className="mt-1 text-white/[0.42]">{item.message ?? healthStatusLabel(item.status, t)}</div>
            </div>
            <span className="rounded-sm border border-white/10 px-2 py-1 uppercase text-white/[0.48]">{healthStatusLabel(item.status, t)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

export function StatusChip({ tone, label }: { tone: EmulatorDraftTone; label: string }) {
  return (
    <span className={`shrink-0 rounded-lg border px-2 py-1 text-[10px] font-semibold ${statusToneClass(tone)}`}>
      {label}
    </span>
  );
}

export function SummaryCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-sm border border-white/10 bg-black/[0.34] p-4">
      <div className="text-[10px] font-semibold text-white/[0.32]">{label}</div>
      <div className="mt-2 text-2xl font-black text-white">{value}</div>
    </div>
  );
}

export function PathCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-sm border border-white/10 bg-white/[0.025] p-4">
      <div className="text-[10px] font-semibold text-white/[0.32]">{label}</div>
      <div className="mt-2 truncate text-sm text-white/[0.62]">{value}</div>
    </div>
  );
}

export function MetricLine({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span>{label}</span>
      <span className="font-black text-white/80">{value}</span>
    </div>
  );
}
