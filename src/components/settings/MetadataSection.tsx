import { Ban, DatabaseZap, Loader2, RefreshCcw } from 'lucide-react';
import type { LibraryScrapeProgressEvent, ScreenScraperStatus, SteamGridDbStatus } from '@/types/repository';

export function MetadataSection({
  status,
  ssid,
  password,
  region,
  busy,
  steamStatus,
  steamKey,
  steamBusy,
  batchBusy,
  batchProgress,
  onSsidChange,
  onPasswordChange,
  onRegionChange,
  onSave,
  onSteamKeyChange,
  onSaveSteam,
  onScrapeLibrary,
  onCancelLibraryScrape
}: {
  status: ScreenScraperStatus | null;
  ssid: string;
  password: string;
  region: string;
  busy: boolean;
  steamStatus: SteamGridDbStatus | null;
  steamKey: string;
  steamBusy: boolean;
  batchBusy: boolean;
  batchProgress: LibraryScrapeProgressEvent | null;
  onSsidChange: (value: string) => void;
  onPasswordChange: (value: string) => void;
  onRegionChange: (value: string) => void;
  onSave: () => Promise<void>;
  onSteamKeyChange: (value: string) => void;
  onSaveSteam: () => Promise<void>;
  onScrapeLibrary: () => Promise<void>;
  onCancelLibraryScrape: () => Promise<void>;
}) {
  const running = steamStatus?.batchRunning ?? false;
  const progressPercent = batchProgress && batchProgress.total > 0
    ? Math.min(100, Math.round((batchProgress.done / batchProgress.total) * 100))
    : 0;

  return (
    <section className="grid gap-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="text-sm font-semibold text-fusion-accent">ScreenScraper metadata</div>
          <p className="mt-2 max-w-2xl text-sm leading-6 text-white/[0.52]">
            Store your ScreenScraper account locally to fill missing game metadata and covers after imports.
          </p>
        </div>
        <span className={`rounded-lg border px-2 py-1 text-[10px] font-semibold ${
          status?.configured ? 'border-emerald-200/[0.24] bg-emerald-200/10 text-emerald-100' : 'border-amber-200/[0.24] bg-amber-200/10 text-amber-100'
        }`}>
          {status?.configured ? 'Configured' : 'Not configured'}
        </span>
      </div>

      <div className="grid gap-3 lg:grid-cols-2">
        <label className="rounded-sm border border-white/10 bg-black/[0.34] p-4">
          <span className="text-sm font-black text-white/90">ScreenScraper SSID</span>
          <input
            value={ssid}
            onChange={(event) => onSsidChange(event.target.value)}
            className="mt-3 h-11 w-full rounded-sm border border-white/10 bg-black/40 px-3 text-sm text-white/80 outline-none transition placeholder:text-white/25 focus:border-white/60"
            placeholder="username"
            autoComplete="username"
            spellCheck={false}
          />
        </label>

        <label className="rounded-sm border border-white/10 bg-black/[0.34] p-4">
          <span className="text-sm font-black text-white/90">Password</span>
          <input
            value={password}
            onChange={(event) => onPasswordChange(event.target.value)}
            className="mt-3 h-11 w-full rounded-sm border border-white/10 bg-black/40 px-3 text-sm text-white/80 outline-none transition placeholder:text-white/25 focus:border-white/60"
            placeholder={status?.configured ? 'Leave blank to keep saved password' : 'password'}
            type="password"
            autoComplete="current-password"
          />
        </label>

        <label className="rounded-sm border border-white/10 bg-black/[0.34] p-4">
          <span className="text-sm font-black text-white/90">Cover region</span>
          <select
            value={region}
            onChange={(event) => onRegionChange(event.target.value)}
            className="mt-3 h-11 w-full rounded-sm border border-white/10 bg-black/40 px-3 text-sm text-white/80 outline-none transition focus:border-white/60"
          >
            <option value="auto">Auto</option>
            <option value="eu">Europe</option>
            <option value="us">United States</option>
            <option value="jp">Japan</option>
          </select>
        </label>

        <div className="rounded-sm border border-white/10 bg-black/[0.34] p-4">
          <div className="text-sm font-black text-white/90">Daily request budget</div>
          <div className="mt-3 text-2xl font-black text-white">
            {status ? `${status.dailyRequests}/${status.dailyLimit}` : '...'}
          </div>
          <div className="mt-2 text-xs text-white/[0.42]">Tracked locally per calendar day.</div>
        </div>
      </div>

      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={() => void onSave()}
          disabled={busy || !ssid.trim()}
          className="rh-mini-action"
        >
          {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <DatabaseZap className="h-3.5 w-3.5" />}
          Save metadata settings
        </button>
      </div>

      <div className="rounded-sm border border-white/10 bg-black/[0.34] p-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <div className="text-sm font-semibold text-fusion-accent">SteamGridDB artwork</div>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-white/[0.52]">
              Adds hero, logo, and grid artwork after ScreenScraper metadata without proxying requests.
            </p>
          </div>
          <span className={`rounded-lg border px-2 py-1 text-[10px] font-semibold ${
            steamStatus?.configured ? 'border-emerald-200/[0.24] bg-emerald-200/10 text-emerald-100' : 'border-amber-200/[0.24] bg-amber-200/10 text-amber-100'
          }`}>
            {steamStatus?.keySource === 'built-in' ? 'Built-in key' : steamStatus?.keySource === 'user' ? 'User key' : 'No key'}
          </span>
        </div>

        <div className="mt-4 grid gap-3 lg:grid-cols-[minmax(0,1fr)_220px]">
          <label className="min-w-0">
            <span className="text-sm font-black text-white/90">SteamGridDB API key</span>
            <input
              value={steamKey}
              onChange={(event) => onSteamKeyChange(event.target.value)}
              className="mt-3 h-11 w-full rounded-sm border border-white/10 bg-black/40 px-3 text-sm text-white/80 outline-none transition placeholder:text-white/25 focus:border-white/60"
              placeholder="Leave blank to use the built-in key"
              type="password"
              autoComplete="off"
              spellCheck={false}
            />
          </label>

          <div className="rounded-sm border border-white/10 bg-white/[0.035] p-3">
            <div className="text-[10px] font-semibold text-white/[0.32]">SGDB daily requests</div>
            <div className="mt-2 text-xl font-black text-white">
              {steamStatus ? `${steamStatus.dailyRequests}/${steamStatus.dailyLimit}` : '...'}
            </div>
            <div className="mt-1 text-xs text-white/[0.38]">{steamStatus?.pendingBatch ?? 0} queued</div>
          </div>
        </div>

        {(batchProgress || running) && (
          <div className="mt-4 rounded-sm border border-white/10 bg-black/30 p-3">
            <div className="mb-2 flex items-center justify-between gap-3 text-xs text-white/[0.54]">
              <span>{batchProgress?.currentGameId ?? (running ? 'Scraping library' : 'Batch scrape')}</span>
              <span>{batchProgress ? `${batchProgress.done}/${batchProgress.total}` : `${steamStatus?.pendingBatch ?? 0} pending`}</span>
            </div>
            <div className="h-2 overflow-hidden rounded-sm bg-white/10">
              <div className="h-full bg-fusion-accent transition-all" style={{ width: `${progressPercent}%` }} />
            </div>
          </div>
        )}

        <div className="mt-4 flex flex-wrap gap-2">
          <button
            type="button"
            onClick={() => void onSaveSteam()}
            disabled={steamBusy}
            className="rh-mini-action"
          >
            {steamBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <DatabaseZap className="h-3.5 w-3.5" />}
            Save SteamGridDB key
          </button>
          <button
            type="button"
            onClick={() => void onScrapeLibrary()}
            disabled={batchBusy || running}
            className="rh-mini-action"
          >
            {batchBusy && !running ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCcw className="h-3.5 w-3.5" />}
            Scrape entire library
          </button>
          <button
            type="button"
            onClick={() => void onCancelLibraryScrape()}
            disabled={batchBusy || !running}
            className="rh-mini-action"
          >
            {batchBusy && running ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Ban className="h-3.5 w-3.5" />}
            Cancel
          </button>
        </div>
      </div>
    </section>
  );
}
