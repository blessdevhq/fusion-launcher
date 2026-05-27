'use client';

import { AlertTriangle, Download, Settings, ShieldAlert, X } from 'lucide-react';
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
  const view = launchFailureView(failure);

  const runAction = () => {
    if (view.actionKind === 'settings') onOpenSettings();
    else if (view.actionKind === 'details') onOpenDetails();
    else if (view.actionKind === 'retry-download') onRetryDownload();
    else onClose();
  };

  return (
    <div className="fixed inset-0 z-[60] grid place-items-center bg-black/72 px-5">
      <section className="w-full max-w-md rounded-lg border border-white/12 bg-[#141417] p-5 shadow-2xl">
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
            title="Close"
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        <div className="mt-5 flex justify-end gap-2 border-t border-white/10 pt-4">
          <button
            onClick={onClose}
            className="h-10 rounded-md border border-white/10 px-4 text-sm font-semibold text-white/66 transition hover:bg-white/10"
          >
            Close
          </button>
          <button
            onClick={runAction}
            className="h-10 rounded-md bg-hydra-accent px-4 text-sm font-bold text-white shadow-glow transition hover:bg-violet-500"
          >
            {view.actionLabel}
          </button>
        </div>
      </section>
    </div>
  );
}
