import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import test from 'node:test';

const checker = new URL('../check-ui-parts.mjs', import.meta.url).pathname;

function fixture(t, files, baseline) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'hide-ui-parts-'));
  t.after(() => fs.rmSync(root, {recursive: true, force: true}));
  for (const [file, text] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(root, file)), {recursive: true});
    fs.writeFileSync(path.join(root, file), text);
  }
  fs.mkdirSync(path.join(root, 'scripts'), {recursive: true});
  fs.writeFileSync(path.join(root, 'scripts/ui-parts-baseline.json'), JSON.stringify(baseline));
  const run = (...args) => spawnSync(process.execPath, [checker, ...args, root], {encoding: 'utf8'});
  const write = (file, text) => fs.writeFileSync(path.join(root, file), text);
  const remove = file => fs.rmSync(path.join(root, file));
  const saved = () => JSON.parse(fs.readFileSync(path.join(root, 'scripts/ui-parts-baseline.json'), 'utf8'));
  return {run, write, remove, saved};
}

const twoButtons = '<button>a</button><button>b</button>';

test('a file at its baseline passes and the parts, tests and selector strings are not counted', t => {
  const {run} = fixture(t, {
    'web/src/A.tsx': `${twoButtons} const s = '[role="menu"]';`,
    'web/src/components/ui/button.tsx': '<button />',
    'web/src/A.test.tsx': '<button />',
  }, {'web/src/A.tsx': {button: 2}});
  const result = run();
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /2 button, 0 input, 0 select, 0 textarea, 0 layer/);
});

test('a new raw control fails and names the part to use', t => {
  const {run} = fixture(t, {'web/src/A.tsx': twoButtons, 'web/src/B.tsx': '<div role="dialog" />'}, {'web/src/A.tsx': {button: 2}});
  const result = run();
  assert.equal(result.status, 1);
  assert.match(result.stderr, /web\/src\/B\.tsx: 1 raw layer \(baseline 0\); open it through `Dialog`/);
});

test('a lower count asks for --update, which locks it in', t => {
  const {run, write, saved} = fixture(t, {'web/src/A.tsx': twoButtons}, {'web/src/A.tsx': {button: 2}});
  write('web/src/A.tsx', '<button>a</button>');
  const asked = run();
  assert.equal(asked.status, 1);
  assert.match(asked.stderr, /--update/);
  assert.equal(run('--update').status, 0);
  assert.deepEqual(saved(), {'web/src/A.tsx': {button: 1}});
  assert.equal(run().status, 0);
});

test('--update carries a moved file but refuses a kind whose total grew', t => {
  const {run, write, remove, saved} = fixture(t, {'web/src/A.tsx': twoButtons}, {'web/src/A.tsx': {button: 2}});
  remove('web/src/A.tsx');
  write('web/src/B.tsx', twoButtons);
  assert.equal(run('--update').status, 0);
  assert.deepEqual(saved(), {'web/src/B.tsx': {button: 2}});
  write('web/src/B.tsx', `${twoButtons}<button>c</button>`);
  const refused = run('--update');
  assert.equal(refused.status, 1);
  assert.match(refused.stderr, /raw button grew from 2 to 3/);
  assert.deepEqual(saved(), {'web/src/B.tsx': {button: 2}});
});
