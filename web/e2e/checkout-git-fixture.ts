// Private repository and provider answers shared by browser and native checks.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import type { HerdrFixture } from "./herdr-fixture";
import { fixtureProgram } from "./platform-fixture";
import { git } from "./work-prs-fixture";

export function checkoutGitFixture(herdr: HerdrFixture) {
  const root = fs.realpathSync(path.join(herdr.root, "fixture"));
  git(root, ["init"]);
  fs.writeFileSync(path.join(root, "tracked.txt"), "original\n");
  git(root, ["add", "."]);
  git(root, ["commit", "-m", "한글 commit subject on main"]);
  git(root, ["remote", "add", "origin", "https://github.com/example/header-fixture.git"]);
  const sha = git(root, ["rev-parse", "HEAD"]).trim();
  const log = path.join(herdr.root, "header-gh.jsonl");
  const executable = fixtureProgram(path.join(herdr.root, "bin"), "gh", `
const fs = require('node:fs');
const args = process.argv.slice(2);
if (args[0] === '--version') { console.log('gh fixture'); process.exit(0); }
fs.appendFileSync(${JSON.stringify(log)}, JSON.stringify(args)+'\\n');
const endpoint = args.at(-1);
const match = /^repos\\/example\\/header-fixture\\/commits\\/([a-f0-9]{40})\\/(check-runs\\?per_page=100|status)$/.exec(endpoint);
if (args[0] !== 'api' || args[1] !== '--jq' || !args[2].startsWith('{state:') || !match) process.exit(1);
// A successful check run and a failing commit status must produce failure.
// A subsequent HEAD gets no checks, never the old HEAD's result.
console.log(JSON.stringify({state: match[1] !== ${JSON.stringify(sha)} ? 'none' : match[2] === 'status' ? 'failed' : 'passing', url: null}));
`);
  // Warm the fixture executable before the product's bounded read starts.
  execFileSync(executable, ["--version"]);
  return { root, sha, log };
}
