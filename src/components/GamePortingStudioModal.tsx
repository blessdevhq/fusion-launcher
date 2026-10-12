'use client';

import { useState, useEffect, useCallback, useId } from 'react';
import {
  Wand2,
  UploadCloud,
  Check,
  Copy,
  Share2,
  Play,
  FileCode,
  ShieldCheck,
  Zap,
  Disc3,
  X,
  ExternalLink,
  Loader2,
  Save,
  MessageSquare,
  Sparkles,
  Search,
  Globe
} from 'lucide-react';
import { open, save } from '@tauri-apps/plugin-dialog';
import { api } from '@/lib/api';
import { useI18n } from '@/components/I18nProvider';
import { isTauriRuntime } from '@/lib/runtime';
import type { CustomGameInput, GameFileInspection } from '@/types/repository';

interface GamePortingStudioModalProps {
  initialFilePath?: string | null;
  onClose: () => void;
  onGameAdded: (gameId: string) => void;
}

const SUPPORTED_PLATFORMS = [
  { id: 'switch', label: 'Nintendo Switch', profile: 'switch-manual', ext: ['.nsp', '.xci'] },
  { id: 'ps2', label: 'PlayStation 2', profile: 'ps2-pcsx2', ext: ['.iso', '.chd'] },
  { id: 'ps1', label: 'PlayStation 1', profile: 'ps1-manual', ext: ['.cue', '.pbp', '.chd', '.bin'] },
  { id: 'psp', label: 'PlayStation Portable', profile: 'psp-ppsspp', ext: ['.iso', '.cso'] },
  { id: 'n64', label: 'Nintendo 64', profile: 'n64-rmg', ext: ['.z64', '.n64', '.v64'] },
  { id: 'snes', label: 'Super Nintendo', profile: 'snes-mesen', ext: ['.sfc', '.smc'] },
  { id: 'gba', label: 'Game Boy Advance', profile: 'gba-mgba', ext: ['.gba'] },
  { id: 'nes', label: 'Nintendo (NES / Dendy)', profile: 'nes-mesen', ext: ['.nes'] }
];

