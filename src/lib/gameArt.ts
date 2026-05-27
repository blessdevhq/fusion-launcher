import type { CatalogGame } from '../types/repository.ts';

const PALETTES = [
  ['#151828', '#4f46e5', '#22d3ee'],
  ['#17131f', '#8b5cf6', '#f59e0b'],
  ['#101821', '#10b981', '#38bdf8'],
  ['#1a1018', '#ef4444', '#f97316'],
  ['#121723', '#64748b', '#a78bfa'],
  ['#11151c', '#14b8a6', '#f43f5e']
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
    `radial-gradient(circle at ${offset}% 18%, ${palette[2]}66, transparent 26%)`,
    `radial-gradient(circle at 88% ${offset + 12}%, ${palette[1]}88, transparent 34%)`,
    `linear-gradient(${angle}deg, ${palette[0]}, #050609 68%)`
  ].join(', ');

  const heroPattern = [
    `radial-gradient(circle at 70% 22%, ${palette[2]}55, transparent 24%)`,
    `radial-gradient(circle at 84% 64%, ${palette[1]}7a, transparent 34%)`,
    `linear-gradient(100deg, #050609 0%, ${palette[0]} 42%, #050609 100%)`
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

export function hashString(input: string): number {
  let hash = 2166136261;
  for (let index = 0; index < input.length; index += 1) {
    hash ^= input.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}
