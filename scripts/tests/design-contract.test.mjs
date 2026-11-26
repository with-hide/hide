import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {execFileSync, spawnSync} from 'node:child_process';
import test from 'node:test';

test('the public staged gate carries checker imports and logo registries without private boards', t => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'hide-design-contract-'));
  t.after(() => fs.rmSync(root, {recursive: true, force: true}));
  const scripts = path.join(root, 'scripts');
  fs.mkdirSync(scripts);
  fs.copyFileSync(new URL('../check-design-contract.mjs', import.meta.url), path.join(scripts, 'check-design-contract.mjs'));
  for (const name of ['check-token-contrast', 'check-ui-parts']) {
    fs.writeFileSync(path.join(scripts, `${name}.mjs`), 'process.exitCode = 0;\n');
  }
  fs.writeFileSync(path.join(scripts, 'check-web-tokens.mjs'), "import {value} from './token-policy.mjs';\nif (value !== 'staged') throw new Error('wrong token policy');\n");
  fs.writeFileSync(path.join(scripts, 'check-agent-logos.mjs'), "import fs from 'node:fs';\nif (fs.readFileSync('hide-kit/src/agents.rs', 'utf8') !== 'staged registry') throw new Error('wrong logo registry');\n");
  fs.mkdirSync(path.join(root, 'hide-kit', 'src'), {recursive: true});
  fs.writeFileSync(path.join(root, 'hide-kit', 'src', 'agents.rs'), 'staged registry');
  fs.writeFileSync(path.join(scripts, 'token-policy.mjs'), "export {value} from './token-policy-next.mjs';\n");
  const module = path.join(scripts, 'token-policy-next.mjs');
  fs.writeFileSync(module, "export const value = 'staged';\n");
  const git = args => execFileSync('git', args, {cwd: root, encoding: 'utf8'});
  git(['init', '-q']);
  git(['add', 'scripts', 'hide-kit']);
  const index = git(['ls-files', '--stage']);
  fs.writeFileSync(module, "export const value = 'unstaged';\nthrow new Error('unstaged module must not run');\n");

  const run = () => spawnSync(process.execPath, [path.join(scripts, 'check-design-contract.mjs'), '--staged'], {cwd: root, encoding: 'utf8'});
  const result = run();
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.equal(git(['ls-files', '--stage']), index);
  assert.match(fs.readFileSync(module, 'utf8'), /unstaged module must not run/);

  git(['add', 'scripts/token-policy-next.mjs']);
  const refused = run();
  assert.notEqual(refused.status, 0);
  assert.match(refused.stderr, /unstaged module must not run/);
});

test('the web token check refuses an opacity on a see-through fill and takes the fill itself', t => {
  const repo = path.resolve(import.meta.dirname, '../..');
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'hide-web-tokens-'));
  t.after(() => fs.rmSync(root, {recursive: true, force: true}));
  fs.mkdirSync(path.join(root, 'design'));
  fs.copyFileSync(path.join(repo, 'design/tokens.json'), path.join(root, 'design/tokens.json'));
  fs.mkdirSync(path.join(root, 'web/src/generated'), {recursive: true});
  fs.copyFileSync(path.join(repo, 'web/src/tokens.css'), path.join(root, 'web/src/tokens.css'));
  fs.copyFileSync(path.join(repo, 'web/src/generated/accents.ts'), path.join(root, 'web/src/generated/accents.ts'));
  const check = source => {
    fs.writeFileSync(path.join(root, 'web/src/row.tsx'), `export const row = "${source}";\n`);
    return spawnSync(process.execPath, [path.join(repo, 'scripts/check-web-tokens.mjs'), root], {encoding: 'utf8'});
  };
  for (const faded of ['hover:bg-accent/50', 'bg-secondary/40', 'border-hairline/30']) {
    const refused = check(`rounded-xs ${faded}`);
    assert.notEqual(refused.status, 0, faded);
    assert.match(refused.stderr, /opacity on a see-through fill/);
  }
  const taken = check('rounded-xs bg-secondary hover:bg-accent border-hairline bg-warning/15');
  assert.equal(taken.status, 0, taken.stdout + taken.stderr);
});
