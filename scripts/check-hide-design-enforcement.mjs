import fs from 'node:fs';
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';

// Product-side enforcement stays inside the worktree. The workflow must bind
// the same unified checker and its real regression tests; the commands below
// are deliberately executed so a stale or weakened binding fails closed.
const workflow = fs.readFileSync('.github/workflows/design-contract.yml', 'utf8');
assert(workflow.includes('pull_request:') && workflow.includes('push:'), 'CI must cover review and main');
const workflowCommands = [
  'node scripts/check-design-contract.mjs',
  'node --test scripts/tests/token-contrast.test.mjs',
];
const workflowLines = workflow.split(/\r?\n/).map(line => line.trim());
for (const command of workflowCommands) {
  assert(workflowLines.includes(command), 'Missing CI invocation for ' + command);
}
execFileSync(process.execPath, ['scripts/check-design-contract.mjs'], {stdio: 'inherit'});
for (const suite of ['token-contrast']) {
  execFileSync(process.execPath, ['--test', `scripts/tests/${suite}.test.mjs`], {stdio: 'inherit'});
}
console.log('Product enforcement and CI binding PASS');
