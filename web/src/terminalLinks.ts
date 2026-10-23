// Links in a terminal pane's text, whatever program printed them.
//
// A program can mark a link itself (OSC 8), but most print URLs and paths as
// plain text, so the rows are read when xterm asks about the row under the
// pointer: an http(s) URL, or a path-shaped token that becomes a link only
// once the filesystem says it exists (`terminalLinkProvider.ts` asks). Herdr's
// frames carry no wrap flag, so a token cut at a row's end may continue on
// the next row: at the terminal's edge (the continuation `selection.ts` reads
// for a copy) or at a TUI's own margin, after the indentation it starts its
// wrapped line with; a row that ends short followed by one that starts at the
// left edge is two lines, as `ls` prints them. Every way to join the pieces
// is offered longest first, and the existence check decides which spelling
// is real; a URL cannot be checked, so it joins only at the terminal's edge.

import { rowContinues, type CellRow } from "./selection";

/** Rows above and below the hovered one a joined token may reach. */
export const JOIN_ROWS = 3;

/** Spellings one hovered token may offer across all its joins, which bounds the paths one hover checks. */
export const TOKEN_CANDIDATES = 96;

/** Longer than any macOS path (`PATH_MAX` is 1024). */
const MAX_TEXT = 2048;

/** A piece of a token: its row in the rows handed in, and its columns (zero-based, end exclusive). */
export type Span = { row: number; start: number; end: number };

export type LinkTarget =
  | { kind: "url"; url: string }
  /** `path` as written: absolute, `~/`-relative, or relative to the pane's working folder. */
  | { kind: "path"; path: string; line: number | null; column: number | null };

export type LinkCandidate = { spans: Span[]; text: string; target: LinkTarget; original: boolean };

/**
 * Characters that end a token besides whitespace: the frames TUIs draw around
 * text (box drawing, blocks), the marks they put before a line (arrows,
 * bullets, `⎿`, `⏺`, check marks). `…` is not one, so a path a program
 * shortened is one token the parser refuses rather than a wrong path.
 */
const SEPARATOR = /[\s←-⇿─-▟⎿⏺•○●◦▪▶►✓✔✗✘]/u;

type Run = { row: number; start: number; end: number; text: string; first: boolean; last: boolean };

/** The row's tokens with their columns; a wide glyph's trailing half belongs to the glyph. */
function rowRuns(cells: CellRow, row: number): Run[] {
  const runs: Run[] = [];
  let text = "";
  let start = -1;
  let end = -1;
  const close = () => {
    if (start >= 0) runs.push({ row, start, end, text, first: false, last: false });
    text = "";
    start = -1;
  };
  for (let column = 0; column < cells.length; column += 1) {
    const chars = cells[column];
    if (chars === null) {
      if (start >= 0) end = column + 1;
      continue;
    }
    if (chars === undefined || chars === "" || SEPARATOR.test(chars)) {
      close();
      continue;
    }
    if (start < 0) start = column;
    text += chars;
    end = column + 1;
  }
  close();
  if (runs.length > 0) {
    runs[0]!.first = true;
    runs[runs.length - 1]!.last = true;
  }
  return runs;
}

/**
 * Every link that may cover row `at`, one group per token on that row. A
 * group's candidates are the ways to join that token with its neighbours on
 * the rows around it, longest first; the caller takes the first that holds.
 */
