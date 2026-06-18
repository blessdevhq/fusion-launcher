import { Download, FolderOpen, Loader2, Trash2, Wrench } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import { getEmulatorConfig, getEmulatorPath, type AppSettings } from '@/lib/settings';
import { PLATFORM_LABELS, type EmulatorManagerPlatform } from '@/types/platform';
import type { InstallProgressEvent } from '@/types/emulatorProfile';
import type { PlatformSetupProfile } from '@/types/repository';
import { StatusChip } from './shared';
import type { BusyState } from './types';

const DEFAULT_PROFILE_BY_PLATFORM: Record<EmulatorManagerPlatform, string> = {
  nes: 'nes-mesen',
  snes: 'snes-mesen',
  n64: 'n64-rmg',
  gba: 'gba-mgba',
  ps2: 'ps2-pcsx2',
  psp: 'psp-ppsspp',
  ps1: 'ps1-manual',
  switch: 'switch-manual'
};

const PROFILE_ORDER = Object.values(DEFAULT_PROFILE_BY_PLATFORM);

export function EmulatorsSection({
  settings,
  profiles,
  activeProfileId,
  busy,
  progressByProfileId,
  onFocusProfile,
  onInstall,
  onInstallTo,
  onSelect,
  onRemove,
  onOpenFolder
}: {
  settings: AppSettings;
  profiles: PlatformSetupProfile[];
  activeProfileId: string;
  busy: BusyState;
  progressByProfileId: Record<string, InstallProgressEvent | undefined>;
  onFocusProfile: (profileId: string) => void;
  onInstall: (profile: PlatformSetupProfile) => Promise<void>;
  onInstallTo: (profile: PlatformSetupProfile) => Promise<void>;
  onSelect: (profile: PlatformSetupProfile) => Promise<void>;
  onRemove: (profile: PlatformSetupProfile) => Promise<void>;
  onOpenFolder: (profile: PlatformSetupProfile) => Promise<void>;
}) {
  const { t } = useI18n();
  const profileById = new Map(profiles.map((profile) => [profile.id, profile]));
  const managerProfiles = PROFILE_ORDER.flatMap((profileId) => {
    const profile = profileById.get(profileId);
    return profile ? [profile] : [];
  });
  const activeProfile = profileById.get(activeProfileId) ?? managerProfiles[0];

  return (
    <section className="grid gap-4">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <div className="text-sm font-semibold text-fusion-accent">{t.settings.emulators.title}</div>
          <p className="mt-2 max-w-2xl text-sm leading-6 text-white/[0.52]">
            {t.settings.emulators.copy}
          </p>
        </div>
        <div className="rounded-sm border border-white/10 bg-white/[0.035] px-3 py-2 text-xs font-bold text-white/[0.54]">
          {t.settings.emulators.active(activeProfile ? platformLabel(activeProfile.platform) : t.common.loading)}
        </div>
      </div>

      <div className="grid gap-3">
        {managerProfiles.length === 0 && (
          <div className="rounded-sm border border-white/10 bg-black/[0.34] p-4 text-sm text-white/[0.52]">
            {t.settings.diagnostics.profilesLoading}
          </div>
        )}

        {managerProfiles.map((profile) => {
          const emulatorPath = getEmulatorPath(settings, profile.platform);
          const config = getEmulatorConfig(settings, profile.platform);
          const status = emulatorStatus(emulatorPath, config?.status, {
            ready: t.settings.statusChip.ready,
            fileMoved: t.settings.statusChip.fileMoved,
            invalid: t.settings.statusChip.invalid,
            notSet: t.settings.statusChip.notSet
          });
          const active = activeProfileId === profile.id;
          const installMode = profile.emulator.installMode;
          const downloadable = installMode === 'downloadable';
          const rowBusy = busy === `install:${profile.id}`
            || busy === `remove:${profile.id}`
            || busy === `select:${profile.id}`
            || busy === `open:${profile.id}`;
          const progress = progressByProfileId[profile.id];
          const executableHint = profile.emulator.executableCandidates.join(', ')
            || profile.emulator.executableName
            || 'emulator.exe';
          const detail = downloadable
            ? t.settings.emulators.downloadableDetail(profile.emulator.emulatorName)
            : t.settings.emulators.manualDetail(profile.emulator.emulatorName);

          return (
            <article
              key={profile.id}
              data-testid={`emulator-row-${profile.platform}`}
              onFocusCapture={() => onFocusProfile(profile.id)}
              className={`rounded-sm border p-4 transition ${
                active
                  ? 'border-fusion-accent/45 bg-fusion-accent/[0.07] shadow-[0_0_0_1px_rgba(92,230,140,0.18),0_20px_60px_rgba(0,0,0,0.48)]'
                  : 'border-white/10 bg-black/[0.34] hover:border-white/[0.24] hover:bg-white/[0.045]'
              }`}
            >
              <div className="grid gap-4 lg:grid-cols-[minmax(170px,230px)_minmax(0,1fr)_auto] lg:items-center">
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-sm font-black text-white/90">{platformLabel(profile.platform)}</span>
                    <StatusChip tone={status.tone} label={status.label} />
                  </div>
                  <div className="mt-2 text-xs text-white/[0.38]">{profile.emulator.emulatorName}</div>
                </div>

                <div className="min-w-0">
                  <div className="text-xs leading-5 text-white/[0.50]">{detail}</div>
                  <div className="mt-1 text-xs text-white/[0.34]">{t.settings.emulators.expectedFile(executableHint)}</div>
                  {emulatorPath && (
                    <div className="mt-2 truncate text-xs text-white/[0.48]" title={emulatorPath}>
                      {emulatorPath}
                    </div>
                  )}
                  {config?.version && (
                    <div className="mt-1 text-xs text-white/[0.34]">
                      {t.common.version}: {config.version}
                    </div>
                  )}
                  {progress && busy === `install:${profile.id}` && (
                    <div className="mt-3">
                      <div className="flex items-center justify-between gap-3 text-[10px] font-semibold text-white/[0.48]">
                        <span>{progress.message}</span>
                        <span>{progress.percent}%</span>
                      </div>
                      <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-white/[0.08]">
                        <div className="h-full rounded-full bg-fusion-accent transition-all" style={{ width: `${progress.percent}%` }} />
                      </div>
                    </div>
                  )}
                </div>

                <div className="flex flex-wrap justify-start gap-2 lg:justify-end">
                  {status.ready ? (
                    <>
                      <button
                        type="button"
                        onClick={() => onOpenFolder(profile)}
                        disabled={busy !== null}
                        className="rh-mini-action h-10"
                      >
                        {busy === `open:${profile.id}` ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <FolderOpen className="h-3.5 w-3.5" />}
                        {t.settings.emulators.openFolder}
                      </button>
                      <button
                        type="button"
                        onClick={() => onRemove(profile)}
                        disabled={busy !== null}
                        className="inline-flex h-10 items-center gap-2 rounded-sm border border-red-300/20 px-3 text-xs font-bold text-red-100/80 transition hover:bg-red-300/10 disabled:opacity-40"
                      >
                        {busy === `remove:${profile.id}` ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Trash2 className="h-3.5 w-3.5" />}
                        {t.common.remove}
                      </button>
                    </>
                  ) : downloadable ? (
                    <>
                      <button
                        type="button"
                        onClick={() => onInstall(profile)}
                        disabled={busy !== null}
                        className="rh-mini-action h-10"
                        data-testid={`settings-install-${profile.platform}`}
                      >
                        {rowBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Download className="h-3.5 w-3.5" />}
                        {t.common.download}
                      </button>
                      <button
                        type="button"
                        onClick={() => onInstallTo(profile)}
                        disabled={busy !== null}
                        title="Download to..."
                        className="grid h-10 w-10 place-items-center rounded-sm border border-white/10 text-white/70 transition hover:bg-white/10 disabled:opacity-40"
                      >
                        {rowBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <FolderOpen className="h-3.5 w-3.5" />}
                      </button>
                    </>
                  ) : (
                    <button
                      type="button"
                      onClick={() => onSelect(profile)}
                      disabled={busy !== null}
                      className="rh-mini-action h-10"
                    >
                      {rowBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Wrench className="h-3.5 w-3.5" />}
                      {t.common.select}
                    </button>
                  )}
                </div>
              </div>
            </article>
          );
        })}
      </div>
    </section>
  );
}

function emulatorStatus(
  exePath: string,
  status: string | undefined,
  labels: { ready: string; fileMoved: string; invalid: string; notSet: string }
) {
  if (exePath && (status === 'valid' || !status)) {
    return { ready: true, tone: 'valid' as const, label: labels.ready };
  }
  if (exePath && status === 'missing') {
    return { ready: false, tone: 'missing' as const, label: labels.fileMoved };
  }
  if (exePath && status === 'invalid') {
    return { ready: false, tone: 'invalid' as const, label: labels.invalid };
  }
  return { ready: false, tone: 'empty' as const, label: labels.notSet };
}

function platformLabel(platform: string) {
  return PLATFORM_LABELS[platform as keyof typeof PLATFORM_LABELS] ?? platform.toUpperCase();
}
