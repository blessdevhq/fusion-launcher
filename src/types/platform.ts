export const PLATFORMS = [
  'switch',
  'nes',
  'snes',
  'n64',
  'gba',
  'nds',
  'gamecube',
  'wii',
  'ps1',
  'ps2',
  'ps3',
  'psp',
  'genesis',
  'saturn',
  'dreamcast'
] as const;

export type Platform = (typeof PLATFORMS)[number];

export const MVP_PLATFORMS = ['switch', 'ps1', 'ps2', 'gba', 'nes'] as const satisfies readonly Platform[];
export type MvpPlatform = (typeof MVP_PLATFORMS)[number];

export const EMULATOR_MANAGER_PLATFORMS = ['nes', 'snes', 'n64', 'gba', 'ps2', 'psp', 'ps1', 'switch'] as const satisfies readonly Platform[];
export type EmulatorManagerPlatform = (typeof EMULATOR_MANAGER_PLATFORMS)[number];

export const PLATFORM_LABELS: Record<Platform, string> = {
  switch: 'Nintendo Switch',
  nes: 'NES / Famicom',
  snes: 'Super Nintendo',
  n64: 'Nintendo 64',
  gba: 'Game Boy Advance',
  nds: 'Nintendo DS',
  gamecube: 'GameCube',
  wii: 'Wii',
  ps1: 'PlayStation 1',
  ps2: 'PlayStation 2',
  ps3: 'PlayStation 3',
  psp: 'PSP',
  genesis: 'Sega Genesis / Mega Drive',
  saturn: 'Sega Saturn',
  dreamcast: 'Sega Dreamcast'
};

export const PLATFORM_DEFAULT_EXTENSIONS: Record<Platform, string[]> = {
  switch: ['.nsp', '.xci', '.nca'],
  nes: ['.nes'],
  snes: ['.sfc', '.smc'],
  n64: ['.z64', '.n64', '.v64'],
  gba: ['.gba'],
  nds: ['.nds'],
  gamecube: ['.rvz', '.iso', '.gcm'],
  wii: ['.rvz', '.wbfs', '.iso'],
  ps1: ['.cue', '.bin', '.iso', '.img', '.pbp'],
  ps2: ['.iso', '.bin', '.img'],
  ps3: ['.pkg', '.iso'],
  psp: ['.iso', '.cso', '.pbp'],
  genesis: ['.md', '.bin', '.gen', '.smd'],
  saturn: ['.cue', '.bin', '.iso', '.chd'],
  dreamcast: ['.gdi', '.cdi', '.chd', '.iso']
};

export const PLATFORM_EMULATOR_HINTS: Record<MvpPlatform, string> = {
  switch: 'eden.exe',
  ps1: 'duckstation-qt-x64-ReleaseLTCG.exe',
  ps2: 'pcsx2-qt.exe',
  gba: 'mGBA.exe',
  nes: 'Mesen.exe'
};

export const PLATFORM_DEFAULT_LAUNCH_ARGS: Record<MvpPlatform, string> = {
  switch: '{game_path}',
  ps1: '-batch "{game_path}"',
  ps2: '-fullscreen -- "{game_path}"',
  gba: '-f {game_path}',
  nes: '{game_path}'
};

export function isMvpPlatform(platform: string): platform is MvpPlatform {
  return (MVP_PLATFORMS as readonly string[]).includes(platform);
}
