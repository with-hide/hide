#!/usr/bin/env node
// The CLI reference's source of truth, read from the installed `hide` (docs-user-docs P0): every command
// `hide --help` lists, with the syntax and options of the ones `contracts/hide-cli.json` also describes.
// A person writes one line and an example per command in the notes file; this fails on a command with no
// note and on a note whose command is gone, then prints the reference as Markdown.
// Usage: cli-reference.mjs <hide-binary> <contracts/hide-cli.json> <notes.json> > cli.md
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const isArgument = (word) => /^[<\[(-]/.test(word) || word === "…";

/** Splits a syntax line at the ` | ` that stand between whole commands, not at one inside `[...]`, `(...)` or `<...>`. */
function splitCommands(syntax) {
  const parts = [];
  let depth = 0;
  let current = "";
  for (let i = 0; i < syntax.length; i++) {
    const char = syntax[i];
    if ("[(<".includes(char)) depth++;
    else if ("])>".includes(char)) depth = Math.max(0, depth - 1);
    if (depth === 0 && syntax.startsWith(" | ", i)) {
      parts.push(current);
      current = "";
      i += 2;
      continue;
    }
    current += char;
  }
  parts.push(current);
  return parts.map((part) => part.trim());
}

/**
 * The commands one `--help` syntax line stands for, each as its words before the first argument.
 * Two shapes, and a line may use both:
 *   `request ack|cancel|show <id>`: alternatives inside one word, one command each;
 *   `memory show <card> | search <words> | add <text>`: a `|` between whole commands, each with its own
 *   arguments, where every command after the first takes the first one's words before its last word as its prefix.
 */
export function expand(syntax) {
  const [first, ...rest] = splitCommands(syntax.replace(/^hide\s*/, ""));
  const head = (text) => {
    const words = [];
    for (const word of text.split(/\s+/)) {
      if (!word || isArgument(word)) break;
      words.push(word);
    }
    return words;
  };
  const lead = head(first);
  if (lead.length === 0) return [];
  const prefix = lead.slice(0, -1);
  const commands = [];
  const add = (words) => {
    const last = words.pop();
    for (const alternative of last.split("|")) commands.push([...words, alternative].join(" "));
  };
  add([...lead]);
  for (const segment of rest) {
    const words = head(segment);
    if (words.length) add([...prefix, ...words]);
  }
  return commands;
}

/** Every command `hide --help` lists: `Map<name, { group, syntax }>`. */
export function commandsOf(help) {
  const commands = new Map();
  let group = "";
  for (const line of help.split("\n")) {
    if (/^[A-Z]/.test(line) && !line.startsWith("A command") && !line.startsWith("Commands") && !line.startsWith("On a ")) { group = line.trim(); continue; }
    if (!line.startsWith("  hide ")) continue;
    const syntax = line.trim().split(/\s{2,}/)[0];
    for (const name of expand(syntax)) if (name && !commands.has(name)) commands.set(name, { group, syntax });
  }
  return commands;
}

/** A Rust string literal's value: `\n`, `\"`, `\\`, and a `\` at a line's end that swallows the next line's indent. */
function rustString(raw) {
  return raw.replace(/\\\n\s*|\\(.)/g, (whole, char) => (char === undefined ? "" : char === "n" ? "\n" : char === "t" ? "\t" : char));
}
const STRING = String.raw`"((?:[^"\\]|\\[\s\S])*)"`;

/**
 * `hide --help` as the source writes it: `help_text()` in hided/src/cli.rs joins its own opening lines and, for each
 * element of its list, the usage of a command module (`crate::x::USAGE`), a constant of cli.rs
 * (`NAME_USAGE.trim_start_matches("usage: ")`) or a string, each line indented by two spaces. Reading it here makes the
 * source, not whichever `hide` happens to be installed, the authority on which commands exist.
 */
export function sourceHelp(root) {
  const dir = join(root, "hided/src");
  const cli = readFileSync(join(dir, "cli.rs"), "utf8");
  const start = cli.indexOf("fn help_text()");
  if (start < 0) throw new Error("hided/src/cli.rs: fn help_text() not found; the CLI reference reads the commands from it");
  const body = cli.slice(start, cli.indexOf("\n}\n", start));
  const opening = body.match(new RegExp(`String::from\\(\\s*${STRING}`))?.[1];
  const list = body.match(/for lines in \[([\s\S]*?)\] \{/)?.[1];
  if (opening === undefined || list === undefined) throw new Error("hided/src/cli.rs: help_text() no longer has the shape the CLI reference reads (String::from(\"...\") and `for lines in [...]`)");
  const constantOf = (file, name) => {
    const text = readFileSync(file, "utf8");
    const found = text.match(new RegExp(`const ${name}: &str = ${STRING};`));
    if (!found) throw new Error(`${file}: const ${name} not found`);
    return rustString(found[1]);
  };
  const lines = [rustString(opening)];
  const element = new RegExp(`crate::(\\w+)::USAGE|([A-Z_]+)\\.trim_start_matches\\("usage: "\\)|${STRING}`, "g");
  let count = 0;
  for (const found of list.matchAll(element)) {
    count++;
    const usage = found[1] ? constantOf(join(dir, `${found[1]}.rs`), "USAGE") : found[2] ? constantOf(join(dir, "cli.rs"), found[2]).replace(/^usage: /, "") : rustString(found[3]);
    lines.push(usage.split("\n").map((line) => `  ${line}`).join("\n") + "\n");
  }
  if (count === 0) throw new Error("hided/src/cli.rs: help_text()'s list is empty to the CLI reference");
  return lines.join("");
}

/**
 * The problems between the commands and the notes, naming which source found each: a command with no note, and a
 * note whose command the source no longer has. `sources` maps a label to a `Map` of commands; the first is the authority
 * for "gone", every one can find a command that lacks a note.
 */
export function noteProblems(sources, notes) {
  const entries = Object.entries(sources);
  const missing = new Map();
  for (const [label, commands] of entries) {
    for (const name of commands.keys()) if (!notes[name]) missing.set(name, [...(missing.get(name) ?? []), label]);
  }
  const authority = entries[0][1];
  const stale = Object.keys(notes).filter((name) => !authority.has(name));
  const problems = [];
  if (missing.size) problems.push(`commands with no note (${missing.size}): ${[...missing].map(([name, labels]) => `${name} [${labels.join(" + ")}]`).join(", ")}`);
  if (stale.length) problems.push(`notes for commands that are gone (${stale.length}): ${stale.join(", ")}`);
  return problems;
}

if (import.meta.url !== pathToFileURL(process.argv[1] ?? "").href) {
  // Imported by the tests: nothing below runs.
} else {
const [hide, contractPath, notesPath] = process.argv.slice(2);
if (!notesPath) throw new Error("usage: cli-reference.mjs <hide-binary> <hide-cli.json> <notes.json>");
const help = execFileSync(hide, ["--help"], { encoding: "utf8" });
const contract = new Map(JSON.parse(readFileSync(contractPath, "utf8")).commands.map((command) => [command.command, command]));
const notes = JSON.parse(readFileSync(notesPath, "utf8"));
const root = process.env.HIDE_DOCS_SOURCE_ROOT ?? join(dirname(fileURLToPath(import.meta.url)), "../..");
const fromSource = commandsOf(sourceHelp(root));
const fromBinary = commandsOf(help);
if (fromSource.size === 0) throw new Error("no command found in hided/src; the CLI reference cannot hold the commands to their notes");
// The source is the authority; the installed hide is a second reader and may be older or newer than it.
const problems = noteProblems({ "source": fromSource, "hide --help": fromBinary }, notes);
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}

const commands = fromSource;
const byFirstWord = new Map();
for (const [name, info] of commands) {
  const first = name.split(" ")[0];
  if (!byFirstWord.has(first)) byFirstWord.set(first, []);
  byFirstWord.get(first).push([name, info]);
}
const out = ["<!-- Generated by scripts/docs/cli-reference.mjs from `hide --help` and contracts/hide-cli.json. Edit the notes, not this file. -->", ""];
for (const [first, list] of byFirstWord) {
  out.push(`## hide ${first}`, "");
  for (const [name, info] of list) {
    const spec = contract.get(name);
    out.push(`### \`hide ${name}\``, "", notes[name].line, "");
    out.push("```sh", `hide ${info.syntax.replace(/^hide\s*/, "")}`, "```", "");
    if (notes[name].example) out.push("Example:", "", "```sh", notes[name].example, "```", "");
    if (spec?.options?.length) {
      out.push("| Option | Needed | Value |", "| --- | --- | --- |");
      for (const option of spec.options) out.push(`| \`${option.name}\` | ${option.required ? "yes" : "no"} | ${option.value ? option.value.type : "flag"} |`);
      out.push("");
    }
    if (spec?.refusals?.length) out.push(`Can be refused as: ${spec.refusals.map((r) => `\`${r}\``).join(", ")}.`, "");
    out.push(spec ? `_Syntax from the command contract._` : `_Syntax from \`hide --help\`; not in the command contract._`, "");
  }
}
console.log(out.join("\n"));
console.error(`ok: ${fromSource.size} commands from the source (${fromBinary.size} from the installed hide --help), ${[...commands.keys()].filter((n) => contract.has(n)).length} also in the contract`);
}
