import type { CatalogGame } from '../types/repository.ts';

const PALETTES = [
  ['#0A0A0B', '#2f6f45', '#5CE68C'],
  ['#111315', '#2f6b62', '#3BD6C6'],
  ['#141416', '#3f5f48', '#a1a1a6'],
  ['#121418', '#46516a', '#5CE68C'],
  ['#101512', '#384a3e', '#f5f5f7'],
  ['#0f1110', '#1f7a56', '#ffb84b']
] as const;

export interface GeneratedGameArt {
  palette: readonly [string, string, string];
  posterStyle: Record<string, string>;
  heroStyle: Record<string, string>;
  initials: string;
}

export function createGameArt(game: Pick<CatalogGame, 'id' | 'title' | 'platform'>): GeneratedGameArt {
  const seed = hashString(`${game.id}:${game.platform}:${game.title}`);
  const palette = PALETTES[seed % PALETTES.length];
  const angle = 120 + (seed % 80);
  const offset = 18 + (seed % 46);
  const initials = game.title
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 3)
    .map((word) => word[0]?.toUpperCase())
    .join('') || game.platform.slice(0, 3).toUpperCase();

  const posterPattern = [
    `linear-gradient(${angle}deg, ${palette[1]}cc 0%, transparent 38%)`,
    `linear-gradient(${angle + 62}deg, transparent 12%, ${palette[2]}88 48%, transparent 76%)`,
    `linear-gradient(180deg, ${palette[0]}, #0A0A0B 88%)`
  ].join(', ');

  const heroPattern = [
    `linear-gradient(96deg, #0A0A0B 0%, ${palette[0]} 39%, transparent 66%)`,
    `linear-gradient(24deg, transparent 30%, ${palette[1]}66 58%, ${palette[2]}33 86%)`,
    `linear-gradient(180deg, #141416 0%, #0A0A0B 100%)`
  ].join(', ');

  return {
    palette,
    initials,
    posterStyle: {
      backgroundImage: posterPattern
    },
    heroStyle: {
      backgroundImage: heroPattern
    }
  };
}

export function selectGameArtImageUrl(
  game: Pick<CatalogGame, 'artwork' | 'coverImageUrl'>,
  mode: 'poster' | 'hero'
): string | null {
  const cover = firstUsableImageUrl(game.artwork?.cover, game.coverImageUrl);
  if (mode === 'hero') {
    return firstUsableImageUrl(game.artwork?.hero, cover);
  }
  return firstUsableImageUrl(cover, game.artwork?.hero);
}

function firstUsableImageUrl(...urls: Array<string | null | undefined>) {
  return urls.find((url) => url && !url.includes('...')) ?? null;
}

export function hashString(input: string): number {
  let hash = 2166136261;
  for (let index = 0; index < input.length; index += 1) {
    hash ^= input.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}
