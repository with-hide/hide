#!/usr/bin/env node
// Refuse a web source that writes a value the tokens do not own.
//
// `design/tokens.json` is the numeric and color authority (DESIGN_WORKFLOW.md):
// the generated `web/src/tokens.css` must be current, and a class or style in
// `web/src` must reach every color, size and radius through a token. Tailwind's
// own scales are literals too: `p-4` or `text-sm` is a number nobody chose in
// the token file, so the default numeric spacing and named text/radius sizes
// are refused along with arbitrary px and hex values.

import fs from 'node:fs';
import path from 'node:path';
import { ACCENTS, CSS, generate } from './gen-tokens.mjs';

const root = process.argv[2] ?? process.cwd();
const failures = [];

const generated = generate(root);
for (const [file, text] of [[CSS, generated.css], [ACCENTS, generated.ts]]) {
  const full = path.join(root, file);
  const current = fs.existsSync(full) ? fs.readFileSync(full, 'utf8') : '';
  if (current !== text) failures.push(`${file} is stale. Run \`node scripts/gen-tokens.mjs\`.`);
}

function walk(dir, acc = []) {
  if (!fs.existsSync(dir)) return acc;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory() && entry.name !== 'generated' && entry.name !== 'node_modules') {
      walk(full, acc);
    } else if (entry.isFile() && /\.(ts|tsx|css)$/.test(entry.name) && entry.name !== 'tokens.css' && !entry.name.endsWith('.test.ts')) {
      // A unit test's values are data about a stored value, not a style it draws.
      acc.push(full);
    }
  }
  return acc;
}

const SPACING = '(?:p|px|py|pt|pb|pl|pr|ps|pe|m|mx|my|mt|mb|ml|mr|ms|me|gap|gap-x|gap-y|w|h|size|min-w|min-h|max-w|max-h|top|left|right|bottom|start|end|inset|inset-x|inset-y|space-x|space-y|translate-x|translate-y|scroll-m|scroll-p|basis)';
// A leading `-` or a variant prefix still names the same utility; `0` and `px` are not scale values.
const DEFAULT_SCALE = new RegExp(`(?<![\\w-])-?${SPACING}-(?:[1-9][0-9]*(?:\\.5)?|0\\.5)(?![\\w./-])`);
const DEFAULT_TEXT = /(?<![\w-])text-(?:xs|sm|base|lg|xl|[2-9]xl)(?![\w-])/;
// A container or media query condition takes a compiled length, never var():
// Tailwind drops the whole rule, so the class silently never applies.
const VAR_BREAKPOINT = /(?<![\w-])@?(?:min|max)-(?:\[(?:var|calc)\(|\(--)[^\s"'`]*/;

const files = walk(path.join(root, 'web/src'));
for (const file of files) {
  const source = fs.readFileSync(file, 'utf8');
  const rel = path.relative(root, file);
  if (/\[#[0-9A-Fa-f]{3,8}\]/.test(source)) failures.push(`${rel}: Tailwind arbitrary color`);
  if (/\[[0-9.]+px\]/.test(source)) failures.push(`${rel}: Tailwind arbitrary px`);
  if (/style=\{\{[^}]*?(?:background|color|border)/.test(source)) failures.push(`${rel}: inline color style`);
  // A literal outside a class reaches the page too: a CodeMirror theme object,
  // a style string, or another stylesheet. tokens.css is not walked.
  if (/#[0-9A-Fa-f]{3,8}\b/.test(source)) failures.push(`${rel}: hex color literal`);
  if (/\b(?:rgb|hsl)a?\(/.test(source)) failures.push(`${rel}: rgb/hsl color literal`);
  const px = /\b[0-9]+(?:\.[0-9]+)?px\b/.exec(source);
  if (px) failures.push(`${rel}: px literal \`${px[0]}\`; use a --size-* or --spacing-* token`);
  const scale = DEFAULT_SCALE.exec(source);
  if (scale) failures.push(`${rel}: Tailwind default scale \`${scale[0]}\`; use a --spacing-* or --size-* token`);
  const text = DEFAULT_TEXT.exec(source);
  if (text) failures.push(`${rel}: Tailwind default text size \`${text[0]}\`; use a --text-* token`);
  const breakpoint = VAR_BREAKPOINT.exec(source);
  if (breakpoint) failures.push(`${rel}: query variant on a custom property \`${breakpoint[0]}\` never applies; add a variant to scripts/gen-tokens.mjs, as pane-wide is`);
}

if (failures.length) {
  console.error(failures.join('\n'));
  process.exit(1);
}
console.log('web token contract ok');
