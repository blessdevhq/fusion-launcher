import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { createGameArt, hashString } from './gameArt.ts';

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
});
