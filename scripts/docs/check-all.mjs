#!/usr/bin/env node
// The user docs' one check: what CI runs, and what to run before a docs change is pushed.
//   0. the tests beside this file: the CLI reference reader's (cli-reference.test.mjs: how a help line expands to
//      commands, and that a binary whose help lists a command with no note fails), the shortcuts page's, and the
//      head script's (embed-script.test.mjs: the real script run against a stand-in page, so a frame's theme repair
//      cannot throw, loop or stop repairing);
//   1. the guides and feature pages hold each other (check-features.mjs), and writes each feature's
//      "Used in guides" list from the guides' `features:` headers;
//   1b. the site's own tests (`site/tests`: the landing's download policy, the privacy and support pages' claims held
//      to the code and the deploy files), through `verify-web.sh site test`; they read files and need no install;
//   2. the CLI reference is generated from the help of the one `hide` named below and contracts/hide-cli.json, and fails
//      on a command with no note and on a note for a gone command (cli-reference.mjs);
//   2b. the Keyboard shortcuts page is generated from web/src/shortcuts.ts, imported as it stands, and fails when a
//      command of the registry has no row or lacks its keys on a system (shortcuts-reference.mjs);
//   3. the live parts the pages frame are built (web `build:docs-embed`), then the site (`astro build`),
//      which puts them at /embed/ and writes Pagefind's search index. Between the two, check-still.mjs loads
//      every part in Chromium and fails unless it is byte-identical 120 s apart under reduced motion, and
//      check-map-fits.mjs frames the mission map in a stand-in page running the real head script and fails when
//      content is hidden or blank space is left under it, or the height does not settle, at 1440 and 390.
// Usage: check-all.mjs (--hide <binary> | HIDE_BIN=<binary>) [--ci] [--checks-only]
//   --hide / HIDE_BIN  the `hide` whose `--help` is the one authority for which commands exist (missing notes, notes for
//                      gone commands, the generated cli.md). It must be built from THIS commit: CI builds it in the job
//                      and passes its path here.
//   --ci               (or CI=true) a missing binary is a hard failure before anything is generated; there is no default
//                      and no fallback to an installed hide or the PATH.
//   --checks-only      steps 0 to 2b only, for a quick loop; CI runs all of them
// Locally, without --ci, the default is target/debug/hide, and an installed app's hide is accepted but is a limited
// check: it may be older than this commit, and CI is the gate.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const content = join(root, "site/src/content/docs/docs");
const generated = join(root, "site/src/generated");
const args = process.argv.slice(2);
const checksOnly = args.includes("--checks-only");
const ci = args.includes("--ci") || ["true", "1"].includes(process.env.CI ?? "");
const hideAt = args.indexOf("--hide");
const named = hideAt >= 0 ? args[hideAt + 1] : process.env.HIDE_BIN;
const hide = named || (ci ? undefined : join(root, "target/debug/hide"));
if (!hide || !existsSync(hide)) {
  console.error(`check-all: no hide binary${hide ? ` at ${hide}` : ""}${ci ? " (CI mode: no default, no fallback to an installed hide)" : ""}; build one (bash scripts/verify-cargo.sh build -p hided --bin hide) and pass --hide <binary> or HIDE_BIN`);
  process.exit(2);
}
if (relative(join(root, "target"), resolve(hide)).startsWith("..")) {
  console.error(`check-all: ${hide} is not a build of this checkout (not under target/): installed hide may be older than this commit; CI reads the build of this commit`);
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
mkdirSync(join(content, "reference"), { recursive: true });
run("guides and features hold each other", "node", ["scripts/docs/check-features.mjs", content, "--write", join(generated, "used-in.json")]);

run("the site's own tests: the download policy and the legal pages", "bash", ["scripts/verify-web.sh", "site", "test"]);

const cli = run("CLI reference from hide --help", "node", ["scripts/docs/cli-reference.mjs", hide, join(root, "contracts/hide-cli.json"), join(root, "site/src/data/cli-notes.json")], { capture: true });
const front = ["---", "title: CLI", "description: Every hide command, with one line and an example each.", "sidebar:", "  order: 1", "---", "", ""].join("\n");
writeFileSync(join(content, "reference/cli.md"), front + cli);

const shortcuts = run("Keyboard shortcuts page from web/src/shortcuts.ts", "node", ["scripts/docs/shortcuts-reference.mjs"], { capture: true });
writeFileSync(join(content, "reference/shortcuts.md"), shortcuts);

if (!checksOnly) {
  run("build the live parts the pages frame", "bash", ["scripts/verify-web.sh", "web", "build:docs-embed"]);
  run("every live part holds one still picture under reduced motion", "node", ["web/src/docs/check-still.mjs"]);
  run("the mission map is shown whole in its frame at the widths the pages give it", "node", ["web/src/docs/check-map-fits.mjs"]);
  run("build the site", "bash", ["scripts/verify-web.sh", "site", "build"]);
}
console.log("\ncheck-all: ok");
