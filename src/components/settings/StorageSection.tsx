import { useI18n } from '@/components/I18nProvider';
import { PathCard, StatusChip } from './shared';

export function StorageSection({
  downloadRoot,
  appDataDir,
  logPath,
  changed,
  onDownloadRootChange
}: {
  downloadRoot: string;
  appDataDir: string;
  logPath: string;
  changed: boolean;
  onDownloadRootChange: (value: string) => void;
}) {
  const { t } = useI18n();

  return (
    <section className="grid gap-4">
      <div>
          <div className="text-sm font-semibold text-fusion-accent">{t.settings.storage.title}</div>
          <p className="mt-2 max-w-2xl text-sm leading-6 text-white/[0.52]">
          {t.settings.storage.copy}
        </p>
      </div>
      <label className="rounded-sm border border-white/10 bg-black/[0.34] p-4">
        <div className="flex items-center justify-between gap-3">
          <span className="text-sm font-black text-white/90">{t.settings.storage.downloadFolder}</span>
          {changed && <StatusChip tone="unsaved" label={t.settings.statusChip.unsaved} />}
        </div>
        <input
          value={downloadRoot}
          onChange={(event) => onDownloadRootChange(event.target.value)}
          className="mt-3 h-11 w-full rounded-sm border border-white/10 bg-black/40 px-3 text-sm text-white/80 outline-none transition placeholder:text-white/25 focus:border-white/60"
          placeholder="D:\\Games\\FusionLauncher"
          spellCheck={false}
        />
      </label>
      <div className="grid gap-3 lg:grid-cols-2">
        <PathCard label={t.settings.storage.appData} value={appDataDir || t.common.loading} />
        <PathCard label={t.settings.storage.logs} value={logPath || t.common.loading} />
      </div>
    </section>
  );
}
