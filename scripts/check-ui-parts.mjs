#!/usr/bin/env node
// Refuse a raw control or layer drawn outside the parts (docs/DESIGN_GUIDE.md).
//
// A screen that restyles its own `<button>` or opens its own `role="dialog"`
// drifts from the parts: its hover, focus ring and disabled state are its own,
// and a hand-made layer never joins the shell's Escape owner. The existing ones
// are counted per file in `scripts/ui-parts-baseline.json`; a file may only go
// down. A higher count fails, and a lower one asks for `--update` so the lower
// number becomes the new ceiling. `--update` never raises the total of a kind,
// so moving or splitting a file carries its count but adding one does not.

import fs from 'node:fs';
import path from 'node:path';

const BASELINE = 'scripts/ui-parts-baseline.json';
const KINDS = {
  button: [/<button\b/g, 'use `Button` (components/ui/button), or add the variant it lacks there'],
  input: [/<input\b/g, 'use `Input`, `Checkbox`, `Switch`, `RadioGroup` or `Slider` from components/ui'],
  select: [/<select\b/g, 'use `Select` (components/ui/select)'],
  textarea: [/<textarea\b/g, 'add a multi-line part to components/ui and use it'],
  layer: [null, 'open it through `Dialog`, `AlertDialog`, `Popover`, `Sheet` or `DropdownMenu`'],
};

// A layer role on a raw element (`<div role="dialog">`). The same role on a
// part (`<PopoverContent role="dialog">`) is the part's layer, and a
// `[role="menu"]` selector string is no element at all.
function rawLayers(source) {
  let n = 0;
  for (const match of source.matchAll(/(?<=\s)role="(?:dialog|alertdialog|menu)"/g)) {
    const open = source.lastIndexOf('<', match.index);
    if (/^<[a-z]/.test(source.slice(open, open + 2))) n += 1;
  }
  return n;
}

const args = process.argv.slice(2);
const update = args.includes('--update');
const root = args.find(arg => !arg.startsWith('--')) ?? process.cwd();

function walk(dir, acc = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (path.relative(root, full) !== path.join('web', 'src', 'components', 'ui')) walk(full, acc);
    } else if (entry.name.endsWith('.tsx') && !entry.name.endsWith('.test.tsx')) acc.push(full);
  }
  return acc;
}

function count() {
  const counts = {};
  for (const file of walk(path.join(root, 'web', 'src')).sort()) {
    const source = fs.readFileSync(file, 'utf8');
    const rel = path.relative(root, file).split(path.sep).join('/');
    for (const [kind, [pattern]] of Object.entries(KINDS)) {
      const n = pattern ? (source.match(pattern)?.length ?? 0) : rawLayers(source);
      if (n) (counts[rel] ??= {})[kind] = n;
    }
  }
  return counts;
}

function totals(counts) {
  const sum = {};
  for (const kinds of Object.values(counts)) for (const [kind, n] of Object.entries(kinds)) sum[kind] = (sum[kind] ?? 0) + n;
  return sum;
}

const baselinePath = path.join(root, BASELINE);
const baseline = JSON.parse(fs.readFileSync(baselinePath, 'utf8'));
const current = count();

if (update) {
  const before = totals(baseline), after = totals(current);
  const grown = Object.keys(KINDS).filter(kind => (after[kind] ?? 0) > (before[kind] ?? 0));
  if (grown.length) {
    for (const kind of grown) console.error(`raw ${kind} grew from ${before[kind] ?? 0} to ${after[kind]}: ${KINDS[kind][1]}`);
    process.exit(1);
  }
  fs.writeFileSync(baselinePath, `${JSON.stringify(current, null, 2)}\n`);
  console.log(`${BASELINE} updated`);
  process.exit(0);
}

const failures = [];
let lowered = false;
for (const file of new Set([...Object.keys(baseline), ...Object.keys(current)])) {
  for (const kind of Object.keys(KINDS)) {
    const allowed = baseline[file]?.[kind] ?? 0, n = current[file]?.[kind] ?? 0;
    if (n > allowed) failures.push(`${file}: ${n} raw ${kind} (baseline ${allowed}); ${KINDS[kind][1]}`);
    else if (n < allowed) lowered = true;
  }
}
if (lowered && !failures.length) {
  failures.push(`raw controls went down; run \`node scripts/check-ui-parts.mjs --update\` to lock the lower counts into ${BASELINE}`);
}
if (failures.length) {
  console.error(failures.join('\n'));
  console.error('See docs/DESIGN_GUIDE.md, Which part.');
  process.exit(1);
}
const sum = totals(current);
console.log(`ui parts ok: ${Object.keys(KINDS).map(kind => `${sum[kind] ?? 0} ${kind}`).join(', ')} outside components/ui, none above baseline`);
