import { useI18n } from '@/components/I18nProvider';
import { UpdateCheckPanel } from './shared';
import type { UpdatePanelState } from './types';

export function UpdatesSection({
  state,
  onCheck,
  onInstall
}: {
  state: UpdatePanelState;
  onCheck: () => Promise<void>;
  onInstall: () => Promise<void>;
}) {
  const { t } = useI18n();

  return (
    <section className="grid gap-4" data-testid="settings-modal-updates-panel">
      <div>
        <div className="text-sm font-semibold text-fusion-accent">{t.settings.updates.title}</div>
        <p className="mt-2 max-w-2xl text-sm leading-6 text-white/[0.52]">
          {t.settings.updates.copy}
        </p>
      </div>
      <UpdateCheckPanel state={state} onCheck={onCheck} onInstall={onInstall} />
    </section>
  );
}
