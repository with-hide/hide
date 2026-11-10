#!/usr/bin/env node
// The user docs' one check: what CI runs, and what to run before a docs change is pushed.
//   0. the CLI reference reader's own test (cli-reference.test.mjs), over the usage strings in hided/src;
//   1. the guides and feature pages hold each other (check-features.mjs), and writes each feature's
//      "Used in guides" list from the guides' `features:` headers;
//   2. the CLI reference is generated from the source of `hide --help` (hided/src), cross-read against the given
//      `hide`, and contracts/hide-cli.json; it fails on a command with no note and on a note for a gone command
//      (cli-reference.mjs);
//   3. the live parts the pages frame are built (web `build:docs-embed`), then the site (`astro build`),
//      which puts them at /embed/ and writes Pagefind's search index.
// Usage: check-all.mjs [--hide <binary>] [--checks-only]
//   --hide         the second reader of the commands, a `hide` whose `--help` lists them (default: $HIDE_DOCS_BIN, then target/debug/hide)
//   --checks-only  steps 1 and 2 only, for a quick loop; CI runs all three
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const content = join(root, "site/src/content/docs/docs");
const generated = join(root, "site/src/generated");
const args = process.argv.slice(2);
const checksOnly = args.includes("--checks-only");
const hideAt = args.indexOf("--hide");
const hide = hideAt >= 0 ? args[hideAt + 1] : process.env.HIDE_DOCS_BIN || join(root, "target/debug/hide");
if (!hide || !existsSync(hide)) {
  console.error(`check-all: no hide binary at ${hide ?? "(none)"}; build one (bash scripts/verify-cargo.sh build -p hided --bin hide) or pass --hide <binary>`);
  process.exit(2);
}

function run(title, command, commandArgs, options = {}) {
  console.log(`\n== ${title}`);
  const result = spawnSync(command, commandArgs, { cwd: root, stdio: ["ignore", options.capture ? "pipe" : "inherit", "inherit"], encoding: "utf8" });
  if (result.status !== 0) {
    console.error(`check-all: FAILED at "${title}"`);
    process.exit(result.status ?? 1);
  }
  return result.stdout;
}

run("the CLI reference reader finds every command of the real usage strings", "node", ["--test", "scripts/docs/*.test.mjs"]);
mkdirSync(generated, { recursive: true });
run("guides and features hold each other", "node", ["scripts/docs/check-features.mjs", content, "--write", join(generated, "used-in.json")]);

const cli = run("CLI reference from the source of hide --help", "node", ["scripts/docs/cli-reference.mjs", hide, join(root, "contracts/hide-cli.json"), join(root, "site/src/data/cli-notes.json")], { capture: true });
const front = ["---", "title: CLI", "description: Every hide command, with one line and an example each.", "sidebar:", "  order: 1", "---", "", ""].join("\n");
writeFileSync(join(content, "reference/cli.md"), front + cli);

if (!checksOnly) {
  run("build the live parts the pages frame", "bash", ["scripts/verify-web.sh", "web", "build:docs-embed"]);
  run("build the site", "bash", ["scripts/verify-web.sh", "site", "build"]);
}
console.log("\ncheck-all: ok");
