import type {
  CatalogGame,
  DiagnosticsBundle,
  DownloadRecord,
  EmulatorConfig,
  GameDownloadStartReport,
  HealthReport,
  LaunchReport,
  LibraryGameStatus,
  OnboardingState,
  RecommendedEmulator,
  RepositoryPreview,
  RepositorySummary,
  RequirementsReport,
  TorrentDownloadRecord,
  TorrentStartReport,
  TorrentStatus,
  TrustedExecutable,
  UpdateCheckReport
} from '../types/repository.ts';

const now = '2026-05-26T08:00:00.000Z';

const repository: RepositorySummary = {
  id: 'retrohydra-preview',
  name: 'RetroHydra Preview Repository',
  version: '1.0.0',
  url: 'preview://retrohydra',
  connectedAt: now,
  catalogCount: 12,
  systemFileCount: 3,
  maintainer: 'RetroHydra Team',
  homepageUrl: 'https://retrohydra.local',
  license: 'Mixed legal homebrew/demo content',
  trustLevel: 'official',
  contentHash: '0'.repeat(64),
  lastRefreshedAt: now,
  hasExecutableAssets: false
};

const catalog: CatalogGame[] = [
  game(
    'retrohydra_nes_smoke',
    'nes',
    'RetroHydra NES Smoke Demo',
    'First-party NES smoke demo for the one-click setup path.',
    ['.nes'],
    [{ kind: 'bundled', path: 'demo-content/retrohydra-smoke.nes', sha256: '9'.repeat(64), sizeBytes: 24592 }]
  ),
  game('crystal-caverns', 'gba', 'Crystal Caverns DX', 'A fast homebrew platformer tuned for short sessions.', ['.gba']),
  game('neon-rally', 'psp', 'Neon Rally Portable', 'Arcade racing with synthetic night tracks and drift challenges.', ['.iso']),
  game('star-orbit', 'switch', 'Star Orbit Prototype', 'A community tech demo for testing modern handheld workflows.', ['.nsp', '.xci']),
  game('midnight-pinball', 'ps1', 'Midnight Pinball Club', 'Table physics, neon bumpers, and quick-score runs.', ['.cue', '.bin']),
  game('skyline-runner', 'dreamcast', 'Skyline Runner', 'A compact futuristic runner for launcher smoke tests.', ['.gdi', '.cdi']),
  game('forest-quest', 'snes', 'Forest Quest Recut', '16-bit adventure pacing with a warm pixel palette.', ['.sfc']),
  game('byte-brawlers', 'n64', 'Byte Brawlers Arena', 'Local arena chaos with chunky prototype characters.', ['.z64']),
  game('lunar-keys', 'nds', 'Lunar Keys', 'Puzzle rooms and touch-friendly menu checks.', ['.nds']),
  game('signal-echo', 'ps2', 'Signal Echo Trial', 'Atmospheric test package for large-disc workflows.', ['.iso']),
  game('terra-pico', 'nes', 'Terra Pico', 'Minimal mapper-friendly demo entry.', ['.nes']),
  game('orbit-garden', 'gamecube', 'Orbit Garden', 'A tiny world tour used for metadata rows.', ['.rvz', '.iso']),
  game('copper-line', 'genesis', 'Copper Line', 'A bright scrolling test ROM with punchy status art.', ['.md'])
];

let downloads: TorrentDownloadRecord[] = [
  torrent('crystal-caverns', 'completed', 100, 46_000_000, 46_000_000, 0, 0, null),
  torrent('neon-rally', 'downloading', 68, 2_400_000_000, 3_500_000_000, 12_000_000, 7, null),
  torrent('star-orbit', 'paused', 42, 3_100_000_000, 7_200_000_000, 0, 4, null),
  torrent('signal-echo', 'error', 18, 1_600_000_000, 8_900_000_000, 0, 0, 'No peers were found for this magnet.')
];

let emulatorConfigs: EmulatorConfig[] = [];
let downloadRoot = 'preview://games';

