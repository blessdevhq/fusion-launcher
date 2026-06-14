import { Clipboard, FolderOpen, HeartPulse, Loader2 } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import type { HealthReport, PlatformSetupProfile } from '@/types/repository';
import { HealthGroup, StatusChip } from './shared';
import type { BusyAction } from './types';

export function DiagnosticsSection({
  profiles,
  health,
  busyAction,
  onRunHealth,
  onCopyDiagnostics,
  onOpenLogs
}: {
  profiles: PlatformSetupProfile[];
  health: HealthReport | null;
  busyAction: BusyAction;
  onRunHealth: () => Promise<void>;
  onCopyDiagnostics: () => Promise<void>;
  onOpenLogs: () => Promise<void>;
}) {
  const { t } = useI18n();

  return (
    <section className="grid gap-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="text-sm font-semibold text-fusion-accent">{t.settings.diagnostics.title}</div>
          <p className="mt-2 max-w-2xl text-sm leading-6 text-white/[0.52]">
            {t.settings.diagnostics.copy}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button type="button" onClick={onRunHealth} disabled={busyAction === 'health'} className="rh-mini-action">
            {busyAction === 'health' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <HeartPulse className="h-3.5 w-3.5" />}
            {t.settings.diagnostics.run}
          </button>
          <button type="button" onClick={onCopyDiagnostics} disabled={busyAction === 'diagnostics'} className="rh-mini-action">
            {busyAction === 'diagnostics' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Clipboard className="h-3.5 w-3.5" />}
            {t.settings.diagnostics.copyReport}
          </button>
          <button type="button" onClick={onOpenLogs} disabled={busyAction === 'logs'} className="rh-mini-action">
            {busyAction === 'logs' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <FolderOpen className="h-3.5 w-3.5" />}
            {t.settings.diagnostics.openLogs}
          </button>
        </div>
      </div>
      <div className="grid gap-3 lg:grid-cols-2">
        {profiles.length === 0 ? (
          <div className="rounded-sm border border-white/10 bg-black/[0.34] p-4 text-sm text-white/[0.42] lg:col-span-2">
            {t.settings.diagnostics.profilesLoading}
          </div>
        ) : profiles.map((profile) => {
          const item = health?.platformSetup.find((entry) => entry.id === `profile:${profile.id}`);
          const ready = item?.status === 'ready';

          return (
            <div key={profile.id} className="rounded-sm border border-white/10 bg-black/[0.34] p-4">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="truncate text-sm font-black text-white/[0.86]">{profile.displayName}</div>
                  <div className="mt-1 truncate text-xs text-white/[0.38]">
                    {profile.emulator.emulatorName} / {profile.gameFiles.expectedExtensions.join(', ')}
                  </div>
                </div>
                <StatusChip tone={ready ? 'valid' : 'missing'} label={ready ? t.settings.statusChip.ready : t.settings.statusChip.missing} />
              </div>
              <div className="mt-3 text-xs leading-5 text-white/[0.44]">{item?.message ?? t.settings.diagnostics.notRun}</div>
            </div>
          );
        })}
      </div>
      {health && (
        <div className="grid gap-3 lg:grid-cols-2">
          <HealthGroup title={t.settings.diagnostics.groups.emulators} items={health.emulators} />
          <HealthGroup title={t.settings.diagnostics.groups.launchProfiles} items={health.platformSetup} />
          <HealthGroup title={t.settings.diagnostics.groups.systemFiles} items={health.systemFiles} />
          <HealthGroup title={t.settings.diagnostics.groups.gameFiles} items={health.gameFiles} />
          <HealthGroup title={t.settings.diagnostics.groups.sources} items={[...health.repositories, health.downloader]} />
        </div>
      )}
    </section>
  );
}
