#!/usr/bin/env node
// Write design/hide-screens.pen: every `Screen / <Area>` sheet (PRD
// web-design-system-reset B19, D-18, D-19).
//
//   node scripts/gen-screens.mjs
//
// Regeneration owns every `Screen /` sheet's content, removes the sheets
// `RETIRED_SHEETS` names, and owns this file's local token variables (kept in sync with design/tokens.json through the same pen-tokens.mjs
// pipeline gen-pen.mjs uses), never a sheet's placement (x, y) once drawn - the
// designer's layout on the canvas survives a rerun exactly as pen-bands.mjs
// preserves it for design/hide-ui.lib.pen.

import fs from 'node:fs';
import path from 'node:path';
import {readTokens} from './gen-tokens.mjs';
import {screenSheets, readLocalVariables, themedOverrides, ALIAS, LIBRARY_PATH, RETIRED_SHEETS} from './pen-screens.mjs';
import {THEMES} from './pen-tokens.mjs';
import {serialize} from './pen-bands.mjs';
import {transplant} from './pen-transplant.mjs';

export const FILE = 'design/hide-screens.pen';
const GRID_STRIDE_X = 1400;
const GRID_STRIDE_Y = 900;
const GRID_COLUMNS = 3;

function loadExisting(root) {
  const file = path.join(root, FILE);
  if (!fs.existsSync(file)) return null;
  return JSON.parse(fs.readFileSync(file, 'utf8'));
}

export function generate(root, ownedSheets = []) {
  const tokens = readTokens(root);
  const variables = readLocalVariables(root);
  const existing = loadExisting(root);
  const before = existing ? serialize(existing) : null;
  const generated = screenSheets(tokens, root);

  // A retired sheet drew a screen that is gone, so it goes with it; an authored sheet the generator never built stays.
  let children = existing ? existing.children.filter(node => !RETIRED_SHEETS.includes(node.name)) : [];
  let placed = children.length;
  for (const {name, build} of generated) {
    const found = children.find(node => node.name === name);
    const sheet = found ? {...build(), x: found.x, y: found.y} : {...build(), x: (placed % GRID_COLUMNS) * GRID_STRIDE_X, y: Math.floor(placed / GRID_COLUMNS) * GRID_STRIDE_Y};
    if (!found) placed++;
    children = found ? children.map(node => node === found ? sheet : node) : [...children, sheet];
  }

  let document = {version: existing?.version ?? '2.18', themes: THEMES, imports: {[ALIAS]: LIBRARY_PATH}, variables, children};
  if (ownedSheets.length) {
    if (!existing) throw new Error('Scoped regeneration needs an existing screen document');
    // A scoped run keeps every other sheet as it is, except one the product retired.
    document = transplant(document, {...existing, variables, children: existing.children.filter(node => !RETIRED_SHEETS.includes(node.name))}, ownedSheets).document;
  }
  // An existing screen may import a master that gained a colorable child.
  // Restate only missing inherited colors, including authored sheets, without
  // replacing their layout, content or explicit overrides (Dark import rule).
  function restate(node) {
    // Retained authored Files states are outside generated Workspace boards.
    // Their IDs name their tool role; no translated display label decides it.
    if (['stl-i-l-f-lane-head', 'stl-i-d-f-lane-head'].includes(node.id)) {
      (node.descendants ??= {})[`${ALIAS}:lane-header-explorer`] = {...node.descendants[`${ALIAS}:lane-header-explorer`], enabled: true};
    }
    if (node.type === 'ref' && node.ref.startsWith(`${ALIAS}:`)) {
      const colors = themedOverrides(node.ref.slice(ALIAS.length + 1));
      for (const [key, props] of Object.entries(colors.descendants)) {
        const address = key.split('/').map(part => `${ALIAS}:${part}`).join('/');
        const patch = node.descendants?.[address] ?? {};
        const missing = Object.entries(props).filter(([prop]) => !Object.hasOwn(patch, prop));
        if (missing.length) (node.descendants ??= {})[address] = {...patch, ...Object.fromEntries(missing)};
      }
    }
    for (const child of node.children ?? []) restate(child);
  }
  document.children.forEach(restate);
  const after = serialize(document);
  return {file: path.join(root, FILE), before, after, sheetCount: generated.length};
}

const invoked = process.argv[1] && path.basename(process.argv[1]) === 'gen-screens.mjs';
if (invoked) {
  const args = process.argv.slice(2);
  const ownedSheets = [];
  for (let i = 0; i < args.length; i += 2) {
    if (args[i] !== '--sheet' || !args[i + 1]) throw new Error('Expected --sheet <owned-sheet-id>');
    ownedSheets.push(args[i + 1]);
  }
  const {file, before, after, sheetCount} = generate(process.cwd(), ownedSheets);
  if (before === after) {
    console.log(`${FILE} is already what the generator writes: ${sheetCount} screen sheets.`);
  } else {
    fs.writeFileSync(file, after);
    console.log(`${FILE} ${before === null ? 'created' : 'updated'}: ${sheetCount} screen sheets written; sheet placement preserved.`);
  }
}
