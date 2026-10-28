import test from 'node:test';
import assert from 'node:assert/strict';
import { portableConfig } from './totk-local.mjs';

test('portable profile keeps F68/binds/hotkeys and removes old data locations', () => {
  const input = '[Controls]\nmouse_panning_x_sensitivity=68\nplayer_0_button_a="engine:keyboard,code:69"\n[Data%20Storage]\nnand_directory=F:/old/user/nand\nnand_directory\\default=false\nsave_directory=F:/old/save\nsave_directory\\default=false\n[UI]\nPaths\\gamedirs\\1\\path=F:/old/games\nPaths\\recentFiles=F:/old/game.nsp\nShortcuts\\Main%20Window\\Exit%20Eden\\KeySeq=\n';
  const result = portableConfig(input);
  assert.ok(!result.includes('F:/old'));
  assert.ok(result.includes('nand_directory\\default=true'));
  assert.ok(result.includes('save_directory\\default=true'));
  assert.ok(result.includes('mouse_panning_x_sensitivity=68'));
  assert.ok(result.includes('player_0_button_a="engine:keyboard,code:69"'));
  assert.ok(result.includes('Exit%20Eden\\KeySeq=\n'));
});

test('portable profile relocation is idempotent', () => {
  const once = portableConfig('[Audio]\nvolume=100\n[Data%20Storage]\nload_directory=F:/old\nload_directory\\default=false\n');
  assert.equal(portableConfig(once), once);
});