export const previewApi = {
  async previewRepository(url = repository.url): Promise<RepositoryPreview> {
    return {
      url,
      id: repository.id,
      name: repository.name,
      version: repository.version,
      maintainer: repository.maintainer,
      homepageUrl: repository.homepageUrl,
      license: repository.license,
      trustLevel: repository.trustLevel,
      catalogCount: repository.catalogCount,
      systemFileCount: repository.systemFileCount,
      hasExecutableAssets: repository.hasExecutableAssets,
      contentHash: repository.contentHash ?? '0'.repeat(64)
    };
  },
  async previewBuiltInDemoRepository(): Promise<RepositoryPreview> {
    return previewApi.previewRepository('retrohydra://builtin/demo-repository.json');
  },
  async connectRepository(_url = repository.url): Promise<RepositorySummary> {
    return repository;
  },
  async connectBuiltInDemoRepository(): Promise<RepositorySummary> {
    return {
      ...repository,
      url: 'retrohydra://builtin/demo-repository.json'
    };
  },
  async refreshRepository(_repositoryId = repository.id): Promise<RepositorySummary> {
    return { ...repository, lastRefreshedAt: new Date().toISOString() };
  },
  async getOnboardingState(): Promise<OnboardingState> {
    return {
      step: emulatorConfigs.length > 0 ? 'complete' : 'configureEmulator',
      repositoriesConfigured: true,
      emulatorsConfigured: emulatorConfigs.length > 0,
      catalogCount: catalog.length,
      validEmulatorCount: emulatorConfigs.filter((config) => config.status === 'valid').length
    };
  },
  async listRepositories(): Promise<RepositorySummary[]> {
    return [repository];
  },
  async disconnectRepository(_repositoryId = repository.id): Promise<boolean> {
    return true;
  },
  async getCatalog(): Promise<CatalogGame[]> {
    return catalog;
  },
  async getGame(gameId: string): Promise<CatalogGame | null> {
    return catalog.find((item) => item.id === gameId) ?? null;
  },
  async checkRequirements(gameId: string): Promise<RequirementsReport> {
    const status = await previewApi.getLibraryStatuses();
    const gameStatus = status.find((item) => item.gameId === gameId);
    return {
      gameId,
      ready: Boolean(gameStatus?.installed && gameStatus.systemRequirementsReady),
      gameDownloaded: Boolean(gameStatus?.installed),
      requirements: []
    };
  },
  async getLibraryStatuses(): Promise<LibraryGameStatus[]> {
    return catalog.map((item, index) => {
      const download = downloads.find((record) => record.gameId === item.id) ?? null;
      const installed = download?.status === 'completed';
      const missingRequirements = installed && index % 5 === 0 ? [`Configure ${item.platform.toUpperCase()} emulator`] : [];

      return {
        gameId: item.id,
        installed,
        systemRequirementsReady: missingRequirements.length === 0,
        missingRequirements,
        download
      };
    });
  },
  async listEmulatorConfigs(): Promise<EmulatorConfig[]> {
    return emulatorConfigs;
  },
  async getRecommendedEmulators(): Promise<RecommendedEmulator[]> {
    return ['switch', 'ps1', 'ps2', 'gba', 'nes'].map((platform) => {
      const config = emulatorConfigs.find((item) => item.platform === platform);
      const nes = platform === 'nes';
      return {
        platform: platform as RecommendedEmulator['platform'],
        platformLabel: platform === 'nes' ? 'NES / Famicom' : platform.toUpperCase(),
        emulatorName: nes ? 'Mesen2' : `${platform.toUpperCase()} emulator`,
        version: nes ? '2.1.1' : null,
        downloadUrl: nes ? 'https://github.com/SourMesen/Mesen2/releases/download/2.1.1/Mesen_2.1.1_Windows.zip' : null,
        sha256: nes ? '23ccc2bc060b663c68dad3a8c5d6da7d23a50f872d04f135bafa2b04ff7d5cbe' : null,
        executableName: nes ? 'Mesen.exe' : `${platform}.exe`,
        status: config ? 'installed' : nes ? 'available' : 'manual',
        installedPath: config?.exePath ?? null,
        message: config ? 'Configured' : nes ? 'Available for automatic setup' : 'Manual setup required'
      };
    });
  },
  async installRecommendedEmulator(platform: string): Promise<EmulatorConfig> {
    if (platform !== 'nes') throw new Error('Automatic setup is available only for NES / Mesen2.');
    return previewApi.saveEmulatorConfig('nes', 'preview://emulators/Mesen2-2.1.1/Mesen.exe', '{game_path}');
  },
  async saveEmulatorConfig(
    platform: string,
    exePath: string,
    launchArgsTemplate?: string
  ): Promise<EmulatorConfig> {
    const config: EmulatorConfig = {
      platform: platform as EmulatorConfig['platform'],
      exePath,
      status: exePath.trim() ? 'valid' : 'invalid',
      lastValidatedAt: new Date().toISOString(),
      launchArgsTemplate
    };
    emulatorConfigs = [config, ...emulatorConfigs.filter((item) => item.platform !== platform)];
    return config;
  },
  async validateEmulatorConfig(platform: string): Promise<EmulatorConfig> {
    const config = emulatorConfigs.find((item) => item.platform === platform);
    if (!config) throw new Error(`No emulator config is stored for ${platform}`);
    return {
      ...config,
      lastValidatedAt: new Date().toISOString()
    };
  },
  async deleteEmulatorConfig(platform: string): Promise<boolean> {
    const before = emulatorConfigs.length;
    emulatorConfigs = emulatorConfigs.filter((item) => item.platform !== platform);
    return emulatorConfigs.length !== before;
  },
  async downloadAsset(assetId: string): Promise<DownloadRecord> {
    return downloadRecord(assetId, 'asset');
  },
  async downloadGame(gameId: string): Promise<DownloadRecord> {
    return downloadRecord(gameId, 'game');
  },
  async startGameDownload(gameId: string): Promise<GameDownloadStartReport> {
    const game = catalog.find((item) => item.id === gameId);
    const source = game?.downloads[0];
    if (source?.kind === 'http' || source?.kind === 'bundled') {
      const record = downloadRecord(gameId, 'game');
      const torrentRecord = directDownloadRecord(
        gameId,
        source.kind,
        record.localPath ?? `${downloadRoot}/${gameId}`,
        source.sizeBytes ?? 24_592
      );
      downloads = [torrentRecord, ...downloads.filter((item) => item.gameId !== gameId)];
      return { gameId, sourceKind: source.kind, saveDir: torrentRecord.saveDir, record, torrent: torrentRecord };
    }
    const torrent = await previewApi.startMagnetDownload(
      gameId,
      source?.kind === 'magnet' ? source.uri : `magnet:?xt=urn:btih:${gameId.replaceAll('-', '')}`,
      `${downloadRoot}/${gameId}`
    );
    const record = downloads.find((item) => item.gameId === gameId) ?? null;
    return { gameId, sourceKind: 'magnet', saveDir: torrent.saveDir, record: null, torrent: record };
  },
  async trustExecutable(assetId: string): Promise<TrustedExecutable> {
    return {
      assetId,
      localPath: `preview://system/${assetId}`,
      sha256: '0'.repeat(64),
      trustedAt: new Date().toISOString()
    };
  },
  async getDownloadRoot(): Promise<string> {
    return downloadRoot;
  },
  async setDownloadRoot(path: string): Promise<string> {
    downloadRoot = path || downloadRoot;
    return downloadRoot;
  },
  async removeGame(gameId: string, _deleteFiles: boolean): Promise<boolean> {
    const before = downloads.length;
    downloads = downloads.filter((item) => item.gameId !== gameId);
    return downloads.length !== before;
  },
  async redownloadAsset(assetId: string): Promise<DownloadRecord> {
    return downloadRecord(assetId, 'asset');
  },
  async openGameFolder(_gameId: string): Promise<void> {},
  async openEmulatorFolder(_platform: string): Promise<void> {},
  async openLogsFolder(): Promise<void> {},
  async runHealthCheck(): Promise<HealthReport> {
    return previewHealthReport();
  },
  async getDiagnosticsBundle(): Promise<DiagnosticsBundle> {
    return {
      generatedAt: new Date().toISOString(),
      appVersion: '0.1.0-preview',
      os: 'preview browser',
      dataDir: 'preview://data',
      logPath: 'preview://logs/retrohydra.log',
      health: previewHealthReport(),
      downloads,
      logs: ['{"event":"preview"}']
    };
  },
  async startMagnetDownload(gameId: string, magnetUri: string, saveDir: string): Promise<TorrentStartReport> {
    downloads = [
      {
        gameId,
        magnetUri,
        saveDir,
        status: 'downloading',
        progressPercent: 2,
        downloadedBytes: 2_000_000,
        totalBytes: 100_000_000,
        downloadSpeedBytesPerSec: 2_400_000,
        uploadSpeedBytesPerSec: 120_000,
        peersCount: 3,
        torrentId: 99,
        errorMessage: null,
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
        completedAt: null
      },
      ...downloads.filter((record) => record.gameId !== gameId)
    ];

    return { gameId, state: 'downloading', saveDir };
  },
  async getTorrentStatus(gameId: string): Promise<TorrentStatus> {
    const record = downloads.find((item) => item.gameId === gameId);
    if (!record) throw new Error(`Unknown preview torrent: ${gameId}`);
    return {
      gameId,
      state: record.status,
      progress: record.progressPercent / 100,
      downloadedBytes: record.downloadedBytes,
      totalBytes: record.totalBytes,
      downloadSpeedBytesPerSec: record.downloadSpeedBytesPerSec,
      uploadSpeedBytesPerSec: record.uploadSpeedBytesPerSec,
      peersCount: record.peersCount,
      finished: record.status === 'completed',
      saveDir: record.saveDir,
      error: record.errorMessage
    };
  },
  async getGameDownload(gameId: string): Promise<TorrentDownloadRecord | null> {
    return downloads.find((item) => item.gameId === gameId) ?? null;
  },
  async listTorrentDownloads(): Promise<TorrentDownloadRecord[]> {
    return downloads;
  },
  async pauseDownload(gameId: string): Promise<TorrentDownloadRecord> {
    return updateTorrentStatus(gameId, 'paused');
  },
  async resumeDownload(gameId: string): Promise<TorrentDownloadRecord> {
    return updateTorrentStatus(gameId, 'downloading', null, 4_800_000);
  },
  async cancelDownload(gameId: string): Promise<TorrentDownloadRecord> {
    return updateTorrentStatus(gameId, 'cancelled');
  },
  async launchGame(gameId: string): Promise<LaunchReport> {
    const game = catalog.find((item) => item.id === gameId);
    if (!game) throw new Error(`Unknown preview game: ${gameId}`);
    const emulator = emulatorConfigs.find((item) => item.platform === game.platform);
    const gamePath = downloads.find((item) => item.gameId === gameId)?.saveDir ?? `preview://games/${gameId}`;
    return {
      pid: 1,
      executable: emulator?.exePath ?? `preview://emulators/${game.platform}`,
      gamePath,
      resolvedGamePath: gamePath,
      args: [emulator?.launchArgsTemplate ?? '{game_path}', game.expectedExtensions.join(',')]
    };
  },
  async checkAppUpdate(): Promise<UpdateCheckReport> {
    return {
      available: false,
      currentVersion: '0.1.0-preview',
      version: null,
      date: null,
      body: null
    };
  },
  async installAppUpdate(): Promise<void> {
    return undefined;
  }
};

