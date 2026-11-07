#!/usr/bin/env node
// Every event kind an e2e reads from the client-event counter must be one the daemon still names.
//
//   node check-e2e-sent-kinds.mjs <e2e dir>   check (exit 1 on a kind no daemon source names)
//
// Run from the package that owns the directory (web or desktop); TypeScript is resolved from there.
// `countSent` (`web/e2e/wire.ts`) answers 0 for a kind the page never sends, so a read of a renamed
// or removed kind passes every "nothing was sent" assertion and fails only a positive one, in
// whichever lane runs it; the desktop suite runs on macOS only. This finds such a read on every
// lint, on every system (docs/TESTING.md, Writing a Playwright e2e test).
//
// A wire read is `<name>.get("<kind>")` or `<name>.get("<kind>.<action>")` with a string literal,
// where `<name>` is bound to a counter (`<name> = countSent(...)`, `<name>: countSent(...)`) or is
// the payload map handed to one (`countSent(page, <name>)`), as an identifier or a property.
// The daemon names a kind it accepts as a quoted string: the core's `validate_event`
// (`hide-core/src/runtime/events.rs`) and the events hided answers itself (`hided/src`).
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const ts = createRequire(path.resolve("package.json"))("typescript");
const [directory] = process.argv.slice(2);
if (!directory) {
  console.error("usage: check-e2e-sent-kinds.mjs <e2e dir>");
  process.exit(2);
}
const ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname), "../..");

function rustFiles(target) {
  const full = path.join(ROOT, target);
  if (fs.statSync(full).isFile()) return [full];
  return fs.readdirSync(full, { recursive: true }).filter((name) => name.endsWith(".rs")).map((name) => path.join(full, name));
}
const named = new Set();
for (const file of ["hide-core/src/runtime/events.rs", "hided/src"].flatMap(rustFiles)) {
  for (const match of fs.readFileSync(file, "utf8").matchAll(/"([a-z][a-z0-9_]*)"/g)) named.add(match[1]);
}

const files = fs.readdirSync(directory).filter((name) => name.endsWith(".ts")).sort().map((name) => path.join(directory, name));
const sources = files.map((file) => ts.createSourceFile(file, fs.readFileSync(file, "utf8"), ts.ScriptTarget.Latest, true));
const isCountSent = (node) => ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "countSent";
const nameOf = (node) => (ts.isIdentifier(node) ? node.text : ts.isPropertyAccessExpression(node) ? node.name.text : null);

// The names a counter or its payload map is bound to, across the directory, since helpers hand them on.
const counters = new Set();
for (const source of sources) {
  const visit = (node) => {
    if (isCountSent(node)) {
      const parent = node.parent;
      if ((ts.isVariableDeclaration(parent) || ts.isPropertyAssignment(parent)) && ts.isIdentifier(parent.name)) counters.add(parent.name.text);
      const last = node.arguments[1];
      if (last && nameOf(last)) counters.add(nameOf(last));
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
}

const errors = [];
let reads = 0;
for (const source of sources) {
  const visit = (node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isPropertyAccessExpression(node.expression) &&
      node.expression.name.text === "get" &&
      counters.has(nameOf(node.expression.expression)) &&
      node.arguments.length === 1 &&
      ts.isStringLiteralLike(node.arguments[0])
    ) {
      reads += 1;
      const kind = node.arguments[0].text.split(".")[0];
      if (!named.has(kind)) {
        const line = source.getLineAndCharacterOfPosition(node.getStart()).line + 1;
        errors.push(`${source.fileName}:${line} reads "${node.arguments[0].text}", a kind neither the core nor hided names; the page never sends it, so its count is always 0`);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
}
if (errors.length) {
  console.error(errors.join("\n"));
  process.exit(1);
}
console.log(`ok: ${reads} wire reads in ${directory} name kinds the daemon accepts`);
