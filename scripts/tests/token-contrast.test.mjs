import assert from 'node:assert/strict';
import test from 'node:test';
import {contrast, failures, layerFailures, over} from '../check-token-contrast.mjs';

// The expected ratios are the ones measured for hide-color-theme's intake,
// independently of this checker: Tango red, blue, magenta and bright black on
// #101112, and GitHub Light's bright blue on white.
test('the ratio is WCAG contrast of relative luminance', () => {
  assert.equal(contrast('#FFFFFF', '#000000').toFixed(2), '21.00');
  assert.equal(contrast('#CC0000', '#101112').toFixed(2), '3.21');
  assert.equal(contrast('#3465A4', '#101112').toFixed(2), '3.19');
  assert.equal(contrast('#75507B', '#101112').toFixed(2), '2.87');
  assert.equal(contrast('#555753', '#101112').toFixed(2), '2.59');
  assert.equal(contrast('#218BFF', '#FFFFFF').toFixed(2), '3.39');
});

test('a color that is not #RRGGBB is refused rather than compared', () => {
  for (const value of ['#FFF', 'red', '#FFFFFF80']) {
    assert.throws(() => contrast(value, '#000000'), /not a #RRGGBB color/);
  }
});

const color = (value, light, hide) => ({type: 'color', value, light, ...(hide ? {hide} : {})});
const ANSI = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'];

function readable() {
  const tokens = {
    '--background': color('#000000', '#FFFFFF', {value: '#0D191B', light: '#F6F8F8'}),
    '--sidebar': color('#000000', '#FFFFFF'),
    '--card': color('#000000', '#FFFFFF'),
    '--muted-foreground': color('#FFFFFF', '#000000'),
    '--subtle-foreground': {type: 'alias', value: '--muted-foreground'},
    '--status-seen': color('#FFFFFF', '#000000'),
    '--terminal-letter-text': color('#FFFFFF', '#000000'),
    '--terminal-letter-band': color('#000000', '#FFFFFF'),
  };
  for (const name of ANSI) {
    tokens[`--terminal-${name}`] = color('#FFFFFF', '#000000');
    tokens[`--terminal-bright-${name}`] = color('#FFFFFF', '#000000');
  }
  return tokens;
}

test('readable tokens pass, and background-only terminal colors are not text', () => {
  const tokens = readable();
  tokens['--terminal-black'] = color('#000000', '#000000');
  tokens['--terminal-white'] = color('#FFFFFF', '#FFFFFF');
  tokens['--terminal-bright-white'] = color('#FFFFFF', '#FFFFFF');
  assert.deepEqual(failures(tokens), []);
});

test('a Hide value below the minimum fails in Hide alone, named with its theme and mode', () => {
  const tokens = readable();
  tokens['--terminal-blue'] = color('#FFFFFF', '#000000', {value: '#1B3034', light: '#000000'});
  assert.deepEqual(failures(tokens), ['hide dark: --terminal-blue #1B3034 on --background #0D191B is 1.30:1']);
});

test('a secondary grey is checked on the background, the sidebar and a card', () => {
  const tokens = readable();
  tokens['--card'] = color('#000000', '#444444');
  const found = failures(tokens);
  // Three greys (muted, subtle, the Seen mark's) in Neutral and Hide Light.
  assert.equal(found.length, 6, found.join('\n'));
  assert.ok(found.some(line => line.includes('--status-seen')), found.join('\n'));
  assert.ok(found.every(line => line.includes('light') && line.includes('on --card')), found.join('\n'));
});

const fill = (alpha, light) => ({type: 'fill', value: {step: '--white', alpha}, light: {step: '--black', alpha: light}});

function layered() {
  const step = (value, light) => color(value, light);
  return {
    '--white': {type: 'palette', oklch: [100, 0, 0]},
    '--black': {type: 'palette', oklch: [0, 0, 0]},
    '--background': step('#0E0F12', '#FFFFFF'),
    '--sidebar': step('#151619', '#F8F8FA'),
    '--card': step('#1C1D20', '#F3F3F5'),
    '--popover': step('#414246', '#FFFFFF'),
    '--dialog': step('#232427', '#FFFFFF'),
    '--border': step('#393A3E', '#D9DBDD'),
    '--secondary': fill(0.06, 0.045),
    '--accent': fill(0.05, 0.04),
    '--hairline': fill(0.08, 0.07),
    '--keycap-line': fill(0.16, 0.15),
    '--code-block': fill(0.035, 0.025),
  };
}

// White at 6% over #1C1D20, by hand: 28 + 227 * 0.06 = 41.6, 29 + 226 * 0.06 = 42.6, 32 + 223 * 0.06 = 45.4.
test('a see-through fill draws its step composited over the surface under it', () => {
  const tokens = layered();
  assert.equal(over(tokens, '--secondary', 'dark', '#1C1D20'), '#2a2b2d');
  // Black at 4.5% over white: 255 * 0.955 = 243.5.
  assert.equal(over(tokens, '--secondary', 'light', '#FFFFFF'), '#f4f4f4');
});

test('a layer that draws the same step as the one under it fails, named with its floor for that mode', () => {
  const tokens = layered();
  assert.deepEqual(layerFailures(tokens), []);
  tokens['--card'] = {type: 'alias', value: '--background'};
  assert.deepEqual(layerFailures(tokens), [
    'neutral dark: --card on --background: #0E0F12 on #0E0F12 is 1.00:1, under 1.08:1',
    'neutral light: --card on --background: #FFFFFF on #FFFFFF is 1.00:1, under 1.04:1',
    'hide dark: --card on --background: #0E0F12 on #0E0F12 is 1.00:1, under 1.08:1',
    'hide light: --card on --background: #FFFFFF on #FFFFFF is 1.00:1, under 1.04:1',
  ]);
});

test('a see-through fill is checked over every surface it can sit on, each composited on its own', () => {
  const tokens = layered();
  // White at 2% shows over the light grey of a popover but not over the darkest page.
  tokens['--code-block'] = fill(0.02, 0.025);
  const found = layerFailures(tokens);
  assert.ok(found.some(line => line.startsWith('neutral dark: --code-block over --background:')), found.join('\n'));
  assert.ok(!found.some(line => line.includes('over --popover:')), found.join('\n'));
  assert.ok(found.every(line => line.includes('dark: --code-block over')), found.join('\n'));
});

test('a filled control\'s hover is checked laid over its own fill, so a hover fainter than the fill fails', () => {
  const tokens = layered();
  tokens['--accent'] = fill(0.01, 0.04);
  const found = layerFailures(tokens);
  assert.ok(found.some(line => line.startsWith('neutral dark: --accent over --secondary over --card:')), found.join('\n'));
  assert.ok(found.every(line => line.includes('dark')), found.join('\n'));
});

test('the doorbell letter words are checked on their band, per theme and mode', () => {
  const tokens = readable();
  assert.deepEqual(failures(tokens), []);
  tokens['--terminal-letter-band'] = color('#000000', '#FFFFFF', {value: '#777777', light: '#FFFFFF'});
  const found = failures(tokens);
  assert.equal(found.length, 1, found.join('\n'));
  assert.ok(found[0].startsWith('hide dark: --terminal-letter-text #FFFFFF on --terminal-letter-band #777777'), found[0]);
});
