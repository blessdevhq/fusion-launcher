function normalizeSearchText(value: string): string {
  // Remove only Latin accents, then recompose meaningful marks (й, ё, voiced kana).
  return value.normalize('NFD').toLowerCase()
    .replace(/(\p{Script=Latin})\p{M}+/gu, '$1')
    .normalize('NFC')
    .replace(/[^\p{L}\p{N}\p{M}]+/gu, ' ')
    .trim();
}

/** Keep fields separate so a query cannot join a title to its source name. */
export function buildLibrarySearchIndex(fields: readonly (string | number | null | undefined)[]): string[] {
  return fields.map((field) => normalizeSearchText(String(field ?? '')).replace(/ /g, '')).filter(Boolean);
}

export function librarySearchTerms(query: string): string[] {
  return normalizeSearchText(query).split(/\s+/).filter(Boolean);
}

export function matchesLibrarySearch(index: readonly string[], terms: readonly string[]): boolean {
  return terms.every((term) => index.some((field) => field.includes(term)));
}
