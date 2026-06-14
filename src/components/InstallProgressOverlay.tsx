import type { InstallProgressEvent } from '@/types/emulatorProfile';
import { useI18n } from './I18nProvider';

export function InstallProgressOverlay({ progress }: { progress: InstallProgressEvent }) {
  const { t } = useI18n();
  const percent = Math.min(Math.max(progress.percent, 0), 100);

  return (
    <div className="absolute inset-0 z-20 flex flex-col items-center justify-center rounded-2xl bg-fusion-bg/95 px-6 text-center backdrop-blur-xl">
      <p className="text-sm font-bold text-white">{t.installProgress[progress.stage] ?? progress.message}</p>
      <p className="mt-2 max-w-sm text-xs text-white/44">{progress.message}</p>
      <div className="mt-5 h-1 w-64 max-w-full overflow-hidden rounded-full bg-white/10">
        <div
          className="h-full rounded-full bg-fusion-green transition-[width] duration-500"
          style={{ width: `${percent}%` }}
        />
      </div>
      <p className="mt-3 font-mono text-xs text-white/40">{percent}%</p>
    </div>
  );
}
