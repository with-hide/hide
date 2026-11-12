import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {generate} from '../gen-tokens.mjs';
import {oklchHex} from '../oklch.mjs';

const repo = path.resolve(import.meta.dirname, '../..');

/** A checkout holding the repository's tokens with one change. */
function withTokens(t, change) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'hide-gen-tokens-'));
  t.after(() => fs.rmSync(root, {recursive: true, force: true}));
  const doc = JSON.parse(fs.readFileSync(path.join(repo, 'design/tokens.json'), 'utf8'));
  change(doc.tokens);
  fs.mkdirSync(path.join(root, 'design'));
  fs.writeFileSync(path.join(root, 'design/tokens.json'), JSON.stringify(doc));
  return root;
}

test('an OKLCH step draws the sRGB hex of its lightness, chroma and hue', () => {
  assert.equal(oklchHex([100, 0, 0]), '#ffffff');
  assert.equal(oklchHex([0, 0, 0]), '#000000');
  // Neutral's old Dark background, measured as L 17.7, C 0.003, h 248.
  assert.equal(oklchHex([17.7, 0.003, 248]), '#101112');
  // Out of gamut: it keeps its lightness and hue and loses chroma until it fits.
  const clipped = oklchHex([60, 0.4, 25]);
  assert.match(clipped, /^#[0-9a-f]{6}$/);
});

test('a semantic color draws its palette step through var(), and steps reach no utility class', t => {
  const {css} = generate(withTokens(t, () => {}));
  assert.match(css, /\.dark \{[^}]*--background: var\(--gray-17\);/s);
  assert.match(css, /--gray-17: #[0-9a-f]{6};/);
  assert.doesNotMatch(css, /--color-gray-17/);
  assert.match(css, /--shadow-overlay: var\(--elevation-overlay\);/);
  assert.match(css, /--color-scrim: var\(--scrim\);/);
});

test('a semantic color whose recorded hex is not its palette step is refused', t => {
  const root = withTokens(t, tokens => { tokens['--card'].value = '#1D1F21'; });
  assert.throws(() => generate(root), /--card records #1D1F21 but its palette step --gray-23 draws #[0-9a-f]{6}/);
});

test('a semantic color that names palette steps for some themes only is refused', t => {
  const root = withTokens(t, tokens => { delete tokens['--card'].palette.hide; });
  assert.throws(() => generate(root), /--card names palette steps for some of its values only/);
});

test('a css token is a plain value, never a var() or a second declaration', t => {
  const root = withTokens(t, tokens => { tokens['--scrim'].value = 'var(--background)'; });
  assert.throws(() => generate(root), /--scrim must be a plain CSS value/);
});
