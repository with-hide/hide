#!/usr/bin/env node
// Every event kind an e2e reads from the client-event counter must be one the daemon accepts.
//
//   node check-e2e-sent-kinds.mjs <e2e dir>   check (exit 1 on a read of a kind the daemon does not accept)
//
// Run from the package that owns the directory (web or desktop); TypeScript is resolved from there.
// `countSent` (`web/e2e/wire.ts`) answers 0 for a kind the page never sends, so a read of a renamed
// or removed kind passes every "nothing was sent" assertion and fails only a positive one, in
// whichever lane runs it; the desktop suite runs on macOS only. This finds such a read on every
// lint (docs/TESTING.md, Writing a Playwright e2e test).
//
// The kinds the daemon accepts are read from the match arms of its dispatch functions and nowhere
// else: hided's `handle_client_text` and `apply_boundary` (`hided/src/server.rs`), which answer some
// kinds themselves before the rest go on, and the core's `validate_event`
// (`hide-core/src/runtime/events.rs`), which decodes the rest. An arm is `"<kind>" =>`,
// `"<kind>" | "<kind>" if ... =>` or `Some("<kind>")`; a string anywhere else in those files (a
// payload field, a log line, a test) is not a kind. A function that is gone or names no kind is a
// failure: the dispatch changed shape and this reader must follow it.
//
// A wire read is `<receiver>.get("<kind>")` or `.get("<kind>.<action>")` with a string literal,
// whose receiver is bound to a counter: TypeScript resolves the receiver to its declaration, which
// is a counter when it holds `countSent(...)`, is the payload map handed to one
// (`countSent(page, <map>)`), or is an object property or destructured name carrying either. A
// parameter or a declared field holds whatever its caller passes, so one typed as a counter
// (`Map<string, number>` or `Map<string, Record<string, unknown>>`) counts as one.
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const COUNTER_TYPES = new Set(["Map<string, number>", "Map<string, Record<string, unknown>>"]);

/** The body of `fn <name>(` in Rust source: from its signature to the brace that closes it. */
function rustFunction(source, name, file) {
  const start = source.search(new RegExp(`\\bfn ${name}\\(`));
  if (start < 0) throw new Error(`${file}: no fn ${name}; the dispatch moved, so update check-e2e-sent-kinds.mjs`);
  const open = source.indexOf("{", source.indexOf(")", start));
  let depth = 0;
  for (let index = open; index < source.length; index += 1) {
    if (source[index] === "{") depth += 1;
    else if (source[index] === "}" && --depth === 0) return source.slice(open, index + 1);
  }
  throw new Error(`${file}: fn ${name} never closes`);
}

/** The event kinds the daemon accepts, from the match arms of its dispatch functions. */
export function acceptedKinds(root = ROOT) {
  const DISPATCH = [
    ["hided/src/server.rs", "handle_client_text"],
    ["hided/src/server.rs", "apply_boundary"],
    ["hide-core/src/runtime/events.rs", "validate_event"],
  ];
  const KIND = /"([a-z][a-z0-9_]*)"/g;
  const ARMS = [/^\s*"[a-z][a-z0-9_]*"(?:\s*\|\s*"[a-z][a-z0-9_]*")*\s*(?:=>|if\b)/gm, /Some\("[a-z][a-z0-9_]*"(?:\s*\|\s*"[a-z][a-z0-9_]*")*\)/g];
  const kinds = new Set();
  for (const [file, fn] of DISPATCH) {
    const body = rustFunction(fs.readFileSync(path.join(root, file), "utf8"), fn, file);
    const found = ARMS.flatMap((arm) => [...body.matchAll(arm)]).flatMap((match) => [...match[0].matchAll(KIND)].map((kind) => kind[1]));
    if (found.length === 0) throw new Error(`${file}: fn ${fn} names no event kind; update check-e2e-sent-kinds.mjs`);
    for (const kind of found) kinds.add(kind);
  }
  return kinds;
}

