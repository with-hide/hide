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

import fs from 'node:fs';
import path from 'node:path';

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

export function generate(root = process.cwd()) {
  const tokens = readTokens(root);
  const smallControl = tokens['--size-control-sm'];
  if (smallControl?.type !== 'number') throw new Error('Expected numeric --size-control-sm');
  const overviewPanel = tokens['--size-overview-panel-breakpoint'];
  if (overviewPanel?.type !== 'number') throw new Error('Expected numeric --size-overview-panel-breakpoint');
  const paneBreakpoint = tokens['--size-pane-parent-breakpoint'];
  if (paneBreakpoint?.type !== 'number') throw new Error('Expected numeric --size-pane-parent-breakpoint');
  const paneTurn = tokens['--size-pane-turn-breakpoint'];
  if (paneTurn?.type !== 'number') throw new Error('Expected numeric --size-pane-turn-breakpoint');
  const missionWide = tokens['--size-mission-wide'];
  if (missionWide?.type !== 'number') throw new Error('Expected numeric --size-mission-wide');
  const theme = [], mapped = [], light = [], dark = [], hideLight = [], hideDark = [], rest = [];
  for (const [name, token] of Object.entries(tokens)) {
    if (token.type === 'color') {
      if (!token.light) throw new Error(`${name} has no Light value`);
      light.push(`  ${name}: ${color(token.light)};`);
      dark.push(`  ${name}: ${color(token.value)};`);
      if (token.hide !== undefined) {
        if (!token.hide.value || !token.hide.light) throw new Error(`${name} has a Hide value without both Dark and Light`);
        if (token.hide.value.toUpperCase() === token.value.toUpperCase() && token.hide.light.toUpperCase() === token.light.toUpperCase()) throw new Error(`${name} repeats its Neutral value as Hide; leave hide out so it inherits`);
        hideLight.push(`  ${name}: ${color(token.hide.light)};`);
        hideDark.push(`  ${name}: ${color(token.hide.value)};`);
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
    // Below this window width the Overview's side panel opens over the tabs (PRD overview-device-tasks B65).
    `@custom-variant overview-narrow (@media (width < ${overviewPanel.value}px));`,
    // A pane wide enough for its header to carry words beside its marks.
    `@custom-variant pane-wide (@container pane (min-width: ${paneBreakpoint.value}px));`,
    // A pane wide enough for its head to carry the turn's words after the title; narrower, the status mark alone says it.
    `@custom-variant pane-turn-room (@container pane (min-width: ${paneTurn.value}px));`,
    // A mission page wide enough for its node panel beside the map, else under it (PRD mission-map-git-criteria D-04).
    `@custom-variant mission-wide (@container mission (min-width: ${missionWide.value}px));`,
    '',
    '@theme static {',
    '  --color-*: initial;',
    '  --text-*: initial;',
    '  --radius-*: initial;',
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
