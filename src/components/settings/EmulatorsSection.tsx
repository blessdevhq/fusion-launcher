import { FolderOpen, Loader2 } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import type { Locale } from '@/lib/i18n';
import { getEmulatorPath, type AppSettings } from '@/lib/settings';
import { getEmulatorDraftState } from '@/lib/settingsModalState';
import { MVP_PLATFORMS, PLATFORM_EMULATOR_HINTS, PLATFORM_LABELS, type MvpPlatform } from '@/types/platform';
import { StatusChip } from './shared';
import type { BusyState } from './types';

export function EmulatorsSection({
  draftSettings,
  savedSettings,
  activePlatform,
  busy,
  locale,
  onFocusPlatform,
  onPathChange,
  onBrowse
}: {
  draftSettings: AppSettings;
  savedSettings: AppSettings;
  activePlatform: MvpPlatform;
  busy: BusyState;
  locale: Locale;
  onFocusPlatform: (platform: MvpPlatform) => void;
  onPathChange: (platform: MvpPlatform, path: string) => void;
  onBrowse: (platform: MvpPlatform) => Promise<void>;
}) {
  const { t } = useI18n();

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
          {t.settings.emulators.active(PLATFORM_LABELS[activePlatform])}
        </div>
      </div>

      <div className="grid gap-3">
        {MVP_PLATFORMS.map((platform) => {
          const emulatorPath = getEmulatorPath(draftSettings, platform);
          const state = getEmulatorDraftState(draftSettings, savedSettings, platform, locale);
          const active = activePlatform === platform;
          const browsing = busy === `browse:${platform}`;

          return (
            <article
              key={platform}
              data-testid={`emulator-row-${platform}`}
              onFocusCapture={() => onFocusPlatform(platform)}
              className={`rounded-sm border p-4 transition ${
                active
                  ? 'border-fusion-accent/45 bg-fusion-accent/[0.07] shadow-[0_0_0_1px_rgba(92,230,140,0.18),0_20px_60px_rgba(0,0,0,0.48)]'
                  : 'border-white/10 bg-black/[0.34] hover:border-white/[0.24] hover:bg-white/[0.045]'
              }`}
            >
              <div className="grid gap-4 lg:grid-cols-[minmax(150px,220px)_minmax(0,1fr)]">
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-black text-white/90">{PLATFORM_LABELS[platform]}</span>
                    <StatusChip tone={state.tone} label={state.label} />
                  </div>
                  <div className="mt-2 text-xs text-white/[0.38]">{t.settings.emulators.expectedFile(PLATFORM_EMULATOR_HINTS[platform])}</div>
                  <div className="mt-3 text-xs leading-5 text-white/[0.44]">{state.detail}</div>
                </div>

                <label className="min-w-0">
                  <span className="sr-only">{t.settings.emulators.pathSr(PLATFORM_LABELS[platform])}</span>
                  <div className="flex min-w-0 gap-2">
                    <input
                      value={emulatorPath}
                      onChange={(event) => onPathChange(platform, event.target.value)}
                      onFocus={() => onFocusPlatform(platform)}
                      className="h-11 min-w-0 flex-1 rounded-sm border border-white/10 bg-black/40 px-3 text-sm text-white/80 outline-none transition placeholder:text-white/25 focus:border-white/60"
                      placeholder={t.settings.emulators.pathPlaceholder}
                      spellCheck={false}
                    />
                    <button
                      type="button"
                      onClick={() => onBrowse(platform)}
                      disabled={busy !== null}
                      aria-label={t.settings.emulators.chooseExecutable(PLATFORM_LABELS[platform])}
                      className="inline-flex h-11 shrink-0 items-center gap-2 rounded-sm border border-white/[0.12] bg-white/[0.045] px-3 text-sm font-bold text-white/70 transition hover:border-white/[0.44] hover:bg-white/[0.08] hover:text-white disabled:opacity-40 sm:px-4"
                    >
                      {browsing ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                      ) : (
                        <FolderOpen className="h-4 w-4" />
                      )}
                      <span className="hidden sm:inline">{t.common.browse}</span>
                    </button>
                  </div>
                </label>
              </div>
            </article>
          );
        })}
      </div>
    </section>
  );
}
