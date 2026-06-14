import { writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const outputPath = resolve(root, 'public/demo-content/fusion-launcher-smoke.nes');

const prg = new Uint8Array(16 * 1024).fill(0xea);
const chr = new Uint8Array(8 * 1024);
const code = [];
const labels = new Map();
const patches = [];

const base = 0x8000;
const pc = () => base + code.length;
const label = (name) => labels.set(name, pc());
const byte = (...values) => code.push(...values.map((value) => value & 0xff));
const word = (value) => byte(value, value >> 8);

function abs(opcode, address) {
  byte(opcode);
  word(address);
}

function absLabel(opcode, name) {
  byte(opcode);
  patches.push({ offset: code.length, name, relative: false });
  word(0);
}

function branch(opcode, name) {
  byte(opcode);
  patches.push({ offset: code.length, name, relative: true, from: pc() + 1 });
  byte(0);
}

const palette = [
  0x0f, 0x21, 0x30, 0x16,
  0x0f, 0x11, 0x28, 0x30,
  0x0f, 0x06, 0x16, 0x26,
  0x0f, 0x00, 0x10, 0x20,
  0x0f, 0x21, 0x30, 0x16,
  0x0f, 0x11, 0x28, 0x30,
  0x0f, 0x06, 0x16, 0x26,
  0x0f, 0x00, 0x10, 0x20
];
const message = 'FUSION LAUNCHER DEMO';
const messageTiles = [...message].map(tileForChar);

label('reset');
byte(0x78); // SEI
byte(0xd8); // CLD
byte(0xa2, 0x40); // LDX #$40
abs(0x8e, 0x4017); // STX $4017
byte(0xa2, 0xff); // LDX #$FF
byte(0x9a); // TXS
byte(0xe8); // INX -> 0
abs(0x8e, 0x2000); // STX PPUCTRL
abs(0x8e, 0x2001); // STX PPUMASK
abs(0x8e, 0x4010); // STX DMC IRQ

label('wait_vblank_1');
abs(0x2c, 0x2002); // BIT PPUSTATUS
branch(0x10, 'wait_vblank_1'); // BPL
label('wait_vblank_2');
abs(0x2c, 0x2002);
branch(0x10, 'wait_vblank_2');

abs(0xad, 0x2002); // LDA PPUSTATUS
byte(0xa9, 0x3f);
abs(0x8d, 0x2006);
byte(0xa9, 0x00);
abs(0x8d, 0x2006);
byte(0xa2, 0x00);
label('palette_loop');
absLabel(0xbd, 'palette'); // LDA palette,X
abs(0x8d, 0x2007);
byte(0xe8);
byte(0xe0, palette.length);
branch(0xd0, 'palette_loop');

byte(0xa9, 0x20);
abs(0x8d, 0x2006);
byte(0xa9, 0x00);
abs(0x8d, 0x2006);
byte(0xa0, 0x04);
byte(0xa2, 0x00);
label('clear_loop');
byte(0xa9, 0x00);
abs(0x8d, 0x2007);
byte(0xe8);
branch(0xd0, 'clear_loop');
byte(0x88);
branch(0xd0, 'clear_loop');

byte(0xa9, 0x21);
abs(0x8d, 0x2006);
byte(0xa9, 0xc8);
abs(0x8d, 0x2006);
byte(0xa2, 0x00);
label('message_loop');
absLabel(0xbd, 'message'); // LDA message,X
abs(0x8d, 0x2007);
byte(0xe8);
byte(0xe0, messageTiles.length);
branch(0xd0, 'message_loop');

byte(0xa9, 0x00);
abs(0x8d, 0x2005);
abs(0x8d, 0x2005);
abs(0x8d, 0x2000);
byte(0xa9, 0x1e);
abs(0x8d, 0x2001);
label('forever');
byte(0x4c);
word(labels.get('forever'));

label('palette');
byte(...palette);
label('message');
byte(...messageTiles);

for (const patch of patches) {
  const target = labels.get(patch.name);
  if (target === undefined) {
    throw new Error(`Unknown label: ${patch.name}`);
  }
  if (patch.relative) {
    const offset = target - patch.from;
    if (offset < -128 || offset > 127) {
      throw new Error(`Branch out of range: ${patch.name}`);
    }
    code[patch.offset] = offset & 0xff;
  } else {
    code[patch.offset] = target & 0xff;
    code[patch.offset + 1] = target >> 8;
  }
}

prg.set(code, 0);
prg[0x3ffa] = base & 0xff;
prg[0x3ffb] = base >> 8;
prg[0x3ffc] = base & 0xff;
prg[0x3ffd] = base >> 8;
prg[0x3ffe] = base & 0xff;
prg[0x3fff] = base >> 8;

buildChr(chr);

const header = Uint8Array.from([
  0x4e, 0x45, 0x53, 0x1a,
  0x01, // 16KB PRG
  0x01, // 8KB CHR
  0x00, // mapper 0, horizontal mirroring
  0x00,
  0x00, 0x00, 0x00, 0x00,
  0x00, 0x00, 0x00, 0x00
]);

await writeFile(outputPath, Buffer.concat([header, prg, chr]));
console.log(`Generated ${outputPath}`);

function tileForChar(char) {
  if (char === ' ') return 0;
  if (char >= 'A' && char <= 'Z') return char.charCodeAt(0) - 64;
  if (char >= '0' && char <= '9') return 27 + Number(char);
  return 0;
}

function buildChr(target) {
  const glyphs = {
    A: ['01110', '10001', '10001', '11111', '10001', '10001', '10001'],
    D: ['11110', '10001', '10001', '10001', '10001', '10001', '11110'],
    E: ['11111', '10000', '10000', '11110', '10000', '10000', '11111'],
    H: ['10001', '10001', '10001', '11111', '10001', '10001', '10001'],
    M: ['10001', '11011', '10101', '10101', '10001', '10001', '10001'],
    O: ['01110', '10001', '10001', '10001', '10001', '10001', '01110'],
    R: ['11110', '10001', '10001', '11110', '10100', '10010', '10001'],
    T: ['11111', '00100', '00100', '00100', '00100', '00100', '00100'],
    Y: ['10001', '10001', '01010', '00100', '00100', '00100', '00100']
  };

  for (let charCode = 65; charCode <= 90; charCode += 1) {
    const char = String.fromCharCode(charCode);
    const tile = tileForChar(char);
    writeGlyph(target, tile, glyphs[char] ?? fallbackGlyph(charCode));
  }

  for (let digit = 0; digit <= 9; digit += 1) {
    writeGlyph(target, 27 + digit, digitGlyph(digit));
  }
}

function writeGlyph(target, tile, rows) {
  const offset = tile * 16;
  rows.forEach((row, rowIndex) => {
    let value = 0;
    for (let column = 0; column < row.length; column += 1) {
      if (row[column] === '1') {
        value |= 1 << (6 - column);
      }
    }
    target[offset + rowIndex] = value;
  });
}

function fallbackGlyph(seed) {
  return Array.from({ length: 7 }, (_, row) => {
    return Array.from({ length: 5 }, (_, column) => (
      row === 0 || row === 6 || column === 0 || column === 4 || ((seed + row + column) % 5 === 0)
        ? '1'
        : '0'
    )).join('');
  });
}

function digitGlyph(digit) {
  const glyphs = [
    ['01110', '10001', '10011', '10101', '11001', '10001', '01110'],
    ['00100', '01100', '00100', '00100', '00100', '00100', '01110'],
    ['01110', '10001', '00001', '00010', '00100', '01000', '11111'],
    ['11110', '00001', '00001', '01110', '00001', '00001', '11110'],
    ['00010', '00110', '01010', '10010', '11111', '00010', '00010'],
    ['11111', '10000', '10000', '11110', '00001', '00001', '11110'],
    ['01110', '10000', '10000', '11110', '10001', '10001', '01110'],
    ['11111', '00001', '00010', '00100', '01000', '01000', '01000'],
    ['01110', '10001', '10001', '01110', '10001', '10001', '01110'],
    ['01110', '10001', '10001', '01111', '00001', '00001', '01110']
  ];
  return glyphs[digit];
}
