import assert from 'node:assert/strict';
import test from 'node:test';
import {contrast, failures, layerFailures} from '../check-token-contrast.mjs';

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

test('a layer that draws the same step as the one under it fails, named with its floor for that mode', () => {
  const step = (value, light) => color(value, light);
  const tokens = {
    '--background': step('#0E0F12', '#FFFFFF'),
    '--sidebar': step('#151619', '#F8F8FA'),
    '--card': step('#1C1D20', '#F3F3F5'),
    '--secondary': step('#2A2B2E', '#E8E9EC'),
    '--accent': step('#343539', '#E0E1E4'),
    '--dialog': step('#232427', '#FFFFFF'),
    '--dialog-raised': step('#2F3033', '#E8E9EC'),
    '--dialog-hover': step('#3C3D40', '#D9DBDD'),
    '--border': step('#393A3E', '#D9DBDD'),
  };
  assert.deepEqual(layerFailures(tokens), []);
  tokens['--accent'] = {type: 'alias', value: '--secondary'};
  assert.deepEqual(layerFailures(tokens), [
    'neutral dark: --accent #2A2B2E on --secondary #2A2B2E is 1.00:1, under 1.06:1',
    'neutral light: --accent #E8E9EC on --secondary #E8E9EC is 1.00:1, under 1.04:1',
    'hide dark: --accent #2A2B2E on --secondary #2A2B2E is 1.00:1, under 1.06:1',
    'hide light: --accent #E8E9EC on --secondary #E8E9EC is 1.00:1, under 1.04:1',
  ]);
});
