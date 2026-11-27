// File paths in an answer's words (feedback 2 frame M [2]): which spellings
// in Desk's answers and a session's text are paths, and what each names.
//
// One rule decides it. A path is read the way a terminal link reads one
// (`parseToken` in `terminalLinks.ts`): from a word of prose, or from inline
// code that holds the path alone; a fenced code block stays code. A relative
// path is read under the session's own folder. What it names decides the
// chip: a file that exists inside a registered project of the session's
// machine is a project chip, opened on Desk's desk; a file or folder of
// this computer outside every project is an outside chip, revealed in the
// system's file manager and never opened; anything else (nothing there, a
// folder inside a project, an outside path of a connected device, which this
// computer's file manager cannot show) stays words.
//
// Bounds: at most `MAX_PATHS` paths are looked up per answer; a file inside a
// project is checked by reading its first byte over the file channel, which a
// device answers itself, and an outside path through the desktop host's
// check (`probePaths`, cached as the terminal's links are); both answers are
// kept `CHECK_TTL_MS`, at most `CHECKS_KEPT` of them, the oldest let go first.
// Every answer on screen shares one line of project file checks: at most
// `CHECKING` read at once, one read per file however many answers name it,
// and a read that gets no answer in `CHECK_WAIT_MS` counts as no file (with a
// diagnostic), so a device gone quiet keeps no answer's chips from showing.

import { requestFileBytesWithTotal } from "./fileBytes";
import { hostBridge } from "./host";
import { windowDeviceId } from "./screenMachine";
import { catalogWorkspaces, type SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { pathLookups, probePaths, terminalPathOwner } from "./terminalLinkProvider";
import { absolutePathRoot, longestRoot, normalize, parseToken } from "./terminalLinks";

/** Paths one answer looks up; the rest stay words. */
export const MAX_PATHS = 32;
/** How long a project file's check is trusted, so a file made after a miss becomes a chip soon after. */
export const CHECK_TTL_MS = 10_000;
/** Project file checks kept; the oldest goes first. */
export const CHECKS_KEPT = 512;
/** Project file checks read at once, across every answer on screen; the rest wait their turn. */
export const CHECKING = 4;
/** How long one check waits for its read before it counts as no file. */
export const CHECK_WAIT_MS = 10_000;

/** A path as an answer wrote it: where it stands (`from` to `to`, a code span's backticks included) and its spelling. */
export type WrittenPath = { from: number; to: number; written: string };

/** What a written path names: a project's file, read and opened under its checkout; or a file or folder of this computer outside every project. */
export type FoundAnswerPath =
  | { kind: "project"; path: string; device: string; root: string }
  | { kind: "outside"; path: string };

/** Where a session's words were written: its machine and its own folder (null for Desk, whose folder is no project). */
export type AnswerPlace = { device: string; cwd: string | null };

const FENCE = /^ {0,3}(`{3,}|~{3,})/;

/**
 * The paths `text` writes, in order, outside fenced code blocks: each word of
 * prose a path reads from, and each inline code span that holds a path alone.
 */
export function writtenPaths(text: string): WrittenPath[] {
  const found: WrittenPath[] = [];
  let fence: string | null = null;
  let offset = 0;
  for (const line of text.split("\n")) {
    const start = offset;
    offset += line.length + 1;
    const marker = FENCE.exec(line)?.[1] ?? null;
    if (fence !== null) {
      if (marker !== null && marker[0] === fence[0] && marker.length >= fence.length && line.trim() === marker) fence = null;
      continue;
    }
    if (marker !== null) {
      fence = marker;
      continue;
    }
    readLine(line, start, found);
  }
  return found;
}

/** A line's paths: inline code spans first, as whole spans, then the words between them. */
function readLine(line: string, start: number, found: WrittenPath[]): void {
  const code = /(`+)([^`]+?)\1(?!`)/g;
  let prose = 0;
  for (let match = code.exec(line); match; match = code.exec(line)) {
    readWords(line.slice(prose, match.index), start + prose, found);
    const inner = match[2]!.trim();
    // Code with words in it (a command) stays code; only a path alone becomes a chip.
    const read = /\s/.test(inner) ? null : parseToken(inner);
    if (read && read.target.kind === "path" && read.lead === 0 && read.trail === 0) found.push({ from: start + match.index, to: start + match.index + match[0].length, written: read.target.path });
    prose = match.index + match[0].length;
  }
  readWords(line.slice(prose), start + prose, found);
}

function readWords(words: string, start: number, found: WrittenPath[]): void {
  for (const word of words.matchAll(/\S+/g)) {
    // A Markdown link keeps its own syntax; its address is not a chip.
    if (word[0].includes("](")) continue;
    const read = parseToken(word[0]);
    if (!read || read.target.kind !== "path") continue;
    const from = start + word.index! + read.lead;
    found.push({ from, to: from + read.text.length, written: read.target.path });
  }
}

/** The session's machine and its own folder, from the pane the core lists; null when no checkout holds the pane. */
export function answerPlace(rest: SnapshotRest | null, paneId: string): AnswerPlace | null {
  for (const workspace of catalogWorkspaces(rest)) {
    for (const checkout of workspace.checkouts) {
      const pane = checkout.tabs.flatMap((tab) => tab.panes).find((row) => row.id === paneId);
      if (pane) return { device: workspace.device_id, cwd: pane.cwd || checkout.path };
    }
  }
  return null;
}

/** The registered checkout of `device` that holds `path`, by the longest root as the terminal's links choose (`longestRoot`). */
function lexicalOwner(rest: SnapshotRest | null, device: string, path: string): string | null {
  const roots = catalogWorkspaces(rest)
    .filter((workspace) => workspace.device_id === device)
    .flatMap((workspace) => workspace.checkouts.map((checkout) => ({ root: checkout.path, owner: checkout.path })));
  return longestRoot(path, roots)?.owner ?? null;
}

type Check = { exists: boolean; at: number };
const checks = new Map<string, Check>();
/** Checks being read now, so a second answer naming the same file waits on the same read. */
const reading = new Map<string, Promise<boolean>>();
/** Checks waiting for one of the `CHECKING` turns. */
const turns: (() => void)[] = [];
let checking = 0;

/** Whether a project's file exists, read as its first byte under its checkout on its machine; a folder or a refused read is no file. */
function projectFileExists(path: string, device: string, root: string, here: string, now: number): Promise<boolean> {
  const key = `${device}\n${path}`;
  const hit = checks.get(key);
  if (hit && now - hit.at < CHECK_TTL_MS) return Promise.resolve(hit.exists);
  const pending = reading.get(key);
  if (pending) return pending;
  const read = (async () => {
    if (checking >= CHECKING) await new Promise<void>((turn) => turns.push(turn));
    checking += 1;
    let exists: boolean;
    try {
      exists = await withinCheck(requestFileBytesWithTotal(path, { length: 1 }, device === here ? null : { device, root }), device, path);
    } finally {
      checking -= 1;
      turns.shift()?.();
    }
    reading.delete(key);
    checks.delete(key);
    checks.set(key, { exists, at: now });
    while (checks.size > CHECKS_KEPT) checks.delete(checks.keys().next().value!);
    return exists;
  })();
  reading.set(key, read);
  return read;
}

/** Whether `read` answered with the file: a refusal is no file, and so is no answer within `CHECK_WAIT_MS`. */
function withinCheck(read: Promise<unknown>, device: string, path: string): Promise<boolean> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      useShellStore.getState().noteDiagnostic(`answer path: no answer within ${CHECK_WAIT_MS} ms device=${device} path=${path}`);
      resolve(false);
    }, CHECK_WAIT_MS);
    read.then(() => true, () => false).then((exists) => {
      clearTimeout(timer);
      resolve(exists);
    });
  });
}

