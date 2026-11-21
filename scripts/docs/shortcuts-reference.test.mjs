// The Keyboard shortcuts page against the registry it is generated from (`web/src/shortcuts.ts`, imported as it stands):
// every command is a row with its keys on every system, a page that lost a row or a key fails, and the page agrees with
// the table docs/UI_BEHAVIOR.md prints from the same registry (that table is the expected answer from outside the
// generator). Run by `check-all.mjs`: `node --test scripts/docs/*.test.mjs`.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { load, problems, render } from "./shortcuts-reference.mjs";

const root = fileURLToPath(new URL("../..", import.meta.url));
const reg = await load();
const page = render(reg);

/** A Markdown table row as its cells, with the code spans' backticks and the cell escapes removed. */
const cellsOf = (line) =>
  line
    .replace(/^\|\s*|\s*\|$/g, "")
    .split(/(?<!\\)\|/)
    .map((text) => text.trim().replace(/^`` (.*) ``$/, "$1").replace(/^`(.*)`$/, "$1").replace(/\\\|/g, "|"));
/** The rows of the first table after `heading`, as `Map<first cell, cells>`. */
function tableAfter(markdown, heading) {
  const lines = markdown.split("\n");
  const start = lines.findIndex((line) => line === heading);
  assert.notEqual(start, -1, `no heading ${heading}`);
  const rows = new Map();
  for (const line of lines.slice(start + 1)) {
    if (line.startsWith("#")) break;
    if (!line.startsWith("| ") || /^\| -/.test(line) || /^\| (Command|Action|Hold) /.test(line)) continue;
    const [name, ...cells] = cellsOf(line);
    rows.set(name, cells);
  }
  return rows;
}

test("every command of the registry is a row with its keys on macOS, Windows and Linux", () => {
  assert.deepEqual(problems(reg, page), []);
  assert.ok(reg.REGISTRY.length > 80, "the registry was read whole");
});

test("a page that lost a command's row fails, for every command of the registry", () => {
  const lines = page.split("\n");
  for (const command of reg.REGISTRY) {
    const numbered = reg.numberedCommand(command.id);
    const title = numbered ? `${reg.NUMBERED_FAMILIES.find((family) => family.family === numbered.family).title} 1-9` : command.title;
    const without = lines.filter((line) => !line.startsWith(`| ${title} |`)).join("\n");
    const found = problems(reg, without);
    assert.ok(found.some((line) => line.startsWith(`command ${command.id} `) && line.endsWith("has no row")), `${command.id}: ${found}`);
  }
});

test("a page that holds a wrong key, or no key where the registry has one, fails", () => {
  // Copy Relative Path is Alt+Shift+H on Windows and Linux only: the one place the rule is overridden for a Mac-style chord.
  const wrong = page.replace("`Alt+Shift+H` | `Alt+Shift+H`", "`Alt+Shift+C` | `Alt+Shift+C`");
  assert.notEqual(wrong, page);
  assert.match(problems(reg, wrong).join("\n"), /command copy_relative_path is missing Alt\+Shift\+H for Windows and Linux/);
  const noChord = page.replace("| Overview | `⇧⌘O` |", "| Overview | none |");
  assert.notEqual(noChord, page);
  assert.match(problems(reg, noChord).join("\n"), /command overview is missing ⇧⌘O for macOS/);
  // Alt+Shift+T must not pass for the registry's Ctrl+Shift+T: a chord is a whole word of its row.
  const prefix = page.replace("| New tab | `⌘T` | `Ctrl+Shift+T` | `Ctrl+Shift+T` |", "| New tab | `⌘T` | `Ctrl+Shift+Tab` | `Ctrl+Shift+Tab` |");
  assert.match(problems(reg, prefix).join("\n"), /command new_tab is missing Ctrl\+Shift\+T for Windows and Linux/);
});

test("the page's app columns and browser rows agree with docs/UI_BEHAVIOR.md, Every command", () => {
  const doc = readFileSync(`${root}/docs/UI_BEHAVIOR.md`, "utf8").split("\n");
  const start = doc.indexOf("### Every command");
  assert.notEqual(start, -1);
  const rows = doc
    .slice(start)
    .filter((line) => /^\| .* \| .* \| .* \| .* \| .* \|$/.test(line) && !/^\| (Command|---)/.test(line))
    .map(cellsOf)
    .map(([title, appMac, appPc, browserMac, browserPc]) => ({ title: title.replace(/ \(exception\)$/, ""), appMac, appPc, browserMac, browserPc }));
  assert.ok(rows.length > 60, `read ${rows.length} rows of the UI_BEHAVIOR table`);
  const groups = [...new Set(reg.REGISTRY.map((command) => command.group))];
  const app = new Map(groups.flatMap((group) => [...tableAfter(page, `## ${group}`)]));
  const browser = tableAfter(page, "## In a browser tab");
  for (const row of rows) {
    const cells = app.get(row.title);
    assert.ok(cells, `no row for ${row.title} on the page`);
    assert.deepEqual(cells.slice(0, 3), [row.appMac, row.appPc, row.appPc], `${row.title}: app keys`);
    // A command the browser tab lists differently is on the page's browser table; every other one is the app's own keys.
    assert.deepEqual(browser.get(row.title)?.slice(0, 3) ?? [row.appMac, row.appPc, row.appPc], [row.browserMac, row.browserPc, row.browserPc], `${row.title}: browser keys`);
  }
  assert.equal(app.size, rows.length, "the page has a row for every row of the table and no other");
});

test("the generator runs under plain node, prints the page", () => {
  const run = spawnSync("node", [`${root}/scripts/docs/shortcuts-reference.mjs`], { encoding: "utf8" });
  assert.equal(run.status, 0, run.stderr);
  assert.equal(run.stdout, `${page}\n`);
  assert.match(run.stdout, /^---\ntitle: Keyboard shortcuts\n/);
});
