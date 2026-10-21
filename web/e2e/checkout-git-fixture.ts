// Private repository and provider answers shared by browser and native checks.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import type { HerdrFixture } from "./herdr-fixture";
import { fixtureProgram } from "./platform-fixture";
import { git } from "./work-prs-fixture";

export function checkoutGitFixture(herdr: HerdrFixture, { divergedUpstream = false, historyCount = 1 } = {}) {
  const root = fs.realpathSync(path.join(herdr.root, "fixture"));
  git(root, ["init"]);
  fs.writeFileSync(path.join(root, "tracked.txt"), "original\n");
  git(root, ["add", "."]);
  git(root, ["commit", "-m", "한글 commit subject on main"]);
  for (let index = 1; index < historyCount; index += 1) git(root, ["commit", "--allow-empty", "-m", `기록 ${index} mixed English commit`]);
  git(root, ["remote", "add", "origin", "https://github.com/example/header-fixture.git"]);
  if (divergedUpstream) {
    const base = git(root, ["rev-parse", "HEAD"]).trim();
    const tree = git(root, ["rev-parse", "HEAD^{tree}"]).trim();
    const remote = git(root, ["commit-tree", tree, "-p", base, "-m", "remote commit"]).trim();
    git(root, ["update-ref", "refs/remotes/origin/main", remote]);
    git(root, ["branch", "--set-upstream-to", "origin/main"]);
    git(root, ["commit", "--allow-empty", "-m", "한글 local commit and English"]);
  }
  const sha = git(root, ["rev-parse", "HEAD"]).trim();
  const log = path.join(herdr.root, "header-gh.jsonl");
  const executable = fixtureProgram(path.join(herdr.root, "bin"), "gh", `
const fs = require('node:fs');
const args = process.argv.slice(2);
if (args[0] === '--version') { console.log('gh fixture'); process.exit(0); }
fs.appendFileSync(${JSON.stringify(log)}, JSON.stringify(args)+'\\n');
const query = args.at(-1);
if (args[0] !== 'api' || args[1] !== 'graphql' || !query.startsWith('query=query HideCheckoutChecks ')) process.exit(1);
const repository = {};
for (const match of query.matchAll(/c(\\d+): object\\(oid:"([a-f0-9]{40})"\\)/g)) {
  const oid = match[2];
  repository['c' + match[1]] = { oid, url: 'https://github.com/example/header-fixture/commit/' + oid, statusCheckRollup: oid === ${JSON.stringify(sha)} ? { state: 'FAILURE' } : null };
}
console.log(JSON.stringify({data: {repository}}));
`);
  // Warm the fixture executable before the product's bounded read starts.
  execFileSync(executable, ["--version"]);
  return { root, sha, log };
}