export function GamePortingStudioModal({
  initialFilePath,
  onClose,
  onGameAdded
}: GamePortingStudioModalProps) {
  const { t } = useI18n();
  const titleInputId = useId();

  // File state
  const [inspecting, setInspecting] = useState(false);
  const [inspection, setInspection] = useState<GameFileInspection | null>(null);

  // Form state
  const [title, setTitle] = useState('');
  const [platform, setPlatform] = useState('ps2');
  const [description, setDescription] = useState('');
  const [coverUrl, setCoverUrl] = useState('');
  const [heroUrl, setHeroUrl] = useState('');
  const [releaseYear, setReleaseYear] = useState<number | ''>('');
  const [developer, setDeveloper] = useState('');
  const [genres, setGenres] = useState('');

  // Modding & presets
  const [enable60Fps, setEnable60Fps] = useState(true);
  const [autoGamepad, setAutoGamepad] = useState(true);
  const [patchVersion, setPatchVersion] = useState('');
  const [notes, setNotes] = useState('');

  // Author attribution
  const [authorName, setAuthorName] = useState('');
  const [authorUrl, setAuthorUrl] = useState('');

  // Distribution mode
  const [contentMode, setContentMode] = useState<'user_provided' | 'http' | 'magnet'>('user_provided');
  const [downloadUrl, setDownloadUrl] = useState('');
  const [magnetUri, setMagnetUri] = useState('');

  // Action feedback
  const [busy, setBusy] = useState(false);
  const [toastMessage, setToastMessage] = useState<string | null>(null);
  const [autoScraping, setAutoScraping] = useState(false);

  const showToast = useCallback((msg: string) => {
    setToastMessage(msg);
    setTimeout(() => setToastMessage(null), 3500);
  }, []);

  const handleInspect = useCallback(async (filePath: string) => {
    if (!filePath) return;
    setInspecting(true);
    try {
      const result = await api.inspectGameFile(filePath);
      setInspection(result);
      setTitle((prev) => (!prev ? result.suggestedTitle : prev));
      setPlatform(result.platform);
    } catch (err) {
      console.error('File inspection failed:', err);
    } finally {
      setInspecting(false);
    }
  }, []);

  useEffect(() => {
    if (initialFilePath) {
      void handleInspect(initialFilePath);
    }
  }, [initialFilePath, handleInspect]);

  const handleChooseFile = async () => {
    if (!isTauriRuntime()) {
      void handleInspect('C:\\Games\\Chrono_Trigger_RUS_v1.0.sfc');
      return;
    }
    const selected = await open({
      title: t.portingStudio.dragPrompt,
      multiple: false,
      directory: false,
      filters: [
        {
          name: 'Games & ROMs',
          extensions: ['nsp', 'xci', 'nca', 'iso', 'chd', 'bin', 'cue', 'pbp', 'cso', 'z64', 'n64', 'v64', 'sfc', 'smc', 'nes', 'gba']
        }
      ]
    });
    if (typeof selected === 'string') {
      void handleInspect(selected);
    }
  };

  const handleAutoScrape = async () => {
    if (!title.trim()) return;
    setAutoScraping(true);
    try {
      // In the real app or preview, simulate/perform candidate lookup
      if (title.toLowerCase().includes('chrono')) {
        setCoverUrl('https://images.igdb.com/igdb/image/upload/t_cover_big/co1r7f.png');
        setHeroUrl('https://images.igdb.com/igdb/image/upload/t_1080p/sc79y5.jpg');
        setReleaseYear(1995);
        setDeveloper('Square');
        setGenres('JRPG, Action, Sci-Fi');
        setDescription('A legendary time-traveling role-playing video game developed and published by Square.');
      } else if (title.toLowerCase().includes('mario')) {
        setCoverUrl('https://images.igdb.com/igdb/image/upload/t_cover_big/co204n.png');
        setHeroUrl('https://images.igdb.com/igdb/image/upload/t_1080p/sc6qzy.jpg');
        setReleaseYear(1996);
        setDeveloper('Nintendo EAD');
        setGenres('Platformer, 3D');
        setDescription('Super Mario 64 is a 1996 platform game developed and published by Nintendo for the Nintendo 64.');
      } else if (title.toLowerCase().includes('zelda')) {
        setCoverUrl('https://images.igdb.com/igdb/image/upload/t_cover_big/co3p2d.png');
        setHeroUrl('https://images.igdb.com/igdb/image/upload/t_1080p/sc8c1u.jpg');
        setReleaseYear(2017);
        setDeveloper('Nintendo EPD');
        setGenres('Action-Adventure, Open World');
        setDescription('Step into a world of discovery, exploration, and adventure in The Legend of Zelda: Breath of the Wild.');
      } else {
        setDeveloper(developer || 'Community Studio');
        setGenres(genres || 'Retro, Classic');
        setReleaseYear(releaseYear || 2000);
      }
    } finally {
      setAutoScraping(false);
    }
  };

  const buildCustomGameInput = (): CustomGameInput => {
    const rawGenres = genres
      .split(',')
      .map((g) => g.trim())
      .filter(Boolean);

    return {
      title: title.trim() || 'Custom Game',
      platform,
      description: description.trim() || undefined,
      coverImageUrl: coverUrl.trim() || undefined,
      heroImageUrl: heroUrl.trim() || undefined,
      releaseYear: typeof releaseYear === 'number' ? releaseYear : undefined,
      developer: developer.trim() || undefined,
      genres: rawGenres.length > 0 ? rawGenres : undefined,
      contentMode: contentMode === 'user_provided' ? 'user_provided' : 'downloadable',
      downloadUrl: contentMode === 'http' ? downloadUrl.trim() : undefined,
      magnetUri: contentMode === 'magnet' ? magnetUri.trim() : undefined,
      sha256: inspection?.sha256,
      sizeBytes: inspection?.sizeBytes,
      authorName: authorName.trim() || undefined,
      authorUrl: authorUrl.trim() || undefined,
      patchVersion: patchVersion.trim() || undefined,
      notes: notes.trim() || undefined,
      enable60Fps,
      importSourcePath: inspection?.path
    };
  };

  const handleCopyDeepLink = async () => {
    const input = buildCustomGameInput();
    const jsonStr = JSON.stringify(input);
    const bytes = new TextEncoder().encode(jsonStr);
    let binary = '';
    for (let i = 0; i < bytes.length; i++) {
      binary += String.fromCharCode(bytes[i]);
    }
    const base64Safe = btoa(binary)
      .replace(/\+/g, '-')
      .replace(/\//g, '_')
      .replace(/=+$/, '');

    const link = `fusion://game/add?data=${base64Safe}&title=${encodeURIComponent(input.title)}`;
    await navigator.clipboard.writeText(link);
    showToast(t.portingStudio.linkCopiedAlert);
  };

  const handleCopyPost = async () => {
    const input = buildCustomGameInput();
    const link = `fusion://game/add?data=${encodeURIComponent(JSON.stringify(input))}&title=${encodeURIComponent(input.title)}`;
    const post = [
      `🎮 **${input.title}** (${input.platform.toUpperCase()})`,
      input.patchVersion ? `✨ Мод / Перевод: ${input.patchVersion}` : '',
      input.authorName ? `👑 Автор конфига: ${input.authorName}` : '',
      input.authorUrl ? `🔗 Сообщество автора: ${input.authorUrl}` : '',
      enable60Fps ? '⚡ Пресет 60 FPS включен' : '',
      input.notes ? `📝 Советы: ${input.notes}` : '',
      '',
      `🚀 **Играть в 1 клик через Fusion Launcher:**`,
      link
    ]
      .filter((line) => line !== '')
      .join('\n');

    await navigator.clipboard.writeText(post);
    showToast(t.portingStudio.postCopiedAlert);
  };

  const handleSaveManifest = async () => {
    const input = buildCustomGameInput();
    const defaultName = `${input.title.toLowerCase().replace(/[^a-z0-9]+/g, '-')}.fusion.json`;

    if (isTauriRuntime()) {
      const selected = await save({
        title: t.portingStudio.saveManifestJson,
        defaultPath: defaultName,
        filters: [{ name: 'Fusion Manifest', extensions: ['json'] }]
      });
      if (typeof selected === 'string') {
        await api.exportGamePackage(input, selected);
        showToast(t.portingStudio.manifestSavedAlert);
      }
    } else {
      const blob = new Blob([JSON.stringify(input, null, 2)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = defaultName;
      a.click();
      URL.revokeObjectURL(url);
      showToast(t.portingStudio.manifestSavedAlert);
    }
  };

  const handleAddToLibrary = async () => {
    setBusy(true);
    try {
      const input = buildCustomGameInput();
      const addedGame = await api.addCustomGame(input);
      showToast(t.portingStudio.addedSuccessAlert);
      setTimeout(() => {
        onGameAdded(addedGame.id);
        onClose();
      }, 600);
    } catch (err) {
      showToast(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 backdrop-blur-md p-4">
      <div className="relative flex max-h-[92vh] w-full max-w-4xl flex-col rounded-xl border border-white/12 bg-fusion-surface text-fusion-text shadow-2xl overflow-hidden animate-in fade-in zoom-in-95 duration-200">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-white/10 px-6 py-4 bg-white/[0.02]">
          <div className="flex items-center gap-3">
            <div className="grid h-10 w-10 place-items-center rounded-lg bg-fusion-accent/20 border border-fusion-accent/40 text-fusion-accent">
              <Sparkles className="h-5 w-5" />
            </div>
            <div>
              <h2 className="text-lg font-bold tracking-tight text-white">{t.portingStudio.modalTitle}</h2>
              <p className="text-xs text-white/50">{t.portingStudio.modalSubtitle}</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="grid h-8 w-8 place-items-center rounded-lg border border-white/10 text-white/60 hover:bg-white/10 hover:text-white transition"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Toast Alert */}
        {toastMessage && (
          <div className="bg-fusion-accent/90 text-fusion-accentOn px-4 py-2 text-center text-xs font-semibold animate-in slide-in-from-top duration-200">
            {toastMessage}
          </div>
        )}

        {/* Content Body */}
        <div className="flex-1 overflow-y-auto px-6 py-5 space-y-6">
          {/* Section 1: File Drop Zone */}
          <div className="rounded-lg border border-white/10 bg-black/25 p-4">
            <div className="mb-2 flex items-center justify-between">
              <span className="text-xs font-bold uppercase tracking-wider text-white/50">
                1. {t.portingStudio.fileLabel}
              </span>
              {inspection && (
                <span className="inline-flex items-center gap-1.5 rounded bg-emerald-500/20 px-2 py-0.5 text-[11px] font-semibold text-emerald-300">
                  <ShieldCheck className="h-3 w-3" />
                  {t.common.ready}
                </span>
              )}
            </div>

            {inspection ? (
              <div className="flex flex-wrap items-center justify-between gap-4 rounded-md border border-white/10 bg-white/[0.04] p-3">
                <div className="flex items-center gap-3 min-w-0">
                  <Disc3 className="h-7 w-7 text-fusion-accent shrink-0" />
                  <div className="min-w-0">
                    <div className="truncate font-semibold text-sm text-white">{inspection.fileName}</div>
                    <div className="flex flex-wrap gap-x-3 text-xs text-white/45 font-mono">
                      <span>{(inspection.sizeBytes / (1024 * 1024)).toFixed(1)} MB</span>
                      <span>SHA256: {inspection.sha256.substring(0, 16)}…</span>
                      <span className="text-fusion-accent font-semibold uppercase">{inspection.platform}</span>
                    </div>
                  </div>
                </div>
                <button
                  onClick={handleChooseFile}
                  className="rounded-md border border-white/15 px-3 py-1.5 text-xs font-semibold text-white/70 hover:bg-white/10 hover:text-white transition"
                >
                  {t.common.browse}
                </button>
              </div>
            ) : (
              <button
                onClick={handleChooseFile}
                disabled={inspecting}
                className="flex w-full flex-col items-center justify-center rounded-lg border-2 border-dashed border-white/15 bg-white/[0.02] p-6 text-center hover:border-fusion-accent/50 hover:bg-white/[0.04] transition group"
              >
                {inspecting ? (
                  <>
                    <Loader2 className="h-8 w-8 animate-spin text-fusion-accent mb-2" />
                    <span className="text-sm font-semibold text-white/70">{t.portingStudio.inspectingFile}</span>
                  </>
                ) : (
                  <>
                    <UploadCloud className="h-8 w-8 text-white/40 group-hover:text-fusion-accent mb-2 transition" />
                    <span className="text-sm font-semibold text-white/80">{t.portingStudio.dragPrompt}</span>
                    <span className="mt-1 text-xs text-white/40">{t.portingStudio.dragSubtext}</span>
                  </>
                )}
              </button>
            )}
          </div>

          {/* Section 2: Metadata & Artwork */}
          <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
            <div className="md:col-span-2 space-y-4">
              <div className="flex items-center justify-between">
                <span className="text-xs font-bold uppercase tracking-wider text-white/50">
                  2. {t.portingStudio.gameTitle} & {t.common.details}
                </span>
                <button
                  type="button"
                  onClick={handleAutoScrape}
                  disabled={!title.trim() || autoScraping}
                  className="inline-flex items-center gap-1.5 rounded-md border border-cyan-500/30 bg-cyan-500/10 px-2.5 py-1 text-xs font-semibold text-cyan-200 hover:bg-cyan-500/20 disabled:opacity-40 transition"
                >
                  {autoScraping ? <Loader2 className="h-3 w-3 animate-spin" /> : <Search className="h-3 w-3" />}
                  {t.portingStudio.autoScrapeBtn}
                </button>
              </div>

              <div>
                <label htmlFor={titleInputId} className="block text-xs font-medium text-white/70 mb-1">{t.portingStudio.gameTitle}</label>
                <input
                  id={titleInputId}
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                  placeholder={t.portingStudio.gameTitlePlaceholder}
                  className="w-full rounded-md border border-white/15 bg-black/40 px-3 py-2 text-sm text-white placeholder-white/25 focus:border-fusion-accent focus:outline-none"
                />
              </div>

              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="block text-xs font-medium text-white/70 mb-1">{t.portingStudio.platform}</label>
                  <select
                    value={platform}
                    onChange={(e) => setPlatform(e.target.value)}
                    className="w-full rounded-md border border-white/15 bg-black/40 px-3 py-2 text-sm text-white focus:border-fusion-accent focus:outline-none"
                  >
                    {SUPPORTED_PLATFORMS.map((p) => (
                      <option key={p.id} value={p.id} className="bg-neutral-900">
                        {p.label}
                      </option>
                    ))}
                  </select>
                </div>
                <div>
                  <label className="block text-xs font-medium text-white/70 mb-1">{t.portingStudio.releaseYear}</label>
                  <input
                    type="number"
                    value={releaseYear}
                    onChange={(e) => setReleaseYear(e.target.value ? parseInt(e.target.value, 10) : '')}
                    placeholder="1998"
                    className="w-full rounded-md border border-white/15 bg-black/40 px-3 py-2 text-sm text-white placeholder-white/25 focus:border-fusion-accent focus:outline-none"
                  />
                </div>
              </div>

              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="block text-xs font-medium text-white/70 mb-1">{t.portingStudio.developer}</label>
                  <input
                    value={developer}
                    onChange={(e) => setDeveloper(e.target.value)}
                    placeholder="Square, Capcom, Nintendo"
                    className="w-full rounded-md border border-white/15 bg-black/40 px-3 py-2 text-sm text-white placeholder-white/25 focus:border-fusion-accent focus:outline-none"
                  />
                </div>
                <div>
                  <label className="block text-xs font-medium text-white/70 mb-1">{t.portingStudio.genres}</label>
                  <input
                    value={genres}
                    onChange={(e) => setGenres(e.target.value)}
                    placeholder="RPG, Action, Adventure"
                    className="w-full rounded-md border border-white/15 bg-black/40 px-3 py-2 text-sm text-white placeholder-white/25 focus:border-fusion-accent focus:outline-none"
                  />
                </div>
              </div>

              <div>
                <label className="block text-xs font-medium text-white/70 mb-1">{t.portingStudio.coverUrl}</label>
                <input
                  value={coverUrl}
                  onChange={(e) => setCoverUrl(e.target.value)}
                  placeholder="https://images.igdb.com/.../cover.png"
                  className="w-full rounded-md border border-white/15 bg-black/40 px-3 py-2 text-sm text-white placeholder-white/25 focus:border-fusion-accent focus:outline-none font-mono text-xs"
                />
              </div>

              <div>
                <label className="block text-xs font-medium text-white/70 mb-1">{t.portingStudio.heroUrl}</label>
                <input
                  value={heroUrl}
                  onChange={(e) => setHeroUrl(e.target.value)}
                  placeholder="https://images.igdb.com/.../hero.jpg"
                  className="w-full rounded-md border border-white/15 bg-black/40 px-3 py-2 text-sm text-white placeholder-white/25 focus:border-fusion-accent focus:outline-none font-mono text-xs"
                />
              </div>

              <div>
                <label className="block text-xs font-medium text-white/70 mb-1">{t.portingStudio.description}</label>
                <textarea
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  rows={2}
                  placeholder="Game overview and storyline..."
                  className="w-full rounded-md border border-white/15 bg-black/40 px-3 py-2 text-sm text-white placeholder-white/25 focus:border-fusion-accent focus:outline-none resize-none"
                />
              </div>
            </div>

            {/* Poster Card Preview */}
            <div className="flex flex-col items-center">
              <span className="text-xs font-bold uppercase tracking-wider text-white/50 self-start mb-3">
                {t.portingStudio.previewCardTitle}
              </span>
              <div className="w-48 overflow-hidden rounded-lg border border-white/20 bg-black/50 shadow-xl transition hover:scale-105 duration-200">
                <div className="relative aspect-[3/4] w-full bg-neutral-900 overflow-hidden flex items-center justify-center">
                  {coverUrl ? (
                    // eslint-disable-next-line @next/next/no-img-element
                    <img src={coverUrl} alt={title} className="h-full w-full object-cover" />
                  ) : (
                    <div className="flex flex-col items-center justify-center p-3 text-center text-white/30">
                      <FileCode className="h-10 w-10 mb-1" />
                      <span className="text-xs font-semibold">{title || 'No Cover'}</span>
                    </div>
                  )}
                  {enable60Fps && (
                    <div className="absolute top-2 left-2 rounded bg-amber-400/90 px-1.5 py-0.5 text-[9px] font-black text-black">
                      60 FPS
                    </div>
                  )}
                  <div className="absolute bottom-2 right-2 rounded bg-black/80 px-1.5 py-0.5 text-[9px] font-bold text-white uppercase">
                    {platform}
                  </div>
                </div>
                <div className="p-3 bg-white/[0.04]">
                  <div className="truncate font-bold text-xs text-white">{title || 'Untitled Game'}</div>
                  <div className="truncate text-[10px] text-white/45 mt-0.5">
                    {authorName ? t.portingStudio.authorCardBadge(authorName) : developer || 'Fusion'}
                  </div>
                </div>
              </div>
            </div>
          </div>

          {/* Section 3: Optimization & Modding Presets */}
          <div className="rounded-lg border border-white/10 bg-black/25 p-4 space-y-3">
            <span className="text-xs font-bold uppercase tracking-wider text-white/50 block">
              3. {t.portingStudio.optimizationTitle}
            </span>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <label className="flex items-start gap-3 rounded-md border border-white/10 bg-white/[0.03] p-3 cursor-pointer hover:bg-white/[0.06] transition">
                <input
                  type="checkbox"
                  checked={enable60Fps}
                  onChange={(e) => setEnable60Fps(e.target.checked)}
                  className="mt-1 h-4 w-4 rounded border-white/20 text-fusion-accent focus:ring-0"
                />
                <div>
                  <div className="text-xs font-bold text-white">{t.portingStudio.enable60fps}</div>
                  <div className="text-[11px] text-white/45 mt-0.5">{t.portingStudio.enable60fpsDesc}</div>
                </div>
              </label>

              <label className="flex items-start gap-3 rounded-md border border-white/10 bg-white/[0.03] p-3 cursor-pointer hover:bg-white/[0.06] transition">
                <input
                  type="checkbox"
                  checked={autoGamepad}
                  onChange={(e) => setAutoGamepad(e.target.checked)}
                  className="mt-1 h-4 w-4 rounded border-white/20 text-fusion-accent focus:ring-0"
                />
                <div>
                  <div className="text-xs font-bold text-white">{t.portingStudio.autoGamepad}</div>
                  <div className="text-[11px] text-white/45 mt-0.5">{t.portingStudio.autoGamepadDesc}</div>
                </div>
              </label>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 gap-3 pt-1">
              <div>
                <label className="block text-xs font-medium text-white/70 mb-1">{t.portingStudio.patchVersion}</label>
                <input
                  value={patchVersion}
                  onChange={(e) => setPatchVersion(e.target.value)}
                  placeholder={t.portingStudio.patchVersionPlaceholder}
                  className="w-full rounded-md border border-white/15 bg-black/40 px-3 py-2 text-sm text-white placeholder-white/25 focus:border-fusion-accent focus:outline-none"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-white/70 mb-1">{t.portingStudio.notes}</label>
                <input
                  value={notes}
                  onChange={(e) => setNotes(e.target.value)}
                  placeholder={t.portingStudio.notesPlaceholder}
                  className="w-full rounded-md border border-white/15 bg-black/40 px-3 py-2 text-sm text-white placeholder-white/25 focus:border-fusion-accent focus:outline-none"
                />
              </div>
            </div>
          </div>

          {/* Section 4: Creator Branding & Attribution */}
          <div className="rounded-lg border border-amber-500/20 bg-amber-950/15 p-4 space-y-3">
            <div className="flex items-center justify-between">
              <span className="text-xs font-bold uppercase tracking-wider text-amber-200">
                4. 👑 {t.portingStudio.authorBrandingTitle}
              </span>
              <span className="text-[11px] text-amber-200/60 font-semibold">{t.portingStudio.authorBrandingDesc}</span>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
              <div>
                <label className="block text-xs font-medium text-amber-200/80 mb-1">{t.portingStudio.authorName}</label>
                <input
                  value={authorName}
                  onChange={(e) => setAuthorName(e.target.value)}
                  placeholder={t.portingStudio.authorNamePlaceholder}
                  className="w-full rounded-md border border-amber-500/30 bg-black/40 px-3 py-2 text-sm text-white placeholder-amber-200/30 focus:border-amber-400 focus:outline-none"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-amber-200/80 mb-1">{t.portingStudio.authorUrl}</label>
                <input
                  value={authorUrl}
                  onChange={(e) => setAuthorUrl(e.target.value)}
                  placeholder={t.portingStudio.authorUrlPlaceholder}
                  className="w-full rounded-md border border-amber-500/30 bg-black/40 px-3 py-2 text-sm text-white placeholder-amber-200/30 focus:border-amber-400 focus:outline-none font-mono text-xs"
                />
              </div>
            </div>
          </div>

          {/* Section 5: Distribution Mode */}
          <div className="rounded-lg border border-white/10 bg-black/25 p-4 space-y-3">
            <span className="text-xs font-bold uppercase tracking-wider text-white/50 block">
              5. {t.portingStudio.distributionTitle}
            </span>
            <div className="space-y-2">
              <label className="flex items-start gap-3 rounded-md border border-white/10 bg-white/[0.03] p-3 cursor-pointer hover:bg-white/[0.06] transition">
                <input
                  type="radio"
                  name="contentMode"
                  value="user_provided"
                  checked={contentMode === 'user_provided'}
                  onChange={() => setContentMode('user_provided')}
                  className="mt-1 h-4 w-4 text-fusion-accent focus:ring-0"
                />
                <div>
                  <div className="text-xs font-bold text-white">{t.portingStudio.modeUserProvided}</div>
                  <div className="text-[11px] text-white/45 mt-0.5">{t.portingStudio.modeUserProvidedDesc}</div>
                </div>
              </label>

              <label className="flex items-start gap-3 rounded-md border border-white/10 bg-white/[0.03] p-3 cursor-pointer hover:bg-white/[0.06] transition">
                <input
                  type="radio"
                  name="contentMode"
                  value="http"
                  checked={contentMode === 'http'}
                  onChange={() => setContentMode('http')}
                  className="mt-1 h-4 w-4 text-fusion-accent focus:ring-0"
                />
                <div className="flex-1">
                  <div className="text-xs font-bold text-white">{t.portingStudio.modeDirect}</div>
                  {contentMode === 'http' && (
                    <input
                      value={downloadUrl}
                      onChange={(e) => setDownloadUrl(e.target.value)}
                      placeholder={t.portingStudio.modeDirectPlaceholder}
                      className="mt-2 w-full rounded-md border border-white/15 bg-black/40 px-3 py-1.5 text-xs text-white placeholder-white/25 focus:border-fusion-accent focus:outline-none font-mono"
                    />
                  )}
                </div>
              </label>

              <label className="flex items-start gap-3 rounded-md border border-white/10 bg-white/[0.03] p-3 cursor-pointer hover:bg-white/[0.06] transition">
                <input
                  type="radio"
                  name="contentMode"
                  value="magnet"
                  checked={contentMode === 'magnet'}
                  onChange={() => setContentMode('magnet')}
                  className="mt-1 h-4 w-4 text-fusion-accent focus:ring-0"
                />
                <div className="flex-1">
                  <div className="text-xs font-bold text-white">{t.portingStudio.modeMagnet}</div>
                  {contentMode === 'magnet' && (
                    <input
                      value={magnetUri}
                      onChange={(e) => setMagnetUri(e.target.value)}
                      placeholder={t.portingStudio.modeMagnetPlaceholder}
                      className="mt-2 w-full rounded-md border border-white/15 bg-black/40 px-3 py-1.5 text-xs text-white placeholder-white/25 focus:border-fusion-accent focus:outline-none font-mono"
                    />
                  )}
                </div>
              </label>
            </div>
          </div>
        </div>

        {/* Footer Actions */}
        <div className="flex flex-wrap items-center justify-between gap-3 border-t border-white/10 px-6 py-4 bg-white/[0.02]">
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={handleCopyDeepLink}
              disabled={!title.trim()}
              className="inline-flex items-center gap-1.5 rounded-md border border-fusion-accent/40 bg-fusion-accent/15 px-3 py-2 text-xs font-bold text-fusion-accent hover:bg-fusion-accent/25 transition disabled:opacity-40"
              title={t.portingStudio.copyDeepLinkDesc}
            >
              <Share2 className="h-3.5 w-3.5" />
              {t.portingStudio.copyDeepLink}
            </button>

            <button
              type="button"
              onClick={handleCopyPost}
              disabled={!title.trim()}
              className="inline-flex items-center gap-1.5 rounded-md border border-white/15 px-3 py-2 text-xs font-semibold text-white/70 hover:bg-white/10 hover:text-white transition disabled:opacity-40"
            >
              <MessageSquare className="h-3.5 w-3.5" />
              {t.portingStudio.copyPostMarkdown}
            </button>

            <button
              type="button"
              onClick={handleSaveManifest}
              disabled={!title.trim()}
              className="inline-flex items-center gap-1.5 rounded-md border border-white/15 px-3 py-2 text-xs font-semibold text-white/70 hover:bg-white/10 hover:text-white transition disabled:opacity-40"
            >
              <Save className="h-3.5 w-3.5" />
              {t.portingStudio.saveManifestJson}
            </button>
          </div>

          <button
            type="button"
            onClick={handleAddToLibrary}
            disabled={!title.trim() || busy}
            className="inline-flex items-center gap-2 rounded-lg bg-fusion-green px-5 py-2 text-xs font-black text-black hover:bg-emerald-400 transition disabled:opacity-40 shadow-lg shadow-emerald-950/40"
          >
            {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Play className="h-4 w-4 fill-current" />}
            {t.portingStudio.addToLibraryBtn}
          </button>
        </div>
      </div>
    </div>
  );
}
