import type { PlatformSetupProfile } from '../types/repository.ts';

export const PLATFORM_SETUP_PROFILES: PlatformSetupProfile[] = [
  {
    id: 'nes-mesen',
    platform: 'nes',
    displayName: 'NES / Mesen2',
    emulator: {
      installMode: 'downloadable',
      emulatorName: 'Mesen2',
      executableName: 'Mesen.exe',
      executableCandidates: ['Mesen.exe'],
      download: {
        url: 'https://github.com/SourMesen/Mesen2/releases/download/2.1.1/Mesen_2.1.1_Windows.zip',
        sha256: '23ccc2bc060b663c68dad3a8c5d6da7d23a50f872d04f135bafa2b04ff7d5cbe',
        version: '2.1.1'
      }
    },
    gameFiles: {
      expectedExtensions: ['.nes'],
      allowDirectory: false,
      preferredFilePatterns: [],
      validators: ['ines']
    },
    systemFiles: [],
    launch: { argsTemplate: '{game_path}', workingDirectory: 'emulator_dir' }
  },
  {
    id: 'snes-mesen',
    platform: 'snes',
    displayName: 'Super Nintendo / Mesen2',
    emulator: {
      installMode: 'downloadable',
      emulatorName: 'Mesen2',
      executableName: 'Mesen.exe',
      executableCandidates: ['Mesen.exe']
    },
    gameFiles: {
      expectedExtensions: ['.sfc', '.smc'],
      allowDirectory: false,
      preferredFilePatterns: [],
      validators: []
    },
    systemFiles: [],
    launch: { argsTemplate: '{game_path}', workingDirectory: 'emulator_dir' }
  },
  {
    id: 'n64-rmg',
    platform: 'n64',
    displayName: 'Nintendo 64 / RMG',
    emulator: {
      installMode: 'downloadable',
      emulatorName: 'RMG',
      executableName: 'RMG.exe',
      executableCandidates: ['RMG.exe']
    },
    gameFiles: {
      expectedExtensions: ['.z64', '.n64', '.v64'],
      allowDirectory: false,
      preferredFilePatterns: [],
      validators: []
    },
    systemFiles: [],
    launch: { argsTemplate: '{game_path}', workingDirectory: 'emulator_dir' }
  },
  {
    id: 'gba-mgba',
    platform: 'gba',
    displayName: 'Game Boy Advance / mGBA',
    emulator: {
      installMode: 'downloadable',
      emulatorName: 'mGBA',
      executableName: 'mGBA.exe',
      executableCandidates: ['mGBA.exe']
    },
    gameFiles: {
      expectedExtensions: ['.gba'],
      allowDirectory: false,
      preferredFilePatterns: [],
      validators: []
    },
    systemFiles: [],
    launch: { argsTemplate: '-f {game_path}', workingDirectory: 'emulator_dir' }
  },
  {
    id: 'ps2-pcsx2',
    platform: 'ps2',
    displayName: 'PlayStation 2 / PCSX2',
    emulator: {
      installMode: 'downloadable',
      emulatorName: 'PCSX2',
      executableName: 'pcsx2-qt.exe',
      executableCandidates: ['pcsx2-qt.exe']
    },
    gameFiles: {
      expectedExtensions: ['.iso', '.bin', '.img', '.chd'],
      allowDirectory: true,
      preferredFilePatterns: ['*.iso', '*.chd'],
      validators: []
    },
    systemFiles: [
      {
        id: 'ps2-bios',
        label: 'PlayStation 2 BIOS',
        assetKind: 'bios',
        required: true,
        extensions: ['.bin', '.rom'],
        targetName: 'bios/ps2-bios.bin',
        sourceMode: 'user_provided',
        notes: 'Импортируй BIOS-образ, снятый с твоего собственного устройства.'
      }
    ],
    launch: { argsTemplate: '-fullscreen -- {game_path}', workingDirectory: 'emulator_dir' }
  },
  {
    id: 'psp-ppsspp',
    platform: 'psp',
    displayName: 'PSP / PPSSPP',
    emulator: {
      installMode: 'downloadable',
      emulatorName: 'PPSSPP',
      executableName: 'PPSSPPWindows64.exe',
      executableCandidates: ['PPSSPPWindows64.exe', 'PPSSPPWindows.exe']
    },
    gameFiles: {
      expectedExtensions: ['.iso', '.cso', '.pbp'],
      allowDirectory: false,
      preferredFilePatterns: [],
      validators: []
    },
    systemFiles: [],
    launch: { argsTemplate: '--fullscreen {game_path}', workingDirectory: 'emulator_dir' }
  },
  {
    id: 'ps1-manual',
    platform: 'ps1',
    displayName: 'PlayStation 1 / DuckStation',
    emulator: {
      installMode: 'manual',
      emulatorName: 'DuckStation',
      executableName: 'duckstation-qt-x64-ReleaseLTCG.exe',
      executableCandidates: ['duckstation-qt-x64-ReleaseLTCG.exe', 'duckstation.exe']
    },
    gameFiles: {
      expectedExtensions: ['.cue', '.bin', '.iso', '.img', '.pbp', '.chd'],
      allowDirectory: true,
      preferredFilePatterns: ['*.cue', '*.chd'],
      validators: []
    },
    systemFiles: [
      {
        id: 'ps1-bios',
        label: 'PlayStation BIOS',
        assetKind: 'bios',
        required: true,
        extensions: ['.bin', '.rom'],
        targetName: 'bios/scph5501.bin',
        sourceMode: 'user_provided',
        notes: 'Импортируй BIOS-образ, снятый с твоего собственного устройства.'
      }
    ],
    launch: { argsTemplate: '-batch "{game_path}"', workingDirectory: 'emulator_dir' }
  },
  {
    id: 'switch-manual',
    platform: 'switch',
    displayName: 'Nintendo Switch / ручная настройка',
    emulator: {
      installMode: 'manual',
      emulatorName: 'Switch-эмулятор',
      executableName: 'eden.exe',
      executableCandidates: ['eden.exe', 'eden-cli.exe', 'Ryujinx.exe', 'suyu.exe']
    },
    gameFiles: {
      expectedExtensions: ['.nsp', '.xci', '.nca'],
      allowDirectory: true,
      preferredFilePatterns: ['*.nsp', '*.xci'],
      validators: []
    },
    systemFiles: [
      {
        id: 'switch-prod-keys',
        label: 'Switch prod.keys',
        assetKind: 'keys',
        required: true,
        extensions: ['.keys'],
        targetName: 'prod.keys',
        sourceMode: 'user_provided',
        notes: 'Импортируй prod.keys из своей легально принадлежащей консоли.'
      },
      {
        id: 'switch-firmware',
        label: 'Switch firmware',
        assetKind: 'firmware',
        required: false,
        extensions: ['.zip'],
        targetName: 'firmware.zip',
        sourceMode: 'user_provided',
        notes: 'Опциональный firmware-пакет, который пользователь добавляет сам.'
      }
    ],
    launch: { argsTemplate: '{game_path}', workingDirectory: 'emulator_dir' }
  },
  {
    id: 'snes-manual',
    platform: 'snes',
    displayName: 'Super Nintendo / ручная настройка',
    emulator: {
      installMode: 'manual',
      emulatorName: 'SNES-эмулятор',
      executableName: 'bsnes.exe',
      executableCandidates: ['bsnes.exe', 'snes9x.exe']
    },
    gameFiles: {
      expectedExtensions: ['.sfc', '.smc'],
      allowDirectory: false,
      preferredFilePatterns: [],
      validators: []
    },
    systemFiles: [],
    launch: { argsTemplate: '{game_path}', workingDirectory: 'emulator_dir' }
  },
  {
    id: 'ps2-manual',
    platform: 'ps2',
    displayName: 'PlayStation 2 / PCSX2',
    emulator: {
      installMode: 'manual',
      emulatorName: 'PCSX2',
      executableName: 'pcsx2-qt.exe',
      executableCandidates: ['pcsx2-qt.exe']
    },
    gameFiles: {
      expectedExtensions: ['.iso', '.bin', '.img', '.chd'],
      allowDirectory: true,
      preferredFilePatterns: ['*.iso', '*.chd'],
      validators: []
    },
    systemFiles: [
      {
        id: 'ps2-bios',
        label: 'PlayStation 2 BIOS',
        assetKind: 'bios',
        required: true,
        extensions: ['.bin', '.rom'],
        targetName: 'bios/ps2-bios.bin',
        sourceMode: 'user_provided',
        notes: 'Import a BIOS image dumped from hardware you own.'
      }
    ],
    launch: { argsTemplate: '-fullscreen -- "{game_path}"', workingDirectory: 'emulator_dir' }
  },
  {
    id: 'psp-manual',
    platform: 'psp',
    displayName: 'PSP / Manual Emulator',
    emulator: {
      installMode: 'manual',
      emulatorName: 'PPSSPP',
      executableName: 'PPSSPPWindows64.exe',
      executableCandidates: ['PPSSPPWindows64.exe', 'PPSSPPWindows.exe']
    },
    gameFiles: {
      expectedExtensions: ['.iso', '.cso', '.pbp'],
      allowDirectory: false,
      preferredFilePatterns: [],
      validators: []
    },
    systemFiles: [],
    launch: { argsTemplate: '{game_path}', workingDirectory: 'emulator_dir' }
  }
];

export function getPlatformSetupProfile(profileId: string | null | undefined) {
  if (!profileId) return null;
  return PLATFORM_SETUP_PROFILES.find((profile) => profile.id === profileId) ?? null;
}

export function getDefaultPlatformSetupProfile(platform: string) {
  const profileId = {
    nes: 'nes-mesen',
    snes: 'snes-mesen',
    n64: 'n64-rmg',
    gba: 'gba-mgba',
    ps2: 'ps2-pcsx2',
    psp: 'psp-ppsspp',
    ps1: 'ps1-manual',
    switch: 'switch-manual'
  }[platform];

  return getPlatformSetupProfile(profileId);
}
