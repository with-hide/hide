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
  for (const name of ['check-token-contrast']) {
    fs.writeFileSync(path.join(scripts, `${name}.mjs`), 'process.exitCode = 0;\n');
  }
  fs.writeFileSync(path.join(scripts, 'check-web-tokens.mjs'), "import {value} from './token-policy.mjs';\nif (value !== 'staged') throw new Error('wrong token policy');\n");
  fs.writeFileSync(path.join(scripts, 'check-agent-logos.mjs'), "import fs from 'node:fs';\nfor (const file of ['hide-kit/src/agents.rs', 'hide-ai/src/registry.rs']) {\n if (fs.readFileSync(file, 'utf8') !== 'staged registry') throw new Error('wrong logo registry');\n}\n");
  for (const name of ['hide-kit', 'hide-ai']) {
    fs.mkdirSync(path.join(root, name, 'src'), {recursive: true});
    fs.writeFileSync(path.join(root, name, 'src', name === 'hide-kit' ? 'agents.rs' : 'registry.rs'), 'staged registry');
  }
  fs.writeFileSync(path.join(scripts, 'token-policy.mjs'), "export {value} from './token-policy-next.mjs';\n");
  const module = path.join(scripts, 'token-policy-next.mjs');
  fs.writeFileSync(module, "export const value = 'staged';\n");
  const git = args => execFileSync('git', args, {cwd: root, encoding: 'utf8'});
  git(['init', '-q']);
  git(['add', 'scripts', 'hide-kit', 'hide-ai']);
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