function game(
  id: string,
  platform: CatalogGame['platform'],
  title: string,
  description: string,
  expectedExtensions: string[],
  downloads?: CatalogGame['downloads']
): CatalogGame {
  return {
    id,
    sourceId: id,
    repositoryId: repository.id,
    repositoryName: repository.name,
    platform,
    title,
    description,
    downloads: downloads ?? [{ kind: 'magnet', uri: `magnet:?xt=urn:btih:${id.replaceAll('-', '')}` }],
    expectedExtensions,
    requiredSystemFileIds: []
  };
}

function torrent(
  gameId: string,
  status: TorrentDownloadRecord['status'],
  progressPercent: number,
  downloadedBytes: number,
  totalBytes: number,
  downloadSpeedBytesPerSec: number,
  peersCount: number,
  errorMessage: string | null
): TorrentDownloadRecord {
  return {
    gameId,
    magnetUri: `magnet:?xt=urn:btih:${gameId.replaceAll('-', '')}`,
    saveDir: `preview://games/${gameId}`,
    status,
    progressPercent,
    downloadedBytes,
    totalBytes,
    downloadSpeedBytesPerSec,
    uploadSpeedBytesPerSec: Math.floor(downloadSpeedBytesPerSec / 20),
    peersCount,
    torrentId: 1,
    errorMessage,
    createdAt: now,
    updatedAt: now,
    completedAt: status === 'completed' ? now : null
  };
}

