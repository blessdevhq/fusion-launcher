import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { buildLibrarySearchIndex, librarySearchTerms, matchesLibrarySearch } from './librarySearch.ts';

function matches(fields: readonly string[], query: string) {
  return matchesLibrarySearch(buildLibrarySearchIndex(fields), librarySearchTerms(query));
}

describe('library search matching', () => {
  it('finds Latin-accented titles with an unaccented query', () => {
    assert.equal(matches(['Pokémon'], 'pokemon'), true);
  });

  it('finds a title when its separators are left out of the query', () => {
    assert.equal(matches(['Sky-Racer!!'], 'skyracer'), true);
  });

  it('normalizes precomposed and decomposed Latin accents on either side', () => {
    assert.equal(matches(['Poke\u0301mon'], 'POKÉMON'), true);
    assert.equal(matches(['Pokemon'], 'poke\u0301mon'), true);
    assert.equal(matches(['CAFÉ'], 'cafe'), true);
  });

  it('accepts spaces and punctuation without losing multi-word matching', () => {
    for (const query of ['sky racer', 'sky-racer', '  SKY   RACER  ']) {
      assert.equal(matches(['Sky-Racer!!'], query), true, query);
    }
    for (const query of ['spiderman', 'spider man', 'spider—man']) {
      assert.equal(matches(['Spider-Man'], query), true, query);
    }
  });

  it('preserves Cyrillic letter distinctions when folding Latin accents', () => {
    assert.equal(matches(['Йоши'], 'йоши'), true);
    assert.equal(matches(['Йоши'], 'Иоши'), false);
    assert.equal(matches(['Ёж'], 'Еж'), false);
    assert.equal(matches(['И\u0306оши'], 'Йоши'), true);
  });

  it('preserves non-Latin names and meaningful marks', () => {
    assert.equal(matches(['ゼルダの伝説'], 'セ\u3099ルダ'), true);
    assert.equal(matches(['ゼルダ'], 'セルダ'), false);
    assert.equal(matches(['超级马里奥'], '马里奥'), true);
    assert.equal(matches(['किताब'], 'किताब'), true);
    assert.equal(matches(['किताब'], 'कताब'), false);
  });

  it('requires every query term but allows different fields and term order', () => {
    assert.equal(matches(['Alpha Quest', 'GBA'], 'gba alpha'), true);
    assert.equal(matches(['Alpha Quest', 'GBA'], 'alpha gba'), true);
    assert.equal(matches(['Alpha Quest', 'GBA'], 'gba beta'), false);
  });

  it('does not form compact words across field boundaries', () => {
    assert.equal(matches(['Sky', 'Racer'], 'skyracer'), false);
    assert.equal(matches(['Sky', 'Racer'], 'sky racer'), true);
  });

  it('treats empty and separator-only queries as no filter', () => {
    for (const query of ['', '  ', '—_!!!', '🎮']) {
      assert.deepEqual(librarySearchTerms(query), [], query);
      assert.equal(matches([], query), true, query);
    }
    assert.equal(matches([], 'pokemon'), false);
  });

  it('builds a reusable index without changing source fields or query terms', () => {
    const fields = Object.freeze(['Pokémon', undefined, null, 1996, 'Sky-Racer!!']);
    const index = Object.freeze(buildLibrarySearchIndex(fields));
    const terms = Object.freeze(librarySearchTerms('pokemon 1996'));

    assert.deepEqual(index, ['pokemon', '1996', 'skyracer']);
    assert.equal(matchesLibrarySearch(index, terms), true);
    assert.equal(matchesLibrarySearch(index, librarySearchTerms('skyracer')), true);
    assert.equal(fields[0], 'Pokémon');
    assert.deepEqual(terms, ['pokemon', '1996']);
    assert.deepEqual(buildLibrarySearchIndex([undefined, null, '', '---']), []);
  });
});
