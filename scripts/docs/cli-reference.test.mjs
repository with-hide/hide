// The CLI reference's reader against the usage strings `hide --help` is made of, taken from the source
// files that own them, never retyped: a command the reader fails to find would need no note, and the
// reference would pass while missing it. Run by `check-all.mjs`: `node --test scripts/docs/*.test.mjs`.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, cpSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { commandsOf, expand, noteProblems, sourceHelp } from "./cli-reference.mjs";

const root = fileURLToPath(new URL("../..", import.meta.url));
const source = (path) => readFileSync(`${root}/${path}`, "utf8");

/** The value of `const NAME: &str = "...";` in a Rust file, with its string escapes read. */
function constant(path, name) {
  const match = source(path).match(new RegExp(`const ${name}: &str = "((?:[^"\\\\]|\\\\.)*)";`));
  assert.ok(match, `${name} not found in ${path}`);
  return JSON.parse(`"${match[1]}"`);
}
/** One usage string as `hide --help` prints it: `usage: ` dropped, each line indented by two spaces. */
const asHelp = (usage) => usage.replace(/^usage: /, "").split("\n").map((line) => `  ${line}`).join("\n");

const links = constant("hided/src/cli.rs", "LINKS_USAGE");
const memory = constant("hided/src/cli.rs", "MEMORY_USAGE");
const notes = JSON.parse(source("site/src/data/cli-notes.json"));

test("a `|` between whole commands is one command each, with its own arguments", () => {
  const found = [...commandsOf(asHelp(memory)).keys()];
  for (const name of ["show", "search", "add", "forget", "undo", "placed", "building", "unbuilt", "outdated", "skipped", "stale", "release", "recheck"]) {
    assert.ok(found.includes(`memory ${name}`), `memory ${name} not found in: ${memory}`);
  }
  const found2 = [...commandsOf(asHelp(links)).keys()];
  for (const name of ["pr", "issue", "branch", "session"]) assert.ok(found2.includes(`links ${name}`), `links ${name} not found in: ${links}`);
});

test("alternatives inside one word are one command each, and a `|` inside brackets splits nothing", () => {
  assert.deepEqual(expand("hide request ack|cancel|show <id>"), ["request ack", "request cancel", "request show"]);
  assert.deepEqual(expand("hide request send <target> --intent <key> [--kind request|block] [--now | --force]"), ["request send"]);
  assert.deepEqual(expand("hide task set (--file <plan.json> | --json <plan>) [--ask [why]]"), ["task set"]);
  assert.deepEqual(expand("hide <path>, hide ."), []);
});

// The two usages are a part of the whole help, so the notes of every other command look "gone" here.
const noNote = (commands, given) => noteProblems({ source: commands }, given).filter((problem) => problem.startsWith("commands with no note"));

test("every command of those usages has a note, and a deleted note fails the check", () => {
  const commands = new Map([...commandsOf(asHelp(memory)), ...commandsOf(asHelp(links))]);
  assert.deepEqual(noNote(commands, notes), []);
  for (const name of ["memory search", "memory stale", "links issue", "links session"]) {
    const { [name]: _removed, ...without } = notes;
    const problems = noNote(commands, without);
    assert.equal(problems.length, 1, name);
    assert.match(problems[0], new RegExp(`commands with no note.*${name}`));
  }
  assert.match(noteProblems({ source: commands }, { ...notes, "memory teleport": { line: "x" } }).join("\n"), /gone.*memory teleport/);
});

test("the source is the authority: every command help_text() joins has a note, and none is left over", () => {
  const fromSource = commandsOf(sourceHelp(root));
  assert.ok(fromSource.size > 100, `only ${fromSource.size} commands read from the source`);
  assert.deepEqual(noteProblems({ source: fromSource }, notes), []);
  for (const name of ["memory search", "links issue", "mission gate", "session read", "browser help", "workspace info", "status"]) {
    assert.ok(fromSource.has(name), `${name} not read from the source`);
  }
});

test("a command added to the source usage fails check-all's reader although the installed hide does not list it", () => {
  // A copy of the source files help_text() reads, with one new command in session_cli.rs's usage.
  const copy = mkdtempSync(`${tmpdir()}/cli-reference-`);
  cpSync(`${root}/hided/src`, `${copy}/hided/src`, { recursive: true });
  const usage = `${copy}/hided/src/session_cli.rs`;
  const original = readFileSync(usage, "utf8");
  const changed = original.replace(/(pub const USAGE: &str = "hide session list)/, '$1 [--json]\\nhide session teleport <id>\\nhide session list');
  assert.notEqual(changed, original, "the usage constant moved; update this test's edit");
  writeFileSync(usage, changed);
  // The "installed" hide prints what the unmodified source prints: it is older than the copy.
  const fake = `${copy}/hide`;
  writeFileSync(`${copy}/help.txt`, sourceHelp(root));
  writeFileSync(fake, `#!/bin/sh\ncat ${copy}/help.txt\n`);
  chmodSync(fake, 0o755);
  const run = (sourceRoot) => spawnSync("node", [`${root}/scripts/docs/cli-reference.mjs`, fake, `${root}/contracts/hide-cli.json`, `${root}/site/src/data/cli-notes.json`], { encoding: "utf8", env: { ...process.env, HIDE_DOCS_SOURCE_ROOT: sourceRoot } });
  const changedRun = run(copy);
  assert.equal(changedRun.status, 1, changedRun.stderr);
  assert.match(changedRun.stderr, /commands with no note.*session teleport \[source\]/);
  assert.doesNotMatch(changedRun.stderr, /hide --help\]/);
  // The unmodified source passes with the same fake binary.
  assert.equal(run(root).status, 0);
});