function directDownloadRecord(
  gameId: string,
  sourceKind: 'http' | 'bundled',
  saveDir: string,
  totalBytes: number
): TorrentDownloadRecord {
  const timestamp = new Date().toISOString();
  return {
    gameId,
    magnetUri: `direct:${sourceKind}`,
    saveDir,
    status: 'completed',
    progressPercent: 100,
    downloadedBytes: totalBytes,
    totalBytes,
    downloadSpeedBytesPerSec: 0,
    uploadSpeedBytesPerSec: 0,
    peersCount: 0,
    torrentId: null,
    errorMessage: null,
    createdAt: timestamp,
    updatedAt: timestamp,
    completedAt: timestamp
  };
}

function downloadRecord(subjectId: string, subjectType: DownloadRecord['subjectType']): DownloadRecord {
  return {
    subjectId,
    subjectType,
    status: 'ready',
    localPath: `preview://${subjectType}/${subjectId}`,
    sha256: '0'.repeat(64),
    updatedAt: new Date().toISOString()
  };
}

function updateTorrentStatus(
  gameId: string,
  status: TorrentDownloadRecord['status'],
  errorMessage: string | null = null,
  downloadSpeedBytesPerSec = 0
) {
  const record = downloads.find((item) => item.gameId === gameId);
  if (!record) throw new Error(`Unknown preview torrent: ${gameId}`);
  const updated = {
    ...record,
    status,
    errorMessage,
    downloadSpeedBytesPerSec,
    updatedAt: new Date().toISOString()
  };
  downloads = downloads.map((item) => item.gameId === gameId ? updated : item);
  return updated;
}