/** Reads of a kind outside `accepted` by a counter, in the `.ts` files of `directory`, as `file:line` messages. */
export function staleReads(directory, accepted, packageDir = process.cwd()) {
  const ts = createRequire(path.resolve(packageDir, "package.json"))("typescript");
  const files = fs.readdirSync(directory).filter((name) => name.endsWith(".ts")).sort().map((name) => path.resolve(directory, name));
  const program = ts.createProgram(files, {
    noEmit: true,
    skipLibCheck: true,
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    allowImportingTsExtensions: true,
  });
  const checker = program.getTypeChecker();
  const sources = files.map((file) => program.getSourceFile(file)).filter(Boolean);
  const isCountSent = (node) => ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "countSent";

  // The declarations that hold a counter, grown until a pass adds none, since helpers hand counters on.
  const counters = new Set();
  const holds = (expression) => {
    if (isCountSent(expression)) return true;
    const symbol = ts.isIdentifier(expression) ? checker.getSymbolAtLocation(expression) : undefined;
    return !!symbol?.declarations?.some((declaration) => counters.has(declaration));
  };
  for (let grown = true; grown; ) {
    const before = counters.size;
    for (const source of sources) {
      const visit = (node) => {
        if (isCountSent(node) && node.arguments[1] && ts.isIdentifier(node.arguments[1])) {
          for (const declaration of checker.getSymbolAtLocation(node.arguments[1])?.declarations ?? []) counters.add(declaration);
        }
        if ((ts.isVariableDeclaration(node) || ts.isPropertyAssignment(node)) && node.initializer && holds(node.initializer)) counters.add(node);
        if (ts.isShorthandPropertyAssignment(node) && checker.getShorthandAssignmentValueSymbol(node)?.declarations?.some((declaration) => counters.has(declaration))) counters.add(node);
        ts.forEachChild(node, visit);
      };
      visit(source);
    }
    grown = counters.size > before;
  }

  const symbolOf = (receiver) => {
    const symbol = checker.getSymbolAtLocation(ts.isPropertyAccessExpression(receiver) ? receiver.name : receiver);
    const declaration = symbol?.declarations?.[0];
    // A destructured name is the property it was taken from.
    if (declaration && ts.isBindingElement(declaration) && ts.isObjectBindingPattern(declaration.parent)) {
      const key = declaration.propertyName ?? declaration.name;
      if (ts.isIdentifier(key)) return checker.getTypeAtLocation(declaration.parent).getProperty(key.text) ?? symbol;
    }
    return symbol;
  };
  const isCounter = (receiver) => {
    const symbol = symbolOf(receiver);
    const declarations = symbol?.declarations ?? [];
    if (declarations.some((declaration) => counters.has(declaration))) return true;
    const passedIn = declarations.some((declaration) => ts.isParameter(declaration) || ts.isPropertySignature(declaration));
    return passedIn && COUNTER_TYPES.has(checker.typeToString(checker.getTypeAtLocation(receiver)));
  };

  const errors = [];
  let reads = 0;
  for (const source of sources) {
    const visit = (node) => {
      if (
        ts.isCallExpression(node) &&
        ts.isPropertyAccessExpression(node.expression) &&
        node.expression.name.text === "get" &&
        node.arguments.length === 1 &&
        ts.isStringLiteralLike(node.arguments[0]) &&
        isCounter(node.expression.expression)
      ) {
        reads += 1;
        const key = node.arguments[0].text;
        if (!accepted.has(key.split(".")[0])) {
          const line = source.getLineAndCharacterOfPosition(node.getStart()).line + 1;
          errors.push(`${path.relative(packageDir, source.fileName)}:${line} reads "${key}", a kind the daemon does not accept; the page never sends it, so its count is always 0`);
        }
      }
      ts.forEachChild(node, visit);
    };
    visit(source);
  }
  return { errors, reads };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [directory] = process.argv.slice(2);
  if (!directory) {
    console.error("usage: check-e2e-sent-kinds.mjs <e2e dir>");
    process.exit(2);
  }
  const { errors, reads } = staleReads(directory, acceptedKinds());
  if (errors.length) {
    console.error(errors.join("\n"));
    process.exit(1);
  }
  console.log(`ok: ${reads} wire reads in ${directory} name kinds the daemon accepts`);
}
