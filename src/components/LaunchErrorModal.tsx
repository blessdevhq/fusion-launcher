'use client';

import { AlertTriangle, Download, Settings, ShieldAlert, X } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import { launchFailureView } from '@/lib/launchErrors';
import type { LaunchFailure } from '@/types/repository';

interface LaunchErrorModalProps {
  failure: LaunchFailure;
  onClose: () => void;
  onOpenSettings: () => void;
  onOpenDetails: () => void;
  onRetryDownload: () => void;
}

export function LaunchErrorModal({
  failure,
  onClose,
  onOpenSettings,
  onOpenDetails,
  onRetryDownload
}: LaunchErrorModalProps) {
  const { locale, t } = useI18n();
  const view = launchFailureView(failure, locale);

  const runAction = () => {
    if (view.actionKind === 'settings') onOpenSettings();
    else if (view.actionKind === 'details') onOpenDetails();
    else if (view.actionKind === 'retry-download') onRetryDownload();
    else onClose();
  };

  return (
    <div className="fixed inset-0 z-[60] grid place-items-center bg-black/72 px-5">
      <section className="w-full max-w-md rounded-2xl border border-white/12 bg-fusion-surface/95 p-5 shadow-[0_40px_120px_rgba(0,0,0,0.65)]">
        <header className="flex items-start gap-4">
          <div className="grid h-10 w-10 shrink-0 place-items-center rounded-md bg-amber-300/12 text-amber-100">
            {view.actionKind === 'settings' ? (
              <Settings className="h-5 w-5" />
            ) : view.actionKind === 'retry-download' ? (
              <Download className="h-5 w-5" />
            ) : view.actionKind === 'details' ? (
              <ShieldAlert className="h-5 w-5" />
            ) : (
              <AlertTriangle className="h-5 w-5" />
            )}
          </div>
          <div className="min-w-0 flex-1">
            <h2 className="text-xl font-black">{view.title}</h2>
            <p className="mt-2 text-sm leading-6 text-white/62">{view.message}</p>
          </div>
          <button
            onClick={onClose}
            className="grid h-9 w-9 place-items-center rounded-md border border-white/10 text-white/60 transition hover:text-white"
            title={t.common.close}
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        <div className="mt-5 flex justify-end gap-2 border-t border-white/10 pt-4">
          <button
            onClick={onClose}
            className="h-10 rounded-md border border-white/10 px-4 text-sm font-semibold text-white/66 transition hover:bg-white/10"
          >
            {t.common.close}
          </button>
          <button
            onClick={runAction}
            className="h-10 rounded-lg bg-fusion-accent px-4 text-sm font-bold text-fusion-accentOn shadow-glow transition hover:bg-fusion-accentHover"
          >
            {view.actionLabel}
          </button>
        </div>
      </section>
    </div>
  );
}