export function linkCandidates(rows: CellRow[], at: number): LinkCandidate[][] {
  const runsByRow = new Map<number, Run[]>();
  const runsOf = (row: number) => {
    let runs = runsByRow.get(row);
    if (!runs) {
      const cells = rows[row];
      runs = cells ? rowRuns(cells, row) : [];
      runsByRow.set(row, runs);
    }
    return runs;
  };
  const groups: LinkCandidate[][] = [];
  for (const run of runsOf(at)) {
    // The chain of pieces this token may be part of: the previous row's last
    // token while this is its row's first, and the next row's first while
    // this is its row's last.
    const before: Run[] = [];
    for (let piece = run, row = at - 1; piece.first && row >= Math.max(0, at - JOIN_ROWS); row -= 1) {
      const previous = runsOf(row).at(-1);
      if (!previous) break;
      before.unshift(previous);
      piece = previous;
    }
    const after: Run[] = [];
    for (let piece = run, row = at + 1; piece.last && row <= Math.min(rows.length - 1, at + JOIN_ROWS); row += 1) {
      const next = runsOf(row)[0];
      if (!next) break;
      after.push(next);
      piece = next;
    }
    const chain = [...before, run, ...after];
    const own = before.length;
    const group: LinkCandidate[] = [];
    for (let from = 0; from <= own; from += 1) {
      for (let to = own; to < chain.length; to += 1) {
        const pieces = chain.slice(from, to + 1);
        group.push(...joined(rows, pieces));
      }
    }
    group.sort((a, b) => spellingLength(b) - spellingLength(a) || b.text.length - a.text.length);
    // Past the bound the shortest go, never a token as written: a skipped
    // spelling loses a link, never makes a wrong one.
    const kept = new Set(group.filter((candidate) => candidate.original));
    for (const candidate of group) if (kept.size < TOKEN_CANDIDATES) kept.add(candidate);
    if (kept.size > 0) groups.push(group.filter((candidate) => kept.has(candidate)));
  }
  return groups;
}

/** Location text is clickable too, but precedence compares the actual path spelling. */
function spellingLength(candidate: LinkCandidate): number {
  return candidate.target.kind === "path" ? candidate.target.path.length : candidate.text.length;
}

function joined(rows: CellRow[], pieces: Run[]): LinkCandidate[] {
  const raw = pieces.map((piece) => piece.text).join("");
  if (raw.length > MAX_TEXT) return [];
  const breaks = pieces.slice(0, -1).map((piece, index) => (rowContinues(rows[piece.row] ?? []) ? "edge" : pieces[index + 1]!.start > 0 ? "margin" : null));
  if (breaks.includes(null)) return [];
  return interpretations(raw).flatMap((parsed) => {
    // URLs retain their existing parser and edge-only wrap rule.
    if (parsed.target.kind === "url" && breaks.includes("margin")) return [];
    const spans = trimSpans(rows, pieces, parsed.lead, parsed.trail);
    return spans ? [{ spans, text: parsed.text, target: parsed.target, original: parsed.original }] : [];
  });
}

/**
 * Map UTF-16 slice boundaries back to the buffer cells that supplied them.
 * One cell may hold a surrogate pair or a combining sequence, and null cells
 * extend its preceding wide glyph. Never cut either kind of glyph in half.
 */
function trimSpans(rows: CellRow[], pieces: Run[], lead: number, trail: number): Span[] | null {
  const end = pieces.reduce((length, piece) => length + piece.text.length, 0) - trail;
  const spans: Span[] = [];
  let offset = 0;
  for (const piece of pieces) {
    const cells = rows[piece.row] ?? [];
    for (let column = piece.start; column < piece.end; column += 1) {
      const chars = cells[column];
      if (!chars) continue;
      let next = column + 1;
      while (next < piece.end && cells[next] === null) next += 1;
      const after = offset + chars.length;
      if ((lead > offset && lead < after) || (end > offset && end < after)) return null;
      if (offset >= lead && after <= end) {
        const last = spans.at(-1);
        if (last?.row === piece.row && last.end === column) last.end = next;
        else spans.push({ row: piece.row, start: column, end: next });
      }
      offset = after;
    }
  }
  return spans.length > 0 ? spans : null;
}

