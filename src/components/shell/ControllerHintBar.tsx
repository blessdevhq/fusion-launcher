'use client';

import type { LauncherView } from '../../stores/launcherStore.ts';

const VIEW_HINTS: Record<LauncherView, Array<{ key: string; label: string; tone: string }>> = {
  home: [
    { key: 'A', label: 'Select', tone: 'bg-green-500' },
    { key: 'B', label: 'Back', tone: 'bg-red-500' },
    { key: 'Y', label: 'Details', tone: 'bg-yellow-500' },
    { key: 'X', label: 'Search', tone: 'bg-blue-500' }
  ],
  library: [
    { key: 'A', label: 'Open', tone: 'bg-green-500' },
    { key: 'B', label: 'Home', tone: 'bg-red-500' },
    { key: 'Y', label: 'Filters', tone: 'bg-yellow-500' }
  ],
  downloads: [
    { key: 'A', label: 'Action', tone: 'bg-green-500' },
    { key: 'B', label: 'Home', tone: 'bg-red-500' },
    { key: 'X', label: 'Retry', tone: 'bg-blue-500' }
  ],
  settings: [
    { key: 'A', label: 'Open', tone: 'bg-green-500' },
    { key: 'B', label: 'Home', tone: 'bg-red-500' },
    { key: 'Y', label: 'Reset', tone: 'bg-yellow-500' }
  ],
  explore: [
    { key: 'A', label: 'Open', tone: 'bg-green-500' },
    { key: 'B', label: 'Home', tone: 'bg-red-500' }
  ],
  collections: [
    { key: 'A', label: 'Open', tone: 'bg-green-500' },
    { key: 'B', label: 'Home', tone: 'bg-red-500' }
  ]
};

export function ControllerHintBar({ activeView }: { activeView: LauncherView }) {
  return (
    <div className="rh-hint-bar">
      {VIEW_HINTS[activeView].map((hint) => (
        <div key={`${hint.key}:${hint.label}`} className="flex items-center gap-2">
          <span className={`grid h-4 w-4 place-items-center rounded-full text-[10px] font-black text-black ${hint.tone}`}>
            {hint.key}
          </span>
          <span>{hint.label}</span>
        </div>
      ))}
    </div>
  );
}