/** Forgets every check (tests). */
export function clearAnswerPathChecks(): void {
  checks.clear();
  reading.clear();
}

/**
 * What each written path names, by its spelling; a path that names nothing a
 * chip can stand for is left out. `here` is the core's own machine, whose
 * files are read without naming a device.
 */
export async function findAnswerPaths(written: readonly string[], place: AnswerPlace, rest: SnapshotRest | null, here: string, now = Date.now()): Promise<Map<string, FoundAnswerPath>> {
  const found = new Map<string, FoundAnswerPath>();
  const spellings = [...new Set(written)].slice(0, MAX_PATHS);
  const window = windowDeviceId(rest?.navigator?.devices);
  const bridge = place.device === window ? hostBridge() : null;
  const outside: { written: string; lookups: string[] }[] = [];
  await Promise.all(spellings.map(async (spelling) => {
    // A `~/` path names the home of the machine that reads it; only this computer's host can say which file that is.
    const lookups = spelling.startsWith("~/") ? [spelling] : pathLookups(spelling, place.cwd, null);
    for (const path of lookups) {
      if (!absolutePathRoot(path)) continue;
      const root = lexicalOwner(rest, place.device, normalize(path));
      if (root && await projectFileExists(path, place.device, root, here, now)) {
        found.set(spelling, { kind: "project", path, device: place.device, root });
        return;
      }
    }
    if (bridge) outside.push({ written: spelling, lookups });
  }));
  if (!bridge || outside.length === 0) return found;
  const answers = await probePaths(outside.flatMap((one) => one.lookups), (paths) => bridge.probePaths(paths));
  for (const one of outside) {
    const hit = one.lookups.map((path) => answers.get(path)).find((answer) => answer);
    if (!hit) continue;
    // The host reads the physical path: one a symlink leads into a project is that project's file.
    const owner = await terminalPathOwner(hit);
    if (owner === undefined) continue;
    if (owner === null) found.set(one.written, { kind: "outside", path: hit.real });
    else if (hit.kind === "file") found.set(one.written, { kind: "project", path: owner.path, device: owner.deviceId, root: owner.checkout.path });
  }
  return found;
}
