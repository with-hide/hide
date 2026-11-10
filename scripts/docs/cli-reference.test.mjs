// The CLI reference's reader against the usage strings `hide --help` is made of, taken from the source
// files that own them, never retyped: a command the reader fails to find would need no note, and the
// reference would pass while missing it. Run by `check-all.mjs`: `node --test scripts/docs/*.test.mjs`.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
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
const noNote = (commands, given) => noteProblems(commands, given).filter((problem) => problem.startsWith("commands with no note"));

test("every command of those usages has a note, and a deleted note fails the check", () => {
  const commands = new Map([...commandsOf(asHelp(memory)), ...commandsOf(asHelp(links))]);
  assert.deepEqual(noNote(commands, notes), []);
  for (const name of ["memory search", "memory stale", "links issue", "links session"]) {
    const { [name]: _removed, ...without } = notes;
    const problems = noNote(commands, without);
    assert.equal(problems.length, 1, name);
    assert.match(problems[0], new RegExp(`commands with no note.*${name}`));
  }
  assert.match(noteProblems(commands, { ...notes, "memory teleport": { line: "x" } }).join("\n"), /gone.*memory teleport/);
});