// A whitespace-free token holds at most one link, and what it is written
// against is prose: brackets and quotes of any script, sentence punctuation,
// and Korean, Japanese or Chinese words with no space before or after
// (`열어뒀어요(http://…).`, `「docs/a.md」를`, `[label](src/a.ts)`). Rather
// than peel known wrappers off the outside, the token is read for its edges,
// the places a link can start or end, by Unicode's own classes, so a script
// or a mark not met before needs no rule of its own:
//
// - after an opening bracket or quote (Ps, Pi, the ASCII quotes, `<`) or an
//   `=` (`--file=/tmp/a.png`), and before a closing one (Pe, Pf, the ASCII
//   quotes, `>`);
// - on either side of sentence punctuation (`,;!?` and every non-ASCII Po,
//   such as `。、，：`), except `…`, which marks a shortened path the parser
//   refuses whole;
// - before a run of `.` or `:` that ends at an end (`a.md).`, `src/a.ts:42:`);
// - where a run of those scripts' letters meets the link, at the token's
//   start or end or beside another edge, unless the run touches `/`, `.`,
//   `-` or `_`, which join the parts of one name (`docs/회의록`, `한글.md`).
//
// A spelling reads as a link when its brackets balance, it holds no quote
// (nor the `'` it was opened with) and it does not end on sentence
// punctuation; from every start, the longest few that do are read for a
// path and its location. A whole name may still carry its punctuation, so
// the spellings between the outermost edges around such a path are offered
// as names too, and the existence check takes the longest that names
// something: a file really named `docs/a.md)` still wins over `docs/a.md`.
// A URL cannot be checked, so it is the longest spelling from its scheme
// that reads as a link.

/** Outermost starts and ends a whole name with its punctuation in it is tried between. */
const EDGES_PER_SIDE = 4;
/** Starts a token is read from; past them a token is data rather than prose. */
const STARTS_READ = 64;
/** The longest spellings read as a link from one start. */
const SPELLINGS_PER_START = 4;
/** Spellings of one token read for a path, which bounds the parser's work on a row of data. */
const SPELLINGS_READ = 64;

