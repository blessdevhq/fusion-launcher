import { Settings } from 'lucide-react';
import { useI18n } from '@/components/I18nProvider';
import { LOCALES, type Locale } from '@/lib/i18n';
import { EMULATOR_MANAGER_PLATFORMS } from '@/types/platform';
import type { HealthReport } from '@/types/repository';
import { SummaryCard } from './shared';
import type { SettingsSection, UpdatePanelPhase } from './types';
import { updatePhaseLabel } from './utils';

export function GeneralSection({
  configuredCount,
  readyCount,
  repositoriesCount,
  activeDownloadsCount,
  updatePhase,
  healthReport,
  hasUnsavedChanges,
  desktopBridge,
  draftLanguage,
  onLanguageChange,
  onOpenSection
}: {
  configuredCount: number;
  readyCount: number;
  repositoriesCount: number;
  activeDownloadsCount: number;
  updatePhase: UpdatePanelPhase;
  healthReport: HealthReport | null;
  hasUnsavedChanges: boolean;
  desktopBridge: boolean;
  draftLanguage: Locale;
  onLanguageChange: (language: Locale) => void;
  onOpenSection: (section: SettingsSection) => void;
}) {
  const { t } = useI18n();
  const healthReady = healthReport
    ? [
        ...healthReport.emulators,
        ...healthReport.platformSetup,
        ...healthReport.systemFiles,
        ...healthReport.gameFiles,
        ...healthReport.repositories,
        healthReport.downloader
      ].filter((item) => item.status === 'ready').length
    : 0;
  const healthTotal = healthReport
    ? healthReport.emulators.length
      + healthReport.platformSetup.length
      + healthReport.systemFiles.length
      + healthReport.gameFiles.length
      + healthReport.repositories.length
      + 1
    : 0;

  return (
    <section className="grid gap-4">
      <div className="grid gap-3 sm:grid-cols-3">
        <SummaryCard label={t.settings.general.configured} value={`${configuredCount}/${EMULATOR_MANAGER_PLATFORMS.length}`} />
        <SummaryCard label={t.settings.general.sources} value={String(repositoriesCount)} />
        <SummaryCard label={t.settings.general.downloads} value={String(activeDownloadsCount)} />
        <SummaryCard label={t.settings.general.ready} value={`${readyCount}/${EMULATOR_MANAGER_PLATFORMS.length}`} />
        <SummaryCard label={t.settings.general.health} value={healthTotal > 0 ? `${healthReady}/${healthTotal}` : t.common.notRun} />
        <SummaryCard label={t.settings.general.update} value={updatePhaseLabel(updatePhase, t)} />
      </div>
      <label className="rounded-sm border border-white/10 bg-black/[0.34] p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <div className="text-sm font-black text-white/90">{t.language.label}</div>
            <p className="mt-1 text-xs leading-5 text-white/[0.46]">{t.language.description}</p>
          </div>
          <select
            value={draftLanguage}
            onChange={(event) => onLanguageChange(event.target.value as Locale)}
            className="h-10 rounded-sm border border-white/10 bg-black/40 px-3 text-sm font-semibold text-white/80 outline-none transition focus:border-white/60"
            data-testid="settings-language"
          >
            {LOCALES.map((language) => (
              <option key={language} value={language}>{t.language.options[language]}</option>
            ))}
          </select>
        </div>
      </label>
      <div className="rounded-sm border border-white/10 bg-black/[0.38] p-5">
        <div className="flex items-start gap-4">
          <div className="grid h-11 w-11 shrink-0 place-items-center rounded-sm border border-white/10 bg-white/[0.055]">
            <Settings className="h-5 w-5 text-white/[0.78]" />
          </div>
          <div className="min-w-0">
            <h4 className="text-lg font-black">{t.settings.general.launcherSetup}</h4>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-white/[0.52]">
              {t.settings.general.copy}
            </p>
            <div className="mt-5 flex flex-wrap gap-2">
              <SectionJumpButton label={t.settings.sections.emulators} onClick={() => onOpenSection('emulators')} />
              <SectionJumpButton label={t.settings.sections.sources} onClick={() => onOpenSection('sources')} />
              <SectionJumpButton label={t.settings.sections.diagnostics} onClick={() => onOpenSection('diagnostics')} />
            </div>
          </div>
        </div>
      </div>
      <div className="rounded-sm border border-white/10 bg-white/[0.025] p-4 text-sm text-white/50">
        {hasUnsavedChanges
          ? t.settings.general.dirty
          : t.settings.general.synced(desktopBridge ? 'desktop bridge' : 'preview')}
      </div>
    </section>
  );
}

function SectionJumpButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="h-10 rounded-lg border border-white/[0.28] px-4 text-sm font-semibold text-white/[0.82] transition hover:border-fusion-accent/50 hover:bg-fusion-accent/10"
    >
      {label}
    </button>
  );
}
