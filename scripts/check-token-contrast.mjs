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
// A see-through fill is compared as it draws: composited over each surface it
// can sit on (`over`), never as its own color.

import {paletteHex, readTokens} from './gen-tokens.mjs';

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

/** The hex a token draws in one color theme and mode, following aliases. A see-through fill has no hex of its own: composite it with `over`. */
export function resolve(tokens, name, palette, mode) {
  const token = tokens[name];
  if (!token) throw new Error(`No token named ${name}`);
  if (token.type === 'alias') return resolve(tokens, token.value, palette, mode);
  if (token.type === 'fill') throw new Error(`${name} is a see-through fill; composite it over a surface`);
  if (token.type !== 'color') throw new Error(`${name} is not a color`);
  const values = palette === 'hide' && token.hide ? token.hide : token;
  return mode === 'dark' ? values.value : values.light;
}

function rgb(hex) {
  if (!/^#[0-9A-Fa-f]{6}$/.test(hex)) throw new Error(`not a #RRGGBB color: ${hex}`);
  return [1, 3, 5].map(at => Number.parseInt(hex.slice(at, at + 2), 16));
}

/** The hex a see-through fill draws laid over `under`, as the browser composites it (source-over in sRGB). */
export function over(tokens, name, mode, under) {
  const token = tokens[name];
  if (token?.type !== 'fill') throw new Error(`${name} is not a see-through fill`);
  const {step, alpha} = mode === 'dark' ? token.value : token.light;
  const top = rgb(paletteHex(tokens, step));
  return `#${rgb(under).map((byte, at) => Math.round(top[at] * alpha + byte * (1 - alpha)).toString(16).padStart(2, '0')).join('')}`;
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
// on, or a layer reads as the one below it: a card on the page and a line on
// any fill it is drawn on. These are luminance steps, not text: the floor is
// that the eye can see an edge. A popover and a modal share white in Light and
// are told apart by --elevation-*, not by fill.
// [layer, the layer under it, Dark minimum, Light minimum]. Light separates its layers with shadows
// as well as fills (DESIGN_GUIDE.md, Tokens at a glance), so its fills may sit closer together.
const LAYER_STEPS = [
  ['--sidebar', '--background', 1.04, 1.03],
  ['--card', '--background', 1.08, 1.04],
  ['--border', '--background', 1.2, 1.2],
  ['--border', '--card', 1.2, 1.15],
  ['--border', '--dialog', 1.2, 1.2],
];

// A see-through fill (a `fill` token) sits on whatever surface is under it, so
// it is composited over each surface a control or a row can sit on and has to
// stand apart from every one of them. [fill, Dark minimum, Light minimum].
const FILL_SURFACES = ['--background', '--sidebar', '--card', '--popover', '--dialog'];
const FILL_STEPS = [
  ['--secondary', 1.12, 1.08],
  ['--accent', 1.1, 1.07],
  ['--code-block', 1.05, 1.04],
  ['--keycap-line', 1.4, 1.3],
];
// A fill laid over another fill: a filled control's hover is --accent over its
// --secondary (`fill-hovered`), and the hairline is the edge of a filled control
// or a code block. [top, the fill under it, Dark minimum, Light minimum].
const STACKED_STEPS = [
  ['--accent', '--secondary', 1.12, 1.07],
  ['--hairline', '--secondary', 1.2, 1.12],
  ['--hairline', '--code-block', 1.2, 1.12],
];

/** Every layer that does not stand apart from the one it sits on, as readable lines. */
export function layerFailures(tokens) {
  const found = [];
  for (const palette of ['neutral', 'hide']) {
    for (const mode of ['dark', 'light']) {
      const at = name => resolve(tokens, name, palette, mode);
      const floor = (dark, light) => (mode === 'dark' ? dark : light);
      const check = (label, top, under, minimum) => {
        const ratio = contrast(top, under);
        if (ratio < minimum) found.push(`${palette} ${mode}: ${label} ${top} on ${under} is ${ratio.toFixed(2)}:1, under ${minimum}:1`);
      };
      for (const [layer, under, dark, light] of LAYER_STEPS) {
        check(`${layer} on ${under}:`, at(layer), at(under), floor(dark, light));
      }
      for (const surface of FILL_SURFACES) {
        for (const [fill, dark, light] of FILL_STEPS) {
          check(`${fill} over ${surface}:`, over(tokens, fill, mode, at(surface)), at(surface), floor(dark, light));
        }
        for (const [top, fill, dark, light] of STACKED_STEPS) {
          const under = over(tokens, fill, mode, at(surface));
          check(`${top} over ${fill} over ${surface}:`, over(tokens, top, mode, under), under, floor(dark, light));
        }
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
    console.error('\nMove the layer to a palette step further from the one it sits on, or give a see-through fill more alpha.');
    process.exit(1);
  }
  const found = failures(readTokens(process.cwd()));
  if (found.length) {
    console.error(`${found.length} text color(s) in design/tokens.json fall below ${MINIMUM}:1:\n`);
    for (const line of found) console.error(`  ${line}`);
    console.error('\nChoose a value that reaches the minimum; a terminal color used only as a background belongs in BACKGROUND_ONLY.');
    process.exit(1);
  }
  console.log(`Text contrast holds: every terminal text color and secondary grey reaches ${MINIMUM}:1, and every surface, see-through fill, line and hover stands apart from what it sits on, in Neutral and Hide, Dark and Light.`);
}
