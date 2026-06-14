import { Activity, AlertTriangle, CheckCircle2, ShieldAlert } from 'lucide-react';
import type { ActivityEvent } from '@/stores/launcherStore';

export function ScreenHeader({ eyebrow, title, description }: { eyebrow: string; title: string; description: string }) {
  return (
    <header className="mb-5">
      <div className="text-sm font-semibold text-fusion-accent">{eyebrow}</div>
      <h1 className="mt-2 text-3xl font-bold tracking-normal">{title}</h1>
      <p className="mt-1 text-sm text-white/46">{description}</p>
    </header>
  );
}

export function ActivityIcon({ tone }: { tone: ActivityEvent['tone'] }) {
  return (
    <div className="grid h-9 w-9 shrink-0 place-items-center rounded-full bg-white/10">
      {tone === 'success' ? (
        <CheckCircle2 className="h-4 w-4 text-fusion-green" />
      ) : tone === 'error' ? (
        <AlertTriangle className="h-4 w-4 text-red-200" />
      ) : tone === 'warning' ? (
        <ShieldAlert className="h-4 w-4 text-amber-200" />
      ) : (
        <Activity className="h-4 w-4 text-white/60" />
      )}
    </div>
  );
}

export function StatsLine({ label, value }: { label: string; value: string }) {
  return (
    <div className="mt-3 flex items-center justify-between text-xs">
      <span className="text-white/48">{label}</span>
      <span className="font-bold">{value}</span>
    </div>
  );
}
