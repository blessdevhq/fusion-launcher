import { useI18n } from '@/components/I18nProvider';
import { PathCard, StatusChip } from './shared';

export function StorageSection({
  libraryRoot,
  appDataDir,
  logPath,
  changed,
  onLibraryRootChange,
  onBrowse
}: {
  libraryRoot: string;
  appDataDir: string;
  logPath: string;
  changed: boolean;
  onLibraryRootChange: (value: string) => void;
  onBrowse?: () => void;
}) {
  const { t } = useI18n();
  const managedRoot = libraryRoot || t.common.loading;
  const gamesRoot = libraryRoot ? appendPath(libraryRoot, 'Games') : t.common.loading;
  const emulatorsRoot = libraryRoot ? appendPath(libraryRoot, 'Emulators') : t.common.loading;
  const systemRoot = libraryRoot ? appendPath(libraryRoot, 'System') : t.common.loading;

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
          <span className="text-sm font-black text-white/90">{t.settings.storage.libraryFolder}</span>
          {changed && <StatusChip tone="unsaved" label={t.settings.statusChip.unsaved} />}
        </div>
        <div className="mt-3 flex items-center gap-2">
          <input
            value={libraryRoot}
            onChange={(event) => onLibraryRootChange(event.target.value)}
            className="h-11 w-full rounded-sm border border-white/10 bg-black/40 px-3 text-sm text-white/80 outline-none transition placeholder:text-white/25 focus:border-white/60"
            placeholder="D:\\FusionLauncher"
            spellCheck={false}
          />
          {onBrowse && (
            <button
              type="button"
              onClick={onBrowse}
              className="h-11 shrink-0 rounded-sm border border-white/10 bg-white/[0.06] px-4 text-sm font-black text-white/80 transition hover:border-white/60 hover:text-white"
            >
              {t.settings.storage.browse}
            </button>
          )}
        </div>
      </label>
      <div className="grid gap-3 lg:grid-cols-2">
        <PathCard label="Library root" value={managedRoot} />
        <PathCard label="Games" value={gamesRoot} />
        <PathCard label="Emulators" value={emulatorsRoot} />
        <PathCard label="System" value={systemRoot} />
        <PathCard label={t.settings.storage.appData} value={appDataDir || t.common.loading} />
        <PathCard label={t.settings.storage.logs} value={logPath || t.common.loading} />
      </div>
    </section>
  );
}

function appendPath(root: string, child: string) {
  const separator = root.startsWith('preview://') || root.includes('/') ? '/' : '\\';
  return `${root.replace(/[\\/]+$/, '')}${separator}${child}`;
}
