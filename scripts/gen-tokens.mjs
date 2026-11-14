#!/usr/bin/env node
// Read design/tokens.json and write web/src/tokens.css.
//
// Colors carry a Dark value and a Light value: `:root` (and `.light`) holds
// Light and `.dark` holds Dark, the way shadcn themes switch, and an alias names another token so
// `--ring` follows `--primary` whichever theme is on. Those are the Neutral
// color theme's values; a color the Hide color theme draws differently carries
// `hide: {value, light}` too, written into blocks scoped to `.palette-hide`, the
// class the shell puts on the document root beside the mode class
// (hide-color-theme D-10). A subtree with no such ancestor draws Neutral. The Tailwind v4 theme maps
// the utility namespaces onto these variables and resets Tailwind's own color,
// text and radius scales, so only a token can reach a class.
//
// A `palette` token is one step of a ramp, written in OKLCH and named by its
// lightness (`--gray-23` is L 23), so neighbouring steps differ by what the eye
// sees. Palette steps are written to `:root` only and reach no utility class:
// a semantic color names the step it draws through `palette`, which must match
// the hex it records, and is written as var() of that step.
//
// A `css` token is a value per mode that is not one color: an elevation's
// box-shadow (`--elevation-*`, mapped onto Tailwind's `shadow-*`) or the scrim
// under a modal (`--scrim`, a translucent color).

import fs from 'node:fs';
import path from 'node:path';
import {oklchHex} from './oklch.mjs';

export const TOKENS = 'design/tokens.json';
export const CSS = 'web/src/tokens.css';
export const ACCENTS = 'web/src/generated/accents.ts';
/** The class that selects the Hide color theme; the web shell's `theme.ts` names the same class. */
export const PALETTE_HIDE = 'palette-hide';

/** The interface text sizes Settings > Appearance > Interface text scales; terminal and editor text do not. */

export const INTERFACE_TEXT = ['--text-micro', '--text-caption', '--text-body', '--text-subhead', '--text-title', '--text-headline'];

/** The line boxes that hold one line of interface text at a fixed height, so they grow with it. */
export const INTERFACE_LINES = ['--size-sidebar-line', '--size-sidebar-line-detail', '--size-sidebar-section-header'];

export function readTokens(root = process.cwd()) {
  return JSON.parse(fs.readFileSync(path.join(root, TOKENS), 'utf8')).tokens;
}

function plain(token) {
  if (token.type === 'scalar') return String(token.value);
  if (token.type === 'number') return `${token.value}px`;
  throw new Error(`Not a numeric token: ${JSON.stringify(token)}`);
}

