'use client';

import { useEffect, useState, useCallback } from 'react';
import { UploadCloud, CheckCircle2, AlertCircle, Loader2, Key, Cpu, Disc3, FileCode } from 'lucide-react';
import { api } from '@/lib/api';
import { useI18n } from '@/components/I18nProvider';
import { isTauriRuntime } from '@/lib/runtime';
import type { CatalogGame } from '@/types/repository';

interface GlobalDropZoneProps {
  catalog: CatalogGame[];
  selectedGameId: string | null;
  onRefresh: () => Promise<void>;
  onSelectGame: (gameId: string) => void;
  onActivityEvent: (event: { title: string; detail: string; tone: 'info' | 'success' | 'warning' | 'error' }) => void;
  onMessage: (message: string | null) => void;
  onOpenPortingStudio?: (filePath?: string) => void;
}

export function GlobalDropZone({
  catalog,
  selectedGameId,
  onRefresh,
  onSelectGame,
  onActivityEvent,
  onMessage,
  onOpenPortingStudio
}: GlobalDropZoneProps) {
  const { t } = useI18n();
  const [isDragging, setIsDragging] = useState(false);
  const [isProcessing, setIsProcessing] = useState(false);
  const [lastResult, setLastResult] = useState<{ success: boolean; text: string } | null>(null);

  const processDroppedPaths = useCallback(
    async (paths: string[]) => {
      if (!paths || paths.length === 0) return;
      setIsProcessing(true);
      setLastResult(null);

      let processedCount = 0;
      let lastSuccessLabel = '';

      try {
        for (const filePath of paths) {
          const trimmed = filePath.trim();
          if (!trimmed) continue;

          // 1. Check if it's a JSON source repository / manifest
          if (trimmed.toLowerCase().endsWith('.json')) {
            try {
              const preview = await api.previewRepository(trimmed);
              await api.connectRepository(trimmed);
              processedCount++;
              lastSuccessLabel = preview.name;
              onActivityEvent({
                title: t.dashboard.dropzone.sourceConnected(preview.name),
                detail: trimmed,
                tone: 'success'
              });
              continue;
            } catch {
              // Fallback to detection if it's not a valid repo manifest
            }
          }

          // 2. Run smart system file detector
          const detection = await api.detectSystemFile(trimmed);

          if (detection.detectedKind !== 'unknown' && detection.detectedKind !== 'game_file') {
            // It is Keys, Firmware, or BIOS -> auto install directly!
            const importReport = await api.autoImportSystemFile(trimmed);
            if (importReport.success) {
              processedCount++;
              lastSuccessLabel = importReport.label;
              onActivityEvent({
                title: t.dashboard.dropzone.installedSystem(importReport.label),
                detail: `${importReport.status}: ${importReport.installedPath ?? trimmed}`,
                tone: 'success'
              });
            } else {
              onActivityEvent({
                title: 'Import Failed',
                detail: importReport.message,
                tone: 'error'
              });
            }
            continue;
          }

          // 3. Game File handling
          if (detection.detectedKind === 'game_file' || isProbableGameFile(trimmed)) {
            const fileName = trimmed.split(/[\\/]/).pop() ?? trimmed;

            // Target 3a: If user has a game details modal open, import directly to this game
            if (selectedGameId) {
              const game = catalog.find((g) => g.id === selectedGameId);
              const report = await api.importGameFile(selectedGameId, trimmed);
              if (report.status !== 'error') {
                processedCount++;
                lastSuccessLabel = game?.title ?? selectedGameId;
                onActivityEvent({
                  title: t.dashboard.dropzone.installedGame(game?.title ?? selectedGameId),
                  detail: report.installedPath,
                  tone: 'success'
                });
              } else {
                onActivityEvent({
                  title: 'Game Import Error',
                  detail: `${report.errorCode}: ${fileName}`,
                  tone: 'error'
                });
              }
              continue;
            }

            // Target 3b: Search catalog for matching game title or ID
            const normalizedFileName = fileName.toLowerCase().replace(/\.[^.]+$/, '');
            const matchedGame = catalog.find((game) => {
              const title = game.title.toLowerCase();
              const id = game.id.toLowerCase();
              return (
                normalizedFileName.includes(id) ||
                normalizedFileName.includes(title) ||
                title.includes(normalizedFileName)
              );
            });

            if (matchedGame) {
              const report = await api.importGameFile(matchedGame.id, trimmed);
              if (report.status !== 'error') {
                processedCount++;
                lastSuccessLabel = matchedGame.title;
                onSelectGame(matchedGame.id);
                onActivityEvent({
                  title: t.dashboard.dropzone.installedGame(matchedGame.title),
                  detail: report.installedPath,
                  tone: 'success'
                });
              }
              continue;
            }

            // Target 3c: Game found, but no exact catalog match -> Open Porting Studio!
            if (onOpenPortingStudio) {
              onOpenPortingStudio(trimmed);
            } else {
              onMessage(t.dashboard.dropzone.gamePayloadNotice(fileName));
            }
            onActivityEvent({
              title: t.portingStudio.triggerButton,
              detail: fileName,
              tone: 'info'
            });
          }
        }

        if (processedCount > 0) {
          await onRefresh();
          setLastResult({
            success: true,
            text: lastSuccessLabel ? `Ready: ${lastSuccessLabel}` : `Processed ${processedCount} file(s)`
          });
        }
      } catch (error) {
        const errorText = error instanceof Error ? error.message : String(error);
        setLastResult({ success: false, text: errorText });
        onActivityEvent({
          title: 'Import Failed',
          detail: errorText,
          tone: 'error'
        });
      } finally {
        setIsProcessing(false);
        setTimeout(() => {
          setIsDragging(false);
          setLastResult(null);
        }, 1200);
      }
    },
    [catalog, selectedGameId, onRefresh, onSelectGame, onActivityEvent, onMessage, t]
  );

  // Tauri 2 native webview drag & drop
  useEffect(() => {
    if (!isTauriRuntime()) return;

    let unlisten: (() => void) | undefined;
    let cancelled = false;

    import('@tauri-apps/api/webview')
      .then(({ getCurrentWebview }) => {
        if (cancelled) return;
        return getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type === 'over' || event.payload.type === 'enter') {
            setIsDragging(true);
          } else if (event.payload.type === 'leave') {
            setIsDragging(false);
          } else if (event.payload.type === 'drop') {
            void processDroppedPaths(event.payload.paths);
          }
        });
      })
      .then((unlistener) => {
        if (cancelled) {
          unlistener?.();
        } else {
          unlisten = unlistener;
        }
      })
      .catch(() => {
        // Fallback or preview
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [processDroppedPaths]);

  // Standard HTML5 drag & drop fallback for browser / preview mode
  useEffect(() => {
    if (isTauriRuntime()) return;

    const handleDragOver = (e: DragEvent) => {
      e.preventDefault();
      setIsDragging(true);
    };

    const handleDragLeave = (e: DragEvent) => {
      if (e.relatedTarget === null) {
        setIsDragging(false);
      }
    };

    const handleDrop = (e: DragEvent) => {
      e.preventDefault();
      setIsDragging(false);
      const files = e.dataTransfer?.files;
      if (!files || files.length === 0) return;

      const paths: string[] = [];
      for (let i = 0; i < files.length; i++) {
        const file = files[i];
        // In electron/hybrid browsers, file.path exists
        const path = (file as unknown as { path?: string }).path ?? file.name;
        paths.push(path);
      }
      void processDroppedPaths(paths);
    };

    window.addEventListener('dragover', handleDragOver);
    window.addEventListener('dragleave', handleDragLeave);
    window.addEventListener('drop', handleDrop);

    return () => {
      window.removeEventListener('dragover', handleDragOver);
      window.removeEventListener('dragleave', handleDragLeave);
      window.removeEventListener('drop', handleDrop);
    };
  }, [processDroppedPaths]);

  if (!isDragging && !isProcessing) return null;

  return (
    <div
      data-testid="global-dropzone-overlay"
      className="fixed inset-0 z-[100] grid place-items-center bg-black/85 px-6 backdrop-blur-2xl transition-all duration-300"
    >
      <div className="relative flex max-w-xl flex-col items-center rounded-3xl border-2 border-dashed border-fusion-accent/70 bg-fusion-surface/90 p-8 text-center shadow-[0_0_90px_rgba(92,230,140,0.28)]">
        {/* Animated Icon Header */}
        <div className="relative mb-5 grid h-20 w-20 place-items-center rounded-2xl bg-fusion-accent/10 ring-1 ring-fusion-accent/30">
          {isProcessing ? (
            <Loader2 className="h-10 w-10 animate-spin text-fusion-accent" />
          ) : lastResult ? (
            lastResult.success ? (
              <CheckCircle2 className="h-10 w-10 text-fusion-accent" />
            ) : (
              <AlertCircle className="h-10 w-10 text-fusion-danger" />
            )
          ) : (
            <UploadCloud className="h-10 w-10 animate-bounce text-fusion-accent" />
          )}
        </div>

        <h3 className={`mb-2 text-2xl font-black tracking-tight text-white ${lastResult && !lastResult.success ? 'select-text break-words' : ''}`}>
          {isProcessing
            ? t.dashboard.dropzone.processing
            : lastResult
            ? lastResult.text
            : t.dashboard.dropzone.title}
        </h3>

        <p className="mb-6 max-w-md text-sm leading-relaxed text-white/65">
          {t.dashboard.dropzone.description}
        </p>

        {/* Feature badges showing zero-friction support */}
        <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
          <div className="flex items-center gap-2 rounded-xl border border-white/10 bg-white/[0.04] px-3 py-2 text-left">
            <Key className="h-4 w-4 shrink-0 text-fusion-accent" />
            <div>
              <div className="text-[11px] font-bold text-white">Keys</div>
              <div className="text-[9px] text-white/40">prod / title</div>
            </div>
          </div>
          <div className="flex items-center gap-2 rounded-xl border border-white/10 bg-white/[0.04] px-3 py-2 text-left">
            <Cpu className="h-4 w-4 shrink-0 text-fusion-teal" />
            <div>
              <div className="text-[11px] font-bold text-white">Firmware</div>
              <div className="text-[9px] text-white/40">ZIP / PUP</div>
            </div>
          </div>
          <div className="flex items-center gap-2 rounded-xl border border-white/10 bg-white/[0.04] px-3 py-2 text-left">
            <Disc3 className="h-4 w-4 shrink-0 text-fusion-warning" />
            <div>
              <div className="text-[11px] font-bold text-white">Games</div>
              <div className="text-[9px] text-white/40">NSP / ISO / 7Z</div>
            </div>
          </div>
          <div className="flex items-center gap-2 rounded-xl border border-white/10 bg-white/[0.04] px-3 py-2 text-left">
            <FileCode className="h-4 w-4 shrink-0 text-fusion-info" />
            <div>
              <div className="text-[11px] font-bold text-white">Sources</div>
              <div className="text-[9px] text-white/40">JSON manifest</div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function isProbableGameFile(path: string): boolean {
  const lower = path.toLowerCase();
  const gameExts = [
    '.nsp',
    '.xci',
    '.nsz',
    '.iso',
    '.chd',
    '.rvz',
    '.wbfs',
    '.cso',
    '.bin',
    '.cue',
    '.nes',
    '.sfc',
    '.smc',
    '.gba',
    '.gbc',
    '.gb',
    '.zip',
    '.7z'
  ];
  return gameExts.some((ext) => lower.endsWith(ext));
}
