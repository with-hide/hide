import assert from 'node:assert/strict';
import test from 'node:test';
import {contrast, failures} from '../check-token-contrast.mjs';

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

test('the doorbell letter words are checked on their band, per theme and mode', () => {
  const tokens = readable();
  assert.deepEqual(failures(tokens), []);
  tokens['--terminal-letter-band'] = color('#000000', '#FFFFFF', {value: '#777777', light: '#FFFFFF'});
  const found = failures(tokens);
  assert.equal(found.length, 1, found.join('\n'));
  assert.ok(found[0].startsWith('hide dark: --terminal-letter-text #FFFFFF on --terminal-letter-band #777777'), found[0]);
});
