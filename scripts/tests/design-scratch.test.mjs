import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawn, spawnSync, execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {PEN_VERSION} from '../pen-cli.mjs';

const repository = fileURLToPath(new URL('../../', import.meta.url));
// Pen is a third-party process boundary. Tests use a fake executable, never
// replace our own functions, and assert published files and failure behavior.
const fakePen = `#!${process.execPath}
const fs = require('node:fs');
const {spawn} = require('node:child_process');
const args = process.argv.slice(2);
const mode = fs.existsSync('.pen-mode') ? fs.readFileSync('.pen-mode', 'utf8') : '';
if (args[0] === 'version') {
  // pen 0.3.8 boxes an update notice around its answer on every command.
  if (mode === 'banner') console.log('\\u001b[33m  ╭──────╮\\u001b[39m\\n\\u001b[33m  │\\u001b[39m  Update available 0.3.8 → 0.3.9  \\u001b[33m│\\u001b[39m\\n  ╰──────╯\\n');
  console.log(mode === 'old' ? 'pen 0.3.8' : 'pen ${PEN_VERSION}');
  process.exit();
}
const output = args[args.indexOf('--out') + 1];
const library = args[args.indexOf('--library') + 1];
if (args.includes('--prompt') || args.includes('--app') || !library) process.exit(9);
if (mode === 'failure') { fs.writeFileSync(output, 'partial'); console.error('Import refused'); process.exit(2); }
if (mode === 'missing') process.exit();
if (mode === 'empty') { fs.writeFileSync(output, ''); process.exit(); }
if (mode === 'hang') {
  const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], {stdio: 'ignore'});
  // Publish readiness only after the complete receipt is visible to the reader.
  fs.writeFileSync('.pen-child.pending', JSON.stringify({cli: process.pid, child: child.pid}));
  fs.renameSync('.pen-child.pending', '.pen-child');
  setInterval(() => {}, 1000);
} else {
  fs.writeFileSync(output, 'linked: ' + library + '\\n' + fs.readFileSync(library, 'utf8'));
  console.log('Imported library');
}
`;

function git(root, args) {
  return execFileSync('git', args, {cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe']});
}
function fixture(t) {
  const directory = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'hide-scratch-test-')));
  t.after(() => fs.rmSync(directory, {recursive: true, force: true}));
  const root = path.join(directory, 'checkout with spaces');
  fs.mkdirSync(root);
  fs.mkdirSync(path.join(root, 'scripts'));
  fs.mkdirSync(path.join(root, 'design'));
  fs.mkdirSync(path.join(directory, 'bin'));
  for (const name of ['design-scratch.mjs', 'pen-cli.mjs', 'pen-tokens.mjs', 'gen-tokens.mjs']) fs.copyFileSync(path.join(repository, 'scripts', name), path.join(root, 'scripts', name));
  fs.writeFileSync(path.join(root, '.gitignore'), '/agents/\n.pen-*\n');
  fs.writeFileSync(path.join(root, 'design/hide-ui.lib.pen'), 'fixture library A');
  fs.writeFileSync(path.join(directory, 'bin/pen'), fakePen, {mode: 0o755});
  git(root, ['init', '--quiet']);
  const env = {...process.env, PATH: path.join(directory, 'bin') + path.delimiter + process.env.PATH};
  return {root, directory, env};
}
const scratch = (root, slug = 'test') => path.join(root, 'agents/runs', slug, 'design/scratch.pen');
function run(f, args = ['test'], root = f.root) {
  return spawnSync(process.execPath, [path.join(root, 'scripts/design-scratch.mjs'), ...args], {cwd: root, env: f.env, encoding: 'utf8', timeout: 10_000});
}
function mode(f, value) { fs.writeFileSync(path.join(f.root, '.pen-mode'), value); }

test('creates an ignored linked scratch, preserves library and refuses overwrite', t => {
  const f = fixture(t);
  const result = run(f);
  assert.equal(result.status, 0, result.stderr);
  const file = scratch(f.root), original = fs.readFileSync(file, 'utf8');
  assert.equal(original, `linked: ${path.join(f.root, 'design/hide-ui.lib.pen')}\nfixture library A`);
  assert.equal(fs.readFileSync(path.join(f.root, 'design/hide-ui.lib.pen'), 'utf8'), 'fixture library A');
  assert.equal(git(f.root, ['check-ignore', '--', file]).trim(), file);
  assert.ok(result.stdout.includes(`Scratch: ${file}`));
  assert.deepEqual(fs.readdirSync(path.dirname(file)), ['scratch.pen']);
  const again = run(f);
  assert.notEqual(again.status, 0);
  assert.match(again.stderr, /already exists/);
  assert.equal(fs.readFileSync(file, 'utf8'), original);
});

test('two worktrees using the same slug keep their own library and scratch', async t => {
  const f = fixture(t), other = path.join(f.directory, 'other worktree');
  git(f.root, ['add', '.']);
  git(f.root, ['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', '-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'Fixture']);
  git(f.root, ['worktree', 'add', '--quiet', '--detach', other]);
  fs.writeFileSync(path.join(other, 'design/hide-ui.lib.pen'), 'fixture library B');
  const launch = root => new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ['scripts/design-scratch.mjs', 'same-task'], {cwd: root, env: f.env, stdio: 'ignore'});
    child.on('error', reject);
    child.on('close', code => resolve(code));
  });
  assert.deepEqual(await Promise.all([launch(f.root), launch(other)]), [0, 0]);
  for (const [root, letter] of [[f.root, 'A'], [other, 'B']]) {
    assert.equal(fs.readFileSync(scratch(root, 'same-task'), 'utf8'), `linked: ${path.join(root, 'design/hide-ui.lib.pen')}\nfixture library ${letter}`);
  }
});

