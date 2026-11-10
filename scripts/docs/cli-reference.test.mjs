// The CLI reference's reader: how a help line expands to commands (checked on the real usage strings, taken from the
// source files that own them, never retyped: a command the reader fails to find would need no note and the reference
// would pass while missing it), and what a `hide` binary's help must hold to pass. Run by `check-all.mjs`: `node --test scripts/docs/*.test.mjs`.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { commandsOf, expand, noteProblems } from "./cli-reference.mjs";

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
const noNote = (commands, given) => noteProblems({ "hide": commands }, given).filter((problem) => problem.startsWith("commands with no note"));

test("every command of those usages has a note, and a deleted note fails the check", () => {
  const commands = new Map([...commandsOf(asHelp(memory)), ...commandsOf(asHelp(links))]);
  assert.deepEqual(noNote(commands, notes), []);
  for (const name of ["memory search", "memory stale", "links issue", "links session"]) {
    const { [name]: _removed, ...without } = notes;
    const problems = noNote(commands, without);
    assert.equal(problems.length, 1, name);
    assert.match(problems[0], new RegExp(`commands with no note.*${name}`));
  }
  assert.match(noteProblems({ "hide": commands }, { ...notes, "memory teleport": { line: "x" } }).join("\n"), /gone.*memory teleport/);
});

// A stand-in `hide` whose `--help` prints the given lines.
function fakeHide(lines) {
  const dir = mkdtempSync(`${tmpdir()}/cli-reference-`);
  writeFileSync(`${dir}/help.txt`, `usage: hide [command]\n\nFor you\n${lines.map((line) => `  ${line}`).join("\n")}\n`);
  const path = `${dir}/hide`;
  writeFileSync(path, `#!/bin/sh\ncat ${dir}/help.txt\n`);
  chmodSync(path, 0o755);
  return path;
}
const generate = (hide) => spawnSync("node", [`${root}/scripts/docs/cli-reference.mjs`, hide, `${root}/contracts/hide-cli.json`, `${root}/site/src/data/cli-notes.json`], { encoding: "utf8" });

test("a binary whose help lists a command with no note fails, tagged with that binary", () => {
  const hide = fakeHide([...Object.keys(notes).map((name) => `hide ${name}`), "hide session teleport <id>"]);
  const run = generate(hide);
  assert.equal(run.status, 1, run.stderr);
  assert.ok(run.stderr.includes(`commands with no note (1): session teleport [${hide}]`), run.stderr);
});

test("a binary whose help lists a note's command no more fails as a gone note, and a matching help passes", () => {
  const names = Object.keys(notes);
  const goneHide = fakeHide(names.slice(1).map((name) => `hide ${name}`));
  const gone = generate(goneHide);
  assert.equal(gone.status, 1, gone.stderr);
  assert.ok(gone.stderr.includes(`notes for commands that are gone (1): ${names[0]} [${goneHide}]`), gone.stderr);
  const matching = generate(fakeHide(names.map((name) => `hide ${name}`)));
  assert.equal(matching.status, 0, matching.stderr);
  assert.match(matching.stdout, /^<!-- Generated by scripts\/docs\/cli-reference.mjs/);
  // One command's own entry: its heading, its syntax line from that help and its note.
  assert.ok(matching.stdout.includes("### `hide mission gate`"), "no heading for hide mission gate");
  assert.ok(matching.stdout.includes("```sh\nhide mission gate\n```"), "no syntax block for hide mission gate");
  assert.ok(matching.stdout.includes(notes["mission gate"].line), "no note line for hide mission gate");
});

test("in CI mode a missing binary is a hard failure before anything is generated, with no fallback", () => {
  const env = { ...process.env, CI: "true" };
  delete env.HIDE_BIN;
  const run = spawnSync("node", [`${root}/scripts/docs/check-all.mjs`, "--checks-only"], { encoding: "utf8", env });
  assert.equal(run.status, 2, run.stderr);
  assert.match(run.stderr, /no hide binary.*CI mode: no default, no fallback/);
  assert.doesNotMatch(run.stdout, /^==/m);
  // A named path that does not exist fails the same way, in CI mode and with --ci.
  const missing = spawnSync("node", [`${root}/scripts/docs/check-all.mjs`, "--ci", "--hide", "/nonexistent/hide", "--checks-only"], { encoding: "utf8", env: { ...process.env, CI: "" } });
  assert.equal(missing.status, 2, missing.stderr);
  assert.doesNotMatch(missing.stdout, /^==/m);
});