function previewHealthReport(): HealthReport {
  return {
    generatedAt: new Date().toISOString(),
    emulators: ['switch', 'ps1', 'ps2', 'gba', 'nes'].map((platform) => {
      const config = emulatorConfigs.find((item) => item.platform === platform);
      return {
        id: `emulator:${platform}`,
        label: `${platform.toUpperCase()} emulator`,
        status: config ? 'ready' : 'missing',
        message: config?.exePath ?? 'Not configured',
        action: config ? 'openEmulatorFolder' : 'reconfigureEmulator',
        path: config?.exePath
      };
    }),
    systemFiles: [
      {
        id: 'asset:ps1-bios',
        label: 'PS1 BIOS',
        status: 'missing',
        message: 'User-provided BIOS is not present.',
        action: 'openTargetFolder'
      }
    ],
    gameFiles: downloads.map((download) => ({
      id: `game:${download.gameId}`,
      label: download.gameId,
      status: download.status === 'completed' ? 'ready' : download.status,
      message: download.saveDir,
      action: 'openGameFolder',
      path: download.saveDir
    })),
    repositories: [
      {
        id: `repository:${repository.id}`,
        label: repository.name,
        status: 'ready',
        message: repository.url,
        action: 'refreshRepository',
        path: repository.url
      }
    ],
    downloader: {
      id: 'downloader:preview',
      label: 'Downloader session',
      status: 'ready',
      message: `${downloads.length} preview records`
    }
  };
}