test('two creators racing for the same scratch publish exactly one complete file', async t => {
  const f = fixture(t);
  const launch = () => new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ['scripts/design-scratch.mjs', 'test'], {cwd: f.root, env: f.env, stdio: 'ignore'});
    child.on('error', reject);
    child.on('close', code => resolve(code));
  });
  assert.deepEqual((await Promise.all([launch(), launch()])).sort(), [0, 1]);
  assert.equal(fs.readFileSync(scratch(f.root), 'utf8'), `linked: ${path.join(f.root, 'design/hide-ui.lib.pen')}\nfixture library A`);
  assert.deepEqual(fs.readdirSync(path.dirname(scratch(f.root))), ['scratch.pen']);
});

test('rejects unsafe names, symlinked directories and library aliases', t => {
  const f = fixture(t);
  for (const args of [[], ['../escape'], ['/absolute'], ['a/b'], ['a', 'b'], ['A'], ['a'.repeat(81)]]) assert.notEqual(run(f, args).status, 0);
  assert.equal(fs.existsSync(path.join(f.root, 'agents')), false);
  const outside = path.join(f.directory, 'outside');
  fs.mkdirSync(outside);
  fs.symlinkSync(outside, path.join(f.root, 'agents'));
  assert.notEqual(run(f).status, 0);
  assert.deepEqual(fs.readdirSync(outside), []);
  fs.unlinkSync(path.join(f.root, 'agents'));
  fs.renameSync(path.join(f.root, 'design'), path.join(f.root, 'real-design'));
  fs.symlinkSync('real-design', path.join(f.root, 'design'));
  assert.match(run(f).stderr, /library linked outside/);
  assert.equal(fs.existsSync(path.join(f.root, 'agents')), false);
});

test('unignored targets and unsupported CLI versions fail before creation', t => {
  const f = fixture(t);
  fs.writeFileSync(path.join(f.root, '.gitignore'), '');
  assert.notEqual(run(f).status, 0);
  assert.equal(fs.existsSync(path.join(f.root, 'agents')), false);
  fs.writeFileSync(path.join(f.root, '.gitignore'), '/agents/\n');
  mode(f, 'old');
  assert.ok(run(f).stderr.includes(`Expected pen ${PEN_VERSION}`));
  assert.equal(fs.existsSync(path.join(f.root, 'agents')), false);
});

test('an update notice around the version answer is not a different version', t => {
  const f = fixture(t);
  mode(f, 'banner');
  const result = run(f);
  assert.equal(result.status, 0, result.stderr);
  assert.ok(fs.existsSync(scratch(f.root)));
});

test('failed or empty imports leave no published or partial scratch', t => {
  const f = fixture(t);
  for (const value of ['failure', 'missing', 'empty']) {
    mode(f, value);
    const result = run(f);
    assert.notEqual(result.status, 0);
    assert.equal(fs.existsSync(scratch(f.root)), false);
    assert.deepEqual(fs.readdirSync(path.dirname(scratch(f.root))), []);
  }
});

test('a dangling target symlink is not overwritten', t => {
  const f = fixture(t), file = scratch(f.root);
  fs.mkdirSync(path.dirname(file), {recursive: true});
  fs.symlinkSync('missing.pen', file);
  assert.notEqual(run(f).status, 0);
  assert.equal(fs.readlinkSync(file), 'missing.pen');
  assert.deepEqual(fs.readdirSync(path.dirname(file)), ['scratch.pen']);
});

for (const signal of ['SIGINT', 'SIGKILL']) test(`${signal} ends the CLI and its child and publishes nothing`, async t => {
  const f = fixture(t);
  mode(f, 'hang');
  const child = spawn(process.execPath, ['scripts/design-scratch.mjs', 'test'], {cwd: f.root, env: f.env, stdio: 'ignore'});
  let pids;
  t.after(() => {
    child.kill('SIGKILL');
    if (pids) for (const pid of [pids.cli, pids.child]) { try { process.kill(pid, 'SIGKILL'); } catch {} }
  });
  const closed = new Promise(resolve => child.on('close', resolve));
  const deadline = Date.now() + 5000;
  while (!fs.existsSync(path.join(f.root, '.pen-child')) && Date.now() < deadline) await new Promise(resolve => setTimeout(resolve, 20));
  pids = JSON.parse(fs.readFileSync(path.join(f.root, '.pen-child'), 'utf8'));
  const guardPid = Number(spawnSync('ps', ['-o', 'ppid=', '-p', String(pids.cli)], {encoding: 'utf8'}).stdout.trim());
  child.kill(signal);
  assert.notEqual(await closed, 0);
  // POSIX may expose a killed descendant briefly before its new parent reaps it.
  for (const pid of [guardPid, pids.cli, pids.child]) {
    const deadline = Date.now() + 3000;
    let running = true;
    while (running && Date.now() < deadline) {
      const status = spawnSync('ps', ['-o', 'stat=', '-p', String(pid)], {encoding: 'utf8'}).stdout.trim();
      running = status !== '' && !status.startsWith('Z');
      if (running) await new Promise(resolve => setTimeout(resolve, 20));
    }
    assert.equal(running, false, `test descendant ${pid} survived cancellation`);
  }
  assert.equal(fs.existsSync(scratch(f.root)), false);
  assert.deepEqual(fs.readdirSync(path.dirname(scratch(f.root))), []);
});