const OPENS = /[\p{Ps}\p{Pi}"'`<]/u;
const CLOSES = /[\p{Pe}\p{Pf}"'`>]/u;
const SENTENCE = /[,;!?]|(?![\p{ASCII}…])\p{Po}/u;
/** A quote no link holds, unlike the `'` a name or URL may carry. */
const QUOTE = /[\p{Pi}\p{Pf}"`<>]/u;
const PROSE = /(?=\p{L})[\p{scx=Hangul}\p{scx=Han}\p{scx=Hiragana}\p{scx=Katakana}]/u;
/** The characters any of the classes above may hold; the rest of a token is skipped unread. */
const MARKED = /[\p{P}\p{S}]|(?=\p{L})[\p{scx=Hangul}\p{scx=Han}\p{scx=Hiragana}\p{scx=Katakana}]/gu;
const OPEN = 1;
const CLOSE = 2;
const STOP = 4;
const QUOTED = 8;
const RAISE = 16;
const LOWER = 32;
const LETTER = 64;

/** Every class a character belongs to; ASCII is answered from a table, the rest once per character. */
const classes = (() => {
  const of = (char: string) =>
    (OPENS.test(char) ? OPEN : 0) | (CLOSES.test(char) ? CLOSE : 0) | (SENTENCE.test(char) ? STOP : 0) | (QUOTE.test(char) ? QUOTED : 0)
    | (/\p{Ps}/u.test(char) ? RAISE : 0) | (/\p{Pe}/u.test(char) ? LOWER : 0) | (PROSE.test(char) ? LETTER : 0);
  const ascii = Array.from({ length: 128 }, (_, code) => of(String.fromCharCode(code)));
  /** Bounded: a pane prints few distinct marks, and a flood of them only costs rereading. */
  const known = new Map<string, number>();
  return (char: string): number => {
    if (char.length === 1 && char.charCodeAt(0) < 128) return ascii[char.charCodeAt(0)]!;
    let found = known.get(char);
    if (found === undefined) {
      if (known.size >= 4096) known.clear();
      found = of(char);
      known.set(char, found);
    }
    return found;
  };
})();

/** Characters that join the parts of one name, so prose touching one is part of that name. */
const JOINER = /[/.\-_]/u;
/** The addresses read as written rather than looked for on disk. */
const SCHEME = /^(?:https?:\/\/|file:\/\/|vscode:\/\/file\/)/iu;

/**
 * A token read once: where a link may start (in order) and end (last first),
 * and, by UTF-16 offset, the bracket depth before it and the next quote at or
 * after it, so each start's readable spellings are found without rereading.
 */
type Scan = { starts: number[]; ends: number[]; depth: Int32Array; nextQuote: Int32Array; nextApostrophe: Int32Array; nextDrop: Int32Array };

function scan(token: string): Scan {
  const length = token.length;
  const starts = new Set([0]);
  const ends = new Set([length]);
  const step = new Int8Array(length);
  const quote = new Uint8Array(length);
  const runs: { from: number; to: number }[] = [];
  for (const match of token.matchAll(MARKED)) {
    const char = match[0];
    const at = match.index;
    const after = at + char.length;
    const kind = classes(char);
    if (kind & (OPEN | STOP) || char === "=") starts.add(after);
    if (kind & (CLOSE | STOP)) ends.add(at);
    if (kind & QUOTED) quote[at] = 1;
    if (kind & RAISE) step[at] = 1;
    else if (kind & LOWER) step[at] = -1;
    if (!(kind & LETTER)) continue;
    const last = runs.at(-1);
    if (last?.to === at) last.to = after;
    else runs.push({ from: at, to: after });
  }
  const runEndingAt = new Map(runs.map((run) => [run.to, run]));
  const runStartingAt = new Map(runs.map((run) => [run.from, run]));
  // Each new edge can expose another: `에.` ends at `.`, then at `에`.
  const pendingEnds = [...ends];
  for (let end = pendingEnds.pop(); end !== undefined; end = pendingEnds.pop()) {
    const more: number[] = [];
    if (end > 0 && (token[end - 1] === "." || token[end - 1] === ":")) more.push(end - 1);
    const run = runEndingAt.get(end);
    if (run && run.from > 0 && !JOINER.test(token[run.from - 1]!)) more.push(run.from);
    for (const edge of more) if (!ends.has(edge)) { ends.add(edge); pendingEnds.push(edge); }
  }
  for (const start of [...starts]) {
    const run = runStartingAt.get(start);
    if (run && run.to < length && !JOINER.test(token[run.to]!)) starts.add(run.to);
  }
  const depth = new Int32Array(length + 1);
  for (let index = 0; index < length; index += 1) depth[index + 1] = depth[index]! + step[index]!;
  const nextQuote = new Int32Array(length + 1).fill(length + 1);
  const nextApostrophe = new Int32Array(length + 1).fill(length + 1);
  for (let index = length - 1; index >= 0; index -= 1) {
    nextQuote[index] = quote[index] ? index : nextQuote[index + 1]!;
    nextApostrophe[index] = token[index] === "'" ? index : nextApostrophe[index + 1]!;
  }
  // The first offset after each where the depth falls below it: a closing bracket the spelling never opened.
  const nextDrop = new Int32Array(length + 1).fill(length + 1);
  const rising: number[] = [];
  for (let index = length; index >= 0; index -= 1) {
    while (rising.length > 0 && depth[rising.at(-1)!]! >= depth[index]!) rising.pop();
    nextDrop[index] = rising.at(-1) ?? length + 1;
    rising.push(index);
  }
  return { starts: [...starts].sort((a, b) => a - b), ends: [...ends].sort((a, b) => b - a), depth, nextQuote, nextApostrophe, nextDrop };
}

/**
 * The ends of the longest spellings from `from` that read as a link, longest
 * first: before the first quote (or the `'` that opened it) and the first
 * closing bracket it never opened, at its own bracket depth, and not on
 * sentence punctuation.
 */
function readableEnds(token: string, read: Scan, from: number): number[] {
  const limit = Math.min(read.nextQuote[from]!, token[from - 1] === "'" ? read.nextApostrophe[from]! : Infinity, read.nextDrop[from]! - 1);
  const found: number[] = [];
  for (const end of read.ends) {
    if (end <= from || found.length === SPELLINGS_PER_START) break;
    const last = token[end - 1]!;
    if (end <= limit && read.depth[end] === read.depth[from] && last !== "." && last !== ":" && last !== "'" && !(classes(last) & STOP)) found.push(end);
  }
  return found;
}

/** The characters of `text` that edges are made of: the measure of how much prose a spelling still carries. */
function edgeCharacters(text: string): number {
  let found = 0;
  for (const char of text) if (classes(char) & (OPEN | CLOSE | STOP | LETTER)) found += 1;
  return found;
}

/** `path:12`, `path:12:5`, `path:12-20`, `path#L12`, `path#L12-L20`, `path(12,5)`. */
const LOCATION = /^(.+?)(?::(\d+)(?::(\d+))?(?:-\d+)?|#L(\d+)(?:-L?\d+)?|\((\d+)(?:,\s*(\d+))?\))$/u;

/** A bare file name: a name with a letter in it and an extension that starts with one. */
const BARE_NAME = /^[\p{L}\p{N}_@+-][\p{L}\p{N}_.@+-]*\.[A-Za-z][A-Za-z0-9]{0,9}$/u;
const DOTFILE = /^\.[A-Za-z][\w.-]*$/u;

/** `parsed` is a spelling read for its location; otherwise the spelling is taken as a whole name. */
type Interpretation = { text: string; lead: number; trail: number; target: LinkTarget; original: boolean; parsed: boolean };

/** What `token` may link to: an address read as written, or the bounded path spellings. */
function interpretations(token: string): Interpretation[] {
  const read = scan(token);
  const { starts, ends } = read;
  const spelling = (from: number, to: number) => ({ text: token.slice(from, to), lead: from, trail: token.length - to, original: from === 0 && to === token.length });
  // An address is never probed as a local path.
  const schemes = starts.filter((from) => SCHEME.test(token.slice(from, from + 16)));
  if (schemes.length > 0) {
    return schemes.flatMap((from) => {
      const to = readableEnds(token, read, from)[0];
      const target = to === undefined ? null : parseTarget(token.slice(from, to));
      return to !== undefined && target ? [{ ...spelling(from, to), target, parsed: true }] : [];
    });
  }
  const result: Interpretation[] = [];
  const seen = new Set<string>();
  const add = (candidate: Interpretation) => {
    const key = `${candidate.lead} ${candidate.trail} ${JSON.stringify(candidate.target)}`;
    if (seen.has(key)) return;
    seen.add(key);
    result.push(candidate);
  };
  let budget = SPELLINGS_READ;
  for (const from of starts.slice(0, STARTS_READ)) {
    if (budget <= 0) break;
    for (const to of readableEnds(token, read, from).slice(0, budget)) {
      budget -= 1;
      const target = parseTarget(token.slice(from, to));
      if (target?.kind === "path") add({ ...spelling(from, to), target, parsed: true });
    }
  }
  const parsed = [...result];
  if (parsed.length === 0) return [];
  // A spelling around a path may be a name with its punctuation, prose or
  // location in it; validation still refuses schemes, controls and shortened paths.
  for (const from of starts.slice(0, EDGES_PER_SIDE)) {
    for (const to of ends.slice(0, EDGES_PER_SIDE)) {
      const text = token.slice(from, to);
      if (to <= from || !parsed.some((path) => path.lead >= from && token.length - path.trail <= to)) continue;
      if (text.length > 1024 || text.includes("…") || /\p{Cc}/u.test(text)) continue;
      if (/^[A-Za-z][\w+-]*:/u.test(text) && !LOCATION.test(text)) continue;
      add({ ...spelling(from, to), target: { kind: "path", path: text, line: null, column: null }, parsed: false });
    }
  }
  return result;
}

/**
 * What a whitespace-free token links to without looking at the filesystem,
 * with how much of it the link leaves out at either end: an address, or the
 * path spelling that reads as one and carries the least prose.
 */
export function parseToken(token: string): { text: string; lead: number; trail: number; target: LinkTarget } | null {
  const best = interpretations(token)
    .filter((each) => each.parsed)
    .sort((a, b) => edgeCharacters(a.text) - edgeCharacters(b.text) || b.text.length - a.text.length)[0];
  return best ? { text: best.text, lead: best.lead, trail: best.trail, target: best.target } : null;
}

function parseTarget(text: string): LinkTarget | null {
  if (/^https?:\/\/\S+$/iu.test(text)) {
    try {
      return { kind: "url", url: new URL(text).href };
    } catch {
      return null;
    }
  }
  const uri = uriTarget(text);
  if (uri !== undefined) return uri;
  const location = LOCATION.exec(text);
  // A Windows shell prints a drive path with backslashes (`C:\Users\example\...`);
  // the wire spells it with `/` (docs/ARCHITECTURE.md, The platform layer).
  const printed = location ? location[1]! : text;
  const path = /^[A-Za-z]:\\/u.test(printed) ? printed.replaceAll("\\", "/") : printed;
  const line = location ? Number(location[2] ?? location[4] ?? location[5]) : null;
  const column = location && (location[3] ?? location[6]) ? Number(location[3] ?? location[6]) : null;
  if (!pathShaped(path)) return null;
  return { kind: "path", path, line: line && line > 0 ? line : null, column: column && column > 0 ? column : null };
}

/** The root kept by the wire spelling: a POSIX root, Windows drive or share. */
export function absolutePathRoot(path: string): string | null {
  const drive = /^[A-Za-z]:\//u.exec(path);
  if (drive) return drive[0];
  if (path.startsWith("//")) {
    const [server, share] = path.slice(2).split("/");
    return server && share ? `//${server}/${share}/` : null;
  }
  return path.startsWith("/") ? "/" : null;
}

function pathShaped(path: string): boolean {
  const root = absolutePathRoot(path);
  if (path.length > 1024 || path.includes("…") || path.slice(root?.length ?? 0).includes(":") || /\p{Cc}/u.test(path)) return false;
  if (path === "/" || (path.startsWith("//") && !root)) return false;
  if (root || path.startsWith("~/") || path.startsWith("./") || path.startsWith("../")) return true;
  if (path.includes("/")) return /[\p{L}]/u.test(path);
  return DOTFILE.test(path) || (BARE_NAME.test(path) && /\p{L}/u.test(path.slice(0, path.lastIndexOf("."))));
}

/**
 * A link a program marked with OSC 8, or `undefined` when the address is not
 * one a pane link follows. `file://` and the `vscode://file/` links Codex
 * writes for its file citations are paths; http(s) is a URL.
 */
export function osc8Target(uri: string): LinkTarget | null | undefined {
  if (/^https?:\/\//iu.test(uri)) {
    try {
      return { kind: "url", url: new URL(uri).href };
    } catch {
      return null;
    }
  }
  return uriTarget(uri);
}

/** A `file:` or `vscode://file/` address as a path; `undefined` for any other text. */
function uriTarget(text: string): LinkTarget | null | undefined {
  let rest: string;
  if (/^file:\/\//iu.test(text)) {
    try {
      const url = new URL(text);
      if (url.host !== "" && url.host !== "localhost") return null;
      rest = decodeURIComponent(url.pathname) + (url.hash.startsWith("#L") ? url.hash : "");
    } catch {
      return null;
    }
  } else if (/^vscode:\/\/file\//iu.test(text)) {
    try {
      rest = decodeURIComponent(text.slice("vscode://file".length));
    } catch {
      return null;
    }
  } else {
    return undefined;
  }
  const drive = rest.slice(1);
  if (!drive.startsWith("/") && absolutePathRoot(drive)) rest = drive;
  const location = LOCATION.exec(rest);
  const path = location ? location[1]! : rest;
  if (!absolutePathRoot(path) || path.includes("\0")) return null;
  const line = location ? Number(location[2] ?? location[4] ?? location[5]) : null;
  const column = location && (location[3] ?? location[6]) ? Number(location[3] ?? location[6]) : null;
  return { kind: "path", path, line: line && line > 0 ? line : null, column: column && column > 0 ? column : null };
}
