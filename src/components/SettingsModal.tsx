'use client';

import { FormEvent, useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen, Loader2, Save, X } from 'lucide-react';
import { isTauriRuntime } from '@/lib/runtime';
import { api } from '@/lib/api';
import { getEmulatorConfig, getEmulatorPath, setEmulatorPath, type AppSettings } from '@/lib/settings';
import { MVP_PLATFORMS, PLATFORM_EMULATOR_HINTS, PLATFORM_LABELS, type Platform } from '@/types/platform';

interface SettingsModalProps {
  settings: AppSettings;
  onClose: () => void;
  onSave: (settings: AppSettings) => Promise<void>;
}

type BusyState = `browse:${Platform}` | 'save' | null;

export function SettingsModal({ settings, onClose, onSave }: SettingsModalProps) {
  const [draftSettings, setDraftSettings] = useState<AppSettings>(settings);
  const [busy, setBusy] = useState<BusyState>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [downloadRoot, setDownloadRoot] = useState('');

  useEffect(() => {
    setDraftSettings(settings);
    setMessage(null);
  }, [settings]);

  useEffect(() => {
    let cancelled = false;
    api.getDownloadRoot()
      .then((value) => {
        if (!cancelled) setDownloadRoot(value);
      })
      .catch((error) => {
        if (!cancelled) setMessage(`Failed to load download folder: ${error}`);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const updateEmulatorPath = (platform: Platform, emulatorPath: string) => {
    setDraftSettings((currentSettings) => setEmulatorPath(currentSettings, platform, emulatorPath));
  };

  const browseForEmulator = async (platform: Platform) => {
    setBusy(`browse:${platform}`);
    setMessage(null);
    try {
      if (!isTauriRuntime()) {
        setMessage('File browsing is available in the desktop build. Paste a preview path to test readiness states.');
        return;
      }

      const currentPath = getEmulatorPath(draftSettings, platform);
      const selected = await open({
        title: `Select emulator for ${PLATFORM_LABELS[platform]}`,
        multiple: false,
        directory: false,
        defaultPath: currentPath || undefined,
        filters: [
          {
            name: 'Executable',
            extensions: ['exe']
          }
        ]
      });

      if (typeof selected === 'string') {
        updateEmulatorPath(platform, selected);
      }
    } catch (error) {
      setMessage(`Failed to open file picker: ${error}`);
    } finally {
      setBusy(null);
    }
  };

  const save = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setBusy('save');
    setMessage(null);
    try {
      await onSave(draftSettings);
      if (downloadRoot.trim()) {
        await api.setDownloadRoot(downloadRoot.trim());
      }
      onClose();
    } catch (error) {
      setMessage(`Failed to save settings: ${error}`);
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/72 px-5">
      <section className="w-full max-w-xl rounded-lg border border-white/12 bg-[#141417] shadow-2xl">
        <header className="flex items-start gap-4 border-b border-white/10 p-5">
          <div className="min-w-0 flex-1">
            <h2 className="text-2xl font-black">Settings</h2>
            <div className="mt-1 text-sm text-white/46">Platform emulator configuration</div>
          </div>
          <button
            onClick={onClose}
            className="grid h-9 w-9 place-items-center rounded-md border border-white/10 text-white/60 transition hover:text-white"
            title="Close"
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        <form onSubmit={save} className="space-y-5 p-5">
          <div>
            <div className="text-sm font-bold text-white/84">Emulators</div>
            <div className="mt-3 max-h-[52vh] space-y-3 overflow-y-auto pr-1">
              {MVP_PLATFORMS.map((platform) => {
                const browsing = busy === `browse:${platform}`;
                const emulatorPath = getEmulatorPath(draftSettings, platform);
                const config = getEmulatorConfig(draftSettings, platform);
                const status = deriveStatusLabel(emulatorPath, config?.exePath, config?.status);

                return (
                  <label
                    key={platform}
                    className="grid gap-2 rounded-md border border-white/10 bg-black/18 p-3"
                  >
                    <span className="text-sm font-semibold text-white/76">
                      {PLATFORM_LABELS[platform]}
                    </span>
                    <span className="text-xs text-white/38">Expected: {PLATFORM_EMULATOR_HINTS[platform]}</span>
                    <div className="flex gap-2">
                      <input
                        value={emulatorPath}
                        onChange={(event) => updateEmulatorPath(platform, event.target.value)}
                        className="h-10 min-w-0 flex-1 rounded-md border border-white/12 bg-black/24 px-3 text-sm text-white/88 outline-none transition placeholder:text-white/28 focus:border-hydra-accent/70"
                        placeholder="Select emulator executable"
                      />
                      <button
                        type="button"
                        onClick={() => browseForEmulator(platform)}
                        disabled={busy !== null}
                        className="inline-flex h-10 items-center gap-2 rounded-md border border-white/10 bg-white/[0.06] px-4 text-sm font-semibold text-white/76 transition hover:bg-white/12 disabled:opacity-40"
                      >
                        {browsing ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <FolderOpen className="h-4 w-4" />
                        )}
                        Browse
                      </button>
                    </div>
                    <span className={`w-fit rounded px-2 py-1 text-[11px] font-bold uppercase ${status.className}`}>
                      {status.label}
                    </span>
                  </label>
                );
              })}
            </div>
          </div>

          <label className="block">
            <span className="text-sm font-bold text-white/84">Download folder</span>
            <input
              value={downloadRoot}
              onChange={(event) => setDownloadRoot(event.target.value)}
              className="mt-3 h-10 w-full rounded-md border border-white/12 bg-black/24 px-3 text-sm text-white/88 outline-none transition placeholder:text-white/28 focus:border-hydra-accent/70"
              placeholder="D:\\Games\\RetroHydra"
            />
          </label>

          {message && (
            <div className="rounded-md border border-amber-300/24 bg-amber-300/10 px-3 py-2 text-sm text-amber-100">
              {message}
            </div>
          )}

          <div className="flex justify-end gap-2 border-t border-white/10 pt-5">
            <button
              type="button"
              onClick={onClose}
              disabled={busy !== null}
              className="h-10 rounded-md border border-white/10 px-4 text-sm font-semibold text-white/66 transition hover:bg-white/10 disabled:opacity-40"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={busy !== null}
              className="inline-flex h-10 items-center gap-2 rounded-md bg-hydra-accent px-4 text-sm font-bold text-white shadow-glow transition hover:bg-violet-500 disabled:opacity-40"
            >
              {busy === 'save' ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Save className="h-4 w-4" />
              )}
              Save
            </button>
          </div>
        </form>
      </section>
    </div>
  );
}

function deriveStatusLabel(
  emulatorPath: string,
  savedPath: string | undefined,
  savedStatus: string | undefined
) {
  if (!emulatorPath) {
    return { label: 'Not set', className: 'bg-white/8 text-white/42' };
  }
  if (emulatorPath !== (savedPath ?? '')) {
    return { label: 'Invalid path', className: 'bg-amber-300/12 text-amber-100' };
  }
  if (savedStatus === 'valid') {
    return { label: 'Valid', className: 'bg-emerald-300/12 text-emerald-100' };
  }
  if (savedStatus === 'missing') {
    return { label: 'File moved', className: 'bg-amber-300/12 text-amber-100' };
  }

  return { label: 'Invalid path', className: 'bg-red-300/12 text-red-100' };
}
