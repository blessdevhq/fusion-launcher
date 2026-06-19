import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { createGameArt, hashString, selectGameArtImageUrl } from './gameArt.ts';

const game = {
  id: 'repo::demo-game',
  platform: 'switch',
  title: 'Demo Homebrew Entry'
};

describe('generated game art', () => {
  it('is deterministic for the same game', () => {
    assert.deepEqual(createGameArt(game), createGameArt(game));
  });

  it('changes when the game identity changes', () => {
    assert.notDeepEqual(
      createGameArt(game).posterStyle,
      createGameArt({ ...game, id: 'repo::other-game' }).posterStyle
    );
  });

  it('creates stable unsigned hashes', () => {
    assert.equal(hashString('Fusion Launcher'), hashString('Fusion Launcher'));
    assert.ok(hashString('Fusion Launcher') >= 0);
  });

  it('uses hero artwork for the large hero when both cover and hero are available', () => {
    const imageUrl = selectGameArtImageUrl({
      coverImageUrl: 'https://example.com/source-cover.jpg',
      artwork: {
        cover: 'https://cdn2.steamgriddb.com/grid/cover.png',
        hero: 'https://cdn2.steamgriddb.com/hero/custom.jpg'
      }
    }, 'hero');

    assert.equal(imageUrl, 'https://cdn2.steamgriddb.com/hero/custom.jpg');
  });

  it('falls back from placeholder cover URLs to hero artwork', () => {
    const imageUrl = selectGameArtImageUrl({
      coverImageUrl: 'https://images.igdb.com/...png',
      artwork: {
        cover: 'https://images.igdb.com/...png',
        hero: 'https://cdn2.steamgriddb.com/hero/hero.png'
      }
    }, 'hero');

    assert.equal(imageUrl, 'https://cdn2.steamgriddb.com/hero/hero.png');
  });
});