function color(value) {
  if (!/^#[0-9A-Fa-f]{6}$/.test(value)) throw new Error(`Not a hex color: ${value}`);
  return value.toLowerCase();
}

/** The hex a palette step draws. */
export function paletteHex(tokens, name) {
  const step = tokens[name];
  if (step?.type !== 'palette') throw new Error(`${name} is not a palette step`);
  return oklchHex(step.oklch);
}

/** The value a semantic color writes for one theme slot: its palette step when it names one. */
function slot(tokens, name, hex, step) {
  if (step === undefined) return color(hex);
  const drawn = paletteHex(tokens, step);
  if (drawn !== color(hex)) throw new Error(`${name} records ${hex} but its palette step ${step} draws ${drawn}`);
  return `var(${step})`;
}

function cssValue(name, value) {
  if (typeof value !== 'string' || !value.trim()) throw new Error(`${name} has no value`);
  if (/var\(|;|[{}]/.test(value)) throw new Error(`${name} must be a plain CSS value: ${value}`);
  return value;
}

export function generate(root = process.cwd()) {
  const tokens = readTokens(root);
  const smallControl = tokens['--size-control-sm'];
  if (smallControl?.type !== 'number') throw new Error('Expected numeric --size-control-sm');
  const overviewPanel = tokens['--size-overview-panel-breakpoint'];
  if (overviewPanel?.type !== 'number') throw new Error('Expected numeric --size-overview-panel-breakpoint');
  const paneBreakpoint = tokens['--size-pane-parent-breakpoint'];
  if (paneBreakpoint?.type !== 'number') throw new Error('Expected numeric --size-pane-parent-breakpoint');
  const paneHeadNarrow = tokens['--size-pane-head-narrow'];
  if (paneHeadNarrow?.type !== 'number') throw new Error('Expected numeric --size-pane-head-narrow');
  const paneHeadTight = tokens['--size-pane-head-tight'];
  if (paneHeadTight?.type !== 'number') throw new Error('Expected numeric --size-pane-head-tight');
  const missionWide = tokens['--size-mission-wide'];
  if (missionWide?.type !== 'number') throw new Error('Expected numeric --size-mission-wide');
  const missionPanelWords = tokens['--size-mission-panel-words'];
  if (missionPanelWords?.type !== 'number') throw new Error('Expected numeric --size-mission-panel-words');
  const missionsSplit = tokens['--size-missions-split'];
  if (missionsSplit?.type !== 'number') throw new Error('Expected numeric --size-missions-split');
  const taskTableWide = tokens['--size-task-table-wide'];
  if (taskTableWide?.type !== 'number') throw new Error('Expected numeric --size-task-table-wide');
  const theme = [], mapped = [], light = [], dark = [], hideLight = [], hideDark = [], rest = [], palette = [];
  for (const [name, token] of Object.entries(tokens)) {
    if (token.type === 'palette') {
      palette.push(`  ${name}: ${paletteHex(tokens, name)};`);
    } else if (token.type === 'css') {
      if (token.light === undefined) throw new Error(`${name} has no Light value`);
      light.push(`  ${name}: ${cssValue(name, token.light)};`);
      dark.push(`  ${name}: ${cssValue(name, token.value)};`);
      if (token.hide !== undefined) throw new Error(`${name} is shared by both color themes`);
      if (name.startsWith('--elevation-')) mapped.push(`  --shadow-${name.slice('--elevation-'.length)}: var(${name});`);
      else if (name === '--scrim') mapped.push(`  --color-scrim: var(${name});`);
      else throw new Error(`${name} is a css token with no utility namespace`);
    } else if (token.type === 'color') {
      if (!token.light) throw new Error(`${name} has no Light value`);
      const steps = token.palette;
      if (steps !== undefined && (!steps.value || !steps.light || (token.hide !== undefined) !== (steps.hide !== undefined))) throw new Error(`${name} names palette steps for some of its values only`);
      light.push(`  ${name}: ${slot(tokens, name, token.light, steps?.light)};`);
      dark.push(`  ${name}: ${slot(tokens, name, token.value, steps?.value)};`);
      if (token.hide !== undefined) {
        if (!token.hide.value || !token.hide.light) throw new Error(`${name} has a Hide value without both Dark and Light`);
        if (token.hide.value.toUpperCase() === token.value.toUpperCase() && token.hide.light.toUpperCase() === token.light.toUpperCase()) throw new Error(`${name} repeats its Neutral value as Hide; leave hide out so it inherits`);
        hideLight.push(`  ${name}: ${slot(tokens, name, token.hide.light, steps?.hide?.light)};`);
        hideDark.push(`  ${name}: ${slot(tokens, name, token.hide.value, steps?.hide?.value)};`);
      }
      mapped.push(`  --color-${name.slice(2)}: var(${name});`);
    } else if (token.type === 'alias') {
      if (!tokens[token.value]) throw new Error(`${name} aliases unknown ${token.value}`);
      if (token.hide !== undefined) throw new Error(`${name} is an alias; it takes its Hide value from ${token.value}`);
      // A custom property resolves its var() where it is declared, so each
      // theme block carries the aliases again for its own values.
      light.push(`  ${name}: var(${token.value});`);
      dark.push(`  ${name}: var(${token.value});`);
      mapped.push(`  --color-${name.slice(2)}: var(${name});`);
    } else if (INTERFACE_TEXT.includes(name)) {
      theme.push(`  ${name}: calc(${plain(token)} * var(--interface-scale, 1));`);
    } else if (INTERFACE_LINES.includes(name)) {
      rest.push(`  ${name}: calc(${plain(token)} * var(--interface-scale, 1));`);
    } else if (name.startsWith('--spacing-') || name.startsWith('--radius-')) {
      theme.push(`  ${name}: ${plain(token)};`);
    } else {
      rest.push(`  ${name}: ${plain(token)};`);
    }
  }
  const css = [
    '/* Generated from design/tokens.json by scripts/gen-tokens.mjs. Do not edit. */',
    // Container query conditions need a compiled length, not a var() expression.
    // Reserve two existing small controls' worth of title room for the count.
    `@custom-variant agent-tab-count-room (@container tab-title (min-width: ${smallControl.value * 2}px));`,
    // A row's title slot with room for a few letters of the title beside the subagent mark: narrower, the mark goes and the title keeps the room.
    `@custom-variant subagent-mark-room (@container agent-title (min-width: ${smallControl.value * 3}px));`,
    // Below this window width the Overview's side panel opens over the tabs (PRD overview-device-tasks B65).
    `@custom-variant overview-narrow (@media (width < ${overviewPanel.value}px));`,
    // A pane wide enough for its header to carry words beside its marks.
    `@custom-variant pane-wide (@container pane (min-width: ${paneBreakpoint.value}px));`,
    // A pane too narrow for its head's chips to carry words: they keep their mark and count, and `@` and zoom wait in the pane menu.
    `@custom-variant pane-head-narrow (@container pane (width < ${paneHeadNarrow.value}px));`,
    // A pane too narrow for its head's chips at all: they go and the title keeps the room.
    `@custom-variant pane-head-tight (@container pane (width < ${paneHeadTight.value}px));`,
    // A mission page wide enough for its node panel beside the map, else under it (PRD mission-map-git-criteria D-04).
    `@custom-variant mission-wide (@container mission (min-width: ${missionWide.value}px));`,
    // A mission node panel wide enough for its head's buttons to carry their words beside a long title, else their icons alone.
    `@custom-variant mission-panel-words (@container mission-panel (min-width: ${missionPanelWords.value}px));`,
    // A mission screen wide enough for its list beside the detail, else the list stacks above it (PRD mission-screen B23).
    `@custom-variant missions-split (@container missions (min-width: ${missionsSplit.value}px));`,
    // A 작업 비용 table wide enough for its tokens and working-time columns, else they ride under the task's name (PRD project-panel-results B27, B35).
    `@custom-variant task-table-wide (@container task-table (min-width: ${taskTableWide.value}px));`,
    '',
    '@theme static {',
    '  --color-*: initial;',
    '  --text-*: initial;',
    '  --radius-*: initial;',
    '  --shadow-*: initial;',
    ...theme,
    '}',
    '',
    '@theme inline {',
    ...mapped,
    '}',
    '',
    ':root {',
    ...rest,
    '}',
    '',
    '/* The palette: OKLCH ramps named by lightness. Semantic colors below draw from these; no class reaches them. */',
    ':root {',
    ...palette,
    '}',
    '',
    '/* `.light` and `.dark` also scope a subtree, so one page can show both (the dev gallery). */',
    ':root,',
    '.light {',
    ...light,
    '}',
    '',
    '.dark {',
    ...dark,
    '}',
    '',
    // After `.dark` and one class more specific than it, so the root's
    // `palette-hide dark` and a mode subtree under it both take Hide's values.
    `/* The Hide color theme: the document root carries .${PALETTE_HIDE} beside its mode class. */`,
    `.${PALETTE_HIDE},`,
    `.${PALETTE_HIDE}.light,`,
    `.${PALETTE_HIDE} .light {`,
    ...hideLight,
    '}',
    '',
    `.${PALETTE_HIDE}.dark,`,
    `.${PALETTE_HIDE} .dark {`,
    ...hideDark,
    '}',
    '',
  ].join('\n');
  // The stored accent is the choice's Dark value (the value the core has
  // always stored); the shell maps it back to the choice so each theme draws
  // that choice's own value.
  const accents = Object.entries(tokens)
    .filter(([name]) => name.startsWith('--accent-choice-'))
    .map(([name, token]) => `  ${JSON.stringify(name.slice('--accent-choice-'.length))}: ${JSON.stringify(color(token.value))},`);
  const ts = [
    '/* Generated from design/tokens.json by scripts/gen-tokens.mjs. Do not edit. */',
    '',
    '/** Each accent choice by name, with the value the core stores for it. */',
    'export const ACCENT_STORED_HEX = {',
    ...accents,
    '} as const;',
    '',
  ].join('\n');
  return { css, ts, tokens };
}

const invoked = process.argv[1] && path.basename(process.argv[1]) === 'gen-tokens.mjs';
if (invoked) {
  const root = process.cwd();
  const { css, ts } = generate(root);
  fs.writeFileSync(path.join(root, CSS), css);
  fs.writeFileSync(path.join(root, ACCENTS), ts);
  console.log(`wrote ${CSS} and ${ACCENTS} from ${TOKENS}`);
}
