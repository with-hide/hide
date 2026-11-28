// The CLI reference's reader: how a help line expands to commands (checked on the real usage strings, taken from the
// source files that own them, never retyped: a command the reader fails to find would need no note and the reference
// would pass while missing it), and what a `hide` binary's help must hold to pass. Run by `check-all.mjs`: `node --test scripts/docs/*.test.mjs`.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { commandsOf, expand, noteProblems, topicsOf } from "./cli-reference.mjs";

const root = fileURLToPath(new URL("../..", import.meta.url));
const source = (path) => readFileSync(`${root}/${path}`, "utf8");

/** The value of `const NAME: &str = "...";` in a Rust file, with its string escapes read. */
function constant(path, name) {
  const match = source(path).match(new RegExp(`const ${name}: &str = "((?:[^"\\\\]|\\\\.)*)";`));
  assert.ok(match, `${name} not found in ${path}`);
  return JSON.parse(`"${match[1]}"`);
}

const links = constant("hided/src/cli.rs", "LINKS_USAGE");
const memory = constant("hided/src/cli.rs", "MEMORY_USAGE");
const notes = JSON.parse(source("site/src/data/cli-notes.json"));

test("a `|` between whole commands is one command each, with its own arguments", () => {
  const found = [...commandsOf(memory, "memory").keys()];
  for (const name of ["show", "search", "add", "forget", "undo", "placed", "building", "unbuilt", "outdated", "skipped", "stale", "release", "recheck"]) {
    assert.ok(found.includes(`memory ${name}`), `memory ${name} not found in: ${memory}`);
  }
  const found2 = [...commandsOf(links, "links").keys()];
  for (const name of ["pr", "issue", "branch", "session"]) assert.ok(found2.includes(`links ${name}`), `links ${name} not found in: ${links}`);
});

test("alternatives inside one word are one command each, and a `|` inside brackets splits nothing", () => {
  assert.deepEqual(expand("hide letter ack|cancel|show <id>"), ["letter ack", "letter cancel", "letter show"]);
  assert.deepEqual(expand("hide letter send <target> --intent <key> [--kind request|block] [--now | --force]"), ["letter send"]);
  assert.deepEqual(expand("hide task set (--file <plan.json> | --json <plan>) [--ask [why]]"), ["task set"]);
  assert.deepEqual(expand("hide <path>, hide ."), []);
});

test("the first screen's rows are its topics, under the group named on a group's first row", () => {
  const screen = [
    "usage: hide [open] [<path>]       open Hide, or a folder in it as a project",
    "       hide <topic> <verb> ...    run one of a topic's commands",
    "",
    "Agents     agent       start, show, read and end agents; their profiles",
    "           ps          the panes and agents running now, as a tree",
    "Work done  mission     plan and run agents that carry pull requests together",
    "",
    "A topic's commands: hide <topic> --help, or hide help <topic>.",
  ].join("\n");
  assert.deepEqual([...topicsOf(screen)], [["agent", "Agents"], ["ps", "Agents"], ["mission", "Work done"]]);
});

test("a topic's help gives its own commands only, and a guide gives the command that prints it", () => {
  const usage = "usage:\n  hide agent list [--live]\n  hide agent read <agent> [--lines <n>]\n  hide pane read <pane>\nThese commands need a running Hide.";
  assert.deepEqual([...commandsOf(usage, "agent").keys()], ["agent list", "agent read"]);
  const guide = "# hide task\nPut up the steps.\n\n```\nhide task set (--file <plan.json> | --inline <plan>)\nhide task approve        hide task drop        hide task show [<agent>]\nhide agent spawn --parent here --plan-node <key>\n```\n- `hide task node` marks a step.";
  assert.deepEqual([...commandsOf(guide, "task").keys()], ["task set", "task approve", "task drop", "task show", "task help"]);
  assert.equal(commandsOf(guide, "task").get("task drop").syntax, "hide task drop");
});

// The two usages are a part of the whole help, so the notes of every other command look "gone" here.
const noNote = (commands, given) => noteProblems({ "hide": commands }, given).filter((problem) => problem.startsWith("commands with no note"));

test("every command of those usages has a note, and a deleted note fails the check", () => {
  const commands = new Map([...commandsOf(memory, "memory"), ...commandsOf(links, "links")]);
  assert.deepEqual(noNote(commands, notes), []);
  for (const name of ["memory search", "memory stale", "links issue", "links session"]) {
    const { [name]: _removed, ...without } = notes;
    const problems = noNote(commands, without);
    assert.equal(problems.length, 1, name);
    assert.match(problems[0], new RegExp(`commands with no note.*${name}`));
  }
  assert.match(noteProblems({ "hide": commands }, { ...notes, "memory teleport": { line: "x" } }).join("\n"), /gone.*memory teleport/);
});

// A stand-in `hide` whose first screen lists the topics of the given lines, and whose `<topic> --help` prints that
// topic's lines as a usage.
function fakeHide(lines) {
  const dir = mkdtempSync(`${tmpdir()}/cli-reference-`);
  const byTopic = new Map();
  for (const line of lines) {
    const topic = line.split(" ")[1];
    byTopic.set(topic, [...(byTopic.get(topic) ?? []), line]);
  }
  const rows = [...byTopic.keys()].map((topic, index) => `${index === 0 ? "Topics" : ""}`.padEnd(11) + topic.padEnd(12) + "what it is for");
  writeFileSync(`${dir}/help.txt`, `usage: hide [open] [<path>]       open Hide\n\n${rows.join("\n")}\n\nA topic's commands: hide <topic> --help.\n`);
  for (const [topic, topicLines] of byTopic) writeFileSync(`${dir}/${topic}.txt`, `usage:\n${topicLines.map((line) => `  ${line}`).join("\n")}\n`);
  const path = `${dir}/hide`;
  writeFileSync(path, `#!/bin/sh\nif [ "$1" = --help ]; then cat ${dir}/help.txt; else cat "${dir}/$1.txt"; fi\n`);
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
  // All thirteen sections in reading order; every command of these names has a named section, none falls to "Other commands".
  const headings = [...matching.stdout.matchAll(/^## (.+)$/gm)].map((match) => match[1]);
  assert.deepEqual(headings, [
    "Start and stop Hide",
    "For agents and tools",
    "Letters and watches",
    "Agents",
    "Skills and MCP servers",
    "Missions",
    "Tasks",
    "Automations",
    "Panes and checkouts",
    "The operator's screen",
    "Devices",
    "Bots",
    "Past sessions and links",
    "Project Memory",
  ]);
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
