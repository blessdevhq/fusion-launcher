'use client';

import { useI18n } from '../I18nProvider';
import type { LauncherView } from '../../stores/launcherStore.ts';

type HintLabel = 'select' | 'back' | 'details' | 'open' | 'home' | 'filters' | 'action' | 'retry';

const VIEW_HINTS: Record<LauncherView, Array<{ key: string; label: HintLabel; tone: string }>> = {
  home: [
    { key: 'A', label: 'select', tone: 'primary' },
    { key: 'B', label: 'back', tone: 'quiet' },
    { key: 'Y', label: 'details', tone: 'warm' }
  ],
  library: [
    { key: 'A', label: 'open', tone: 'primary' },
    { key: 'B', label: 'home', tone: 'quiet' },
    { key: 'Y', label: 'filters', tone: 'warm' }
  ],
  downloads: [
    { key: 'A', label: 'action', tone: 'primary' },
    { key: 'B', label: 'home', tone: 'quiet' },
    { key: 'X', label: 'retry', tone: 'cool' }
  ]
};

export function ControllerHintBar({ activeView }: { activeView: LauncherView }) {
  const { t } = useI18n();

  return (
    <div className="rh-hint-bar">
      {VIEW_HINTS[activeView].map((hint) => (
        <div key={`${hint.key}:${hint.label}`} className="flex items-center gap-2">
          <span className={`rh-hint-key rh-hint-key-${hint.tone}`}>
            {hint.key}
          </span>
          <span>{t.shell.hints[hint.label]}</span>
        </div>
      ))}
    </div>
  );
}
