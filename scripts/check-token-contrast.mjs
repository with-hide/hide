#!/usr/bin/env node
// Refuse a token change that makes text unreadable on the surface it sits on.
//
//   node scripts/check-token-contrast.mjs
//
// For each color theme (Neutral, Hide) in each mode (Dark, Light), every
// terminal color a program writes text in must reach 4.5:1 against the
// terminal's background, and the secondary text greys must reach 4.5:1 on the
// background, the sidebar and a card (hide-color-theme D-07, B10). A terminal
// color used only as a background is left out: black in Dark, white and bright
// white in Light. The ratio is WCAG 2's contrast ratio of relative luminance.

import {readTokens} from './gen-tokens.mjs';

const MINIMUM = 4.5;
const ANSI = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'];
const TERMINAL = ANSI.flatMap(name => [`--terminal-${name}`, `--terminal-bright-${name}`]);
const BACKGROUND_ONLY = {
  dark: ['--terminal-black'],
  light: ['--terminal-white', '--terminal-bright-white'],
};
const SECONDARY_TEXT = ['--muted-foreground', '--subtle-foreground', '--status-seen'];
const SURFACES = ['--background', '--sidebar', '--card'];
/** Text and the band it is drawn on, both of the doorbell's letter rows (docs/UI_BEHAVIOR.md). */
const LETTER = ['--terminal-letter-text', '--terminal-letter-band'];

function channel(byte) {
  const value = byte / 255;
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
}

function luminance(hex) {
  if (!/^#[0-9A-Fa-f]{6}$/.test(hex)) throw new Error(`not a #RRGGBB color: ${hex}`);
  const [r, g, b] = [1, 3, 5].map(at => channel(Number.parseInt(hex.slice(at, at + 2), 16)));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrast(a, b) {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (light + 0.05) / (dark + 0.05);
}

/** The hex a token draws in one color theme and mode, following aliases. */
function resolve(tokens, name, palette, mode) {
  const token = tokens[name];
  if (!token) throw new Error(`No token named ${name}`);
  if (token.type === 'alias') return resolve(tokens, token.value, palette, mode);
  if (token.type !== 'color') throw new Error(`${name} is not a color`);
  const values = palette === 'hide' && token.hide ? token.hide : token;
  return mode === 'dark' ? values.value : values.light;
}

/** Every pair below the minimum, as readable lines. */
export function failures(tokens) {
  const found = [];
  for (const palette of ['neutral', 'hide']) {
    for (const mode of ['dark', 'light']) {
      const at = name => resolve(tokens, name, palette, mode);
      const check = (text, surface) => {
        const ratio = contrast(at(text), at(surface));
        if (ratio < MINIMUM) found.push(`${palette} ${mode}: ${text} ${at(text)} on ${surface} ${at(surface)} is ${ratio.toFixed(2)}:1`);
      };
      for (const text of TERMINAL.filter(name => !BACKGROUND_ONLY[mode].includes(name))) check(text, '--background');
      check(LETTER[0], LETTER[1]);
      for (const text of SECONDARY_TEXT) for (const surface of SURFACES) check(text, surface);
    }
  }
  return found;
}

// A surface, a line and a hover each have to stand apart from what they sit
// on, or a layer reads as the one below it: a card on the page, a line on any
// fill it is drawn on, and a hovered row or control on its resting fill (the
// hover a modal's controls take is its own step). These are luminance steps,
// not text: the floor is that the eye can see an edge. A popover and a modal
// share white in Light and are told apart by --elevation-*, not by fill.
// [layer, the layer under it, Dark minimum, Light minimum]. Light separates its layers with shadows
// as well as fills (DESIGN_GUIDE.md, Tokens at a glance), so its fills may sit closer together.
const LAYER_STEPS = [
  ['--sidebar', '--background', 1.04, 1.03],
  ['--card', '--background', 1.08, 1.04],
  ['--secondary', '--card', 1.08, 1.05],
  ['--accent', '--secondary', 1.06, 1.04],
  ['--dialog-hover', '--dialog-raised', 1.06, 1.04],
  ['--border', '--background', 1.2, 1.2],
  ['--border', '--card', 1.2, 1.15],
  ['--border', '--dialog', 1.2, 1.2],
];

/** Every layer that does not stand apart from the one it sits on, as readable lines. */
export function layerFailures(tokens) {
  const found = [];
  for (const palette of ['neutral', 'hide']) {
    for (const mode of ['dark', 'light']) {
      const at = name => resolve(tokens, name, palette, mode);
      for (const [layer, under, dark, light] of LAYER_STEPS) {
        const minimum = mode === 'dark' ? dark : light;
        const ratio = contrast(at(layer), at(under));
        if (ratio < minimum) found.push(`${palette} ${mode}: ${layer} ${at(layer)} on ${under} ${at(under)} is ${ratio.toFixed(2)}:1, under ${minimum}:1`);
      }
    }
  }
  return found;
}

const invoked = process.argv[1] && process.argv[1].endsWith('check-token-contrast.mjs');
if (invoked) {
  const layers = layerFailures(readTokens(process.cwd()));
  if (layers.length) {
    console.error(`${layers.length} layer step(s) in design/tokens.json do not stand apart:\n`);
    for (const line of layers) console.error(`  ${line}`);
    console.error('\nMove the layer to a palette step further from the one it sits on.');
    process.exit(1);
  }
  const found = failures(readTokens(process.cwd()));
  if (found.length) {
    console.error(`${found.length} text color(s) in design/tokens.json fall below ${MINIMUM}:1:\n`);
    for (const line of found) console.error(`  ${line}`);
    console.error('\nChoose a value that reaches the minimum; a terminal color used only as a background belongs in BACKGROUND_ONLY.');
    process.exit(1);
  }
  console.log(`Text contrast holds: every terminal text color and secondary grey reaches ${MINIMUM}:1, and every surface, line and hover stands apart from the layer under it, in Neutral and Hide, Dark and Light.`);
}
