// The one rule every conversation screen draws its rows by (mockup v3 sections 1, 2, 5): the agent's words always
// show, each piece led by ●, as a terminal shows them; only the tools between two pieces of words fold, into one gray
// line per run that counts them by kind; a letter, a new session, an opened View and the agent's own question stand in
// their place between them. A session beside Desk, the pane conversation mode, Desk's own conversation and the phone
// all feed this rule `ThreadLine`s and draw its rows with the parts beside it, so the same record reads the same
// everywhere. Nothing here reads the store, a pane or a Desk job.

import type { DeskSessionRef, LetterKind, ThreadHidePart, ThreadLine, WorkingPhase } from "../../desk";
import { diffCounts } from "./diff";

export type UserLine = Extract<ThreadLine, { kind: "user" }>;
export type AgentLine = Extract<ThreadLine, { kind: "agent" }>;
export type ToolLine = Extract<ThreadLine, { kind: "tool" }>;
export type QuestionLine = Extract<ThreadLine, { kind: "question" }>;
export type HideLine = Extract<ThreadLine, { kind: "hide" }>;
/** Views opened one after another, which share one row of tiles. */
export type ViewLine = HideLine & { part: Extract<ThreadHidePart, { kind: "view" }> };

/** The kind a tool line, or a run's gray line, is marked with: gray icons, told apart before the words are read. */
export type ToolMark = "command" | "edit" | "read" | "web" | "question" | "other";

/**
 * What one run of tools did, as its gray line counts it: commands run and how many of its calls failed, files read
 * (each file once), searches, web reads, files changed (each once) with the lines added and taken away, other calls,
 * and the thoughts it kept in clear.
 */
export type RunCounts = { commands: number; failed: number; read: number; searched: number; fetched: number; edited: number; added: number; removed: number; other: number; thoughts: number };

/**
 * The tools between two pieces of the agent's words, folded into one gray line (`ToolRunLine`): its calls, thoughts
 * and plan changes in order, what they did by kind, and the mark its line leads with (an edit before a command
 * before a web read before a read).
 */
export type ToolRun = { key: string; lines: ThreadLine[]; counts: RunCounts; mark: ToolMark };

/**
 * A letter as its band draws it (`LetterBand`): sent by this session (a `hide letter send`) or received (a letter
 * delivered as a prompt), the other session when the record names it, its kind, its first line and its whole text.
 * `command` is what the session typed to send it.
 */
export type Letter = { key: string; direction: "sent" | "received"; peer: DeskSessionRef | null; kind: LetterKind | null; first: string; text: string; command: string | null };

/** One thing a turn holds, in the order the session wrote it. */
export type TurnItem =
  | { kind: "text"; line: AgentLine }
  | { kind: "run"; run: ToolRun }
  | { kind: "letter"; letter: Letter }
  /** A new session or a mission the turn reached, drawn once per target where it first reached it. */
  | { kind: "part"; line: HideLine }
  | { kind: "views"; lines: ViewLine[] }
  | { kind: "question"; line: QuestionLine };

/**
 * One turn (B25): the message that began it (the operator's words, an interruption, or a letter delivered as a
 * prompt; the first turn may have none), what it holds in order, every piece of its words, and its newest line as the
 * session wrote it, which says what a running turn does now.
 */
export type ConversationTurn = { key: string; user: UserLine | null; items: TurnItem[]; texts: AgentLine[]; last: ThreadLine | null };

const NO_COUNTS: RunCounts = { commands: 0, failed: 0, read: 0, searched: 0, fetched: 0, edited: 0, added: 0, removed: 0, other: 0, thoughts: 0 };

/** The mark a call's line leads with, by its kind. */
export function toolMark(kind: ToolLine["toolKind"]): ToolMark {
  switch (kind) {
    case "execute":
      return "command";
    case "edit":
    case "delete":
    case "move":
      return "edit";
    case "read":
    case "search":
      return "read";
    case "fetch":
      return "web";
    default:
      return "other";
  }
}

/** What a run's lines did, by kind. */
export function runCounts(lines: readonly ThreadLine[]): RunCounts {
  const counts = { ...NO_COUNTS };
  const read = new Set<string>();
  const edited = new Set<string>();
  for (const line of lines) {
    if (line.kind === "thought") counts.thoughts += 1;
    if (line.kind !== "tool") continue;
    if (line.status === "failed") counts.failed += 1;
    switch (toolMark(line.toolKind)) {
      case "command":
        counts.commands += 1;
        break;
      case "edit": {
        // A file changed twice is one file changed; a call that names none counts on its own.
        for (const path of line.diffs.length ? line.diffs.map((diff) => diff.path) : [line.subject ?? line.id]) edited.add(path);
        const { added, removed } = diffCounts(line.diffs);
        counts.added += added;
        counts.removed += removed;
        break;
      }
      case "read":
        if (line.toolKind === "search") counts.searched += 1;
        else read.add(line.subject ?? line.id);
        break;
      case "web":
        counts.fetched += 1;
        break;
      default:
        counts.other += 1;
    }
  }
  counts.read = read.size;
  counts.edited = edited.size;
  return counts;
}

function runMark(counts: RunCounts): ToolMark {
  if (counts.edited) return "edit";
  if (counts.commands) return "command";
  if (counts.fetched) return "web";
  if (counts.read || counts.searched) return "read";
  return "other";
}

/** A run by the turn it stands in and its first line: a call's id alone repeats in a record replaced at the same path. */
function runOf(turn: string, lines: ThreadLine[]): ToolRun {
  const counts = runCounts(lines);
  return { key: `${turn}~${lines[0]!.id}~run`, lines, counts, mark: runMark(counts) };
}

/** A letter the session sent, as its band draws it. */
function sentLetter(turn: string, line: HideLine & { part: Extract<ThreadHidePart, { kind: "letter" }> }): Letter {
  const text = line.part.body;
  return { key: `${turn}~${line.id}~letter`, direction: "sent", peer: line.part.peer, kind: line.part.letterKind, first: firstLine(text), text, command: line.command };
}

/** A letter delivered to the session as a prompt, as its band draws it: a doorbell's line by the first line it names. */
export function receivedLetter(line: UserLine): Letter {
  const text = line.text.trim();
  return { key: line.id, direction: "received", peer: line.peer, kind: line.letterKind, first: line.summary ?? firstLine(text), text, command: null };
}

const firstLine = (text: string) => text.trim().split("\n", 1)[0] ?? "";

const isLetter = (line: ThreadLine): line is HideLine & { part: Extract<ThreadHidePart, { kind: "letter" }> } => line.kind === "hide" && line.part.kind === "letter";
const isView = (line: ThreadLine): line is ViewLine => line.kind === "hide" && line.part.kind === "view";
/** What folds into a run: the calls, the thoughts and the plan changes. Everything else stands on its own. */
const folds = (line: ThreadLine) => line.kind === "tool" || line.kind === "thought" || line.kind === "plan" || (line.kind === "hide" && line.part.kind === "plan");

/**
 * A turn's lines with each new session, View or mission drawn once, where the turn first reached it, with its newest
 * call's state and command (six `hide mission` calls are one chip). Every letter is its own: two letters to one
 * session are two bands.
 */
function onePerTarget(lines: readonly ThreadLine[]): ThreadLine[] {
  const merged = (line: ThreadLine): line is HideLine => line.kind === "hide" && line.part.kind !== "letter" && line.part.kind !== "plan";
  const newest = new Map<string, HideLine>();
  for (const line of lines) if (merged(line)) newest.set(line.part.target, line);
  const drawn = new Set<string>();
  const kept: ThreadLine[] = [];
  for (const line of lines) {
    if (!merged(line)) {
      kept.push(line);
      continue;
    }
    const target = line.part.target;
    if (drawn.has(target)) continue;
    drawn.add(target);
    const last = newest.get(target)!;
    kept.push(last === line ? line : { ...line, part: last.part, command: last.command });
  }
  return kept;
}

/** A turn's lines as the items it draws, in the order the session wrote them. */
function turnItems(turn: string, body: readonly ThreadLine[]): TurnItem[] {
  const items: TurnItem[] = [];
  let run: ThreadLine[] = [];
  const close = () => {
    if (run.length) items.push({ kind: "run", run: runOf(turn, run) });
    run = [];
  };
  for (const line of onePerTarget(body)) {
    if (folds(line)) {
      run.push(line);
      continue;
    }
    close();
    if (line.kind === "agent") items.push({ kind: "text", line });
    else if (line.kind === "question") items.push({ kind: "question", line });
    else if (isLetter(line)) items.push({ kind: "letter", letter: sentLetter(turn, line) });
    else if (isView(line)) {
      const last = items.at(-1);
      if (last?.kind === "views") last.lines.push(line);
      else items.push({ kind: "views", lines: [line] });
    } else if (line.kind === "hide") items.push({ kind: "part", line });
  }
  close();
  return items;
}

/**
 * A thread's lines as turns (B25): each message, interruption or letter that reached the session starts one; what
 * follows it is its items. The agent's words are never folded, wherever they stand: a turn that wrote its summary and
 * then asked its question still shows the summary.
 */
export function conversationTurns(lines: readonly ThreadLine[]): ConversationTurn[] {
  const turns: ConversationTurn[] = [];
  let open: { user: UserLine | null; body: ThreadLine[]; key: string } | null = null;
  const close = () => {
    if (!open) return;
    const texts = open.body.filter((line): line is AgentLine => line.kind === "agent");
    turns.push({ key: open.key, user: open.user, items: turnItems(open.key, open.body), texts, last: open.body.at(-1) ?? null });
  };
  for (const line of lines) {
    if (line.kind === "user") {
      close();
      open = { user: line, body: [], key: line.id };
      continue;
    }
    open ??= { user: null, body: [], key: line.id };
    open.body.push(line);
  }
  close();
  return turns;
}

/** What the working line says a session's running turn does now, from its newest line: its words, else its newest call or Hide part. */
export function conversationPhase(turn: ConversationTurn | undefined): WorkingPhase {
  const last = turn?.last;
  switch (last?.kind) {
    case "agent":
      return "answering";
    case "tool":
      switch (last.toolKind) {
        case "read":
        case "search":
        case "fetch":
          return "files";
        case "edit":
        case "delete":
        case "move":
          return "editing";
        case "execute":
          return "other";
        default:
          return "thinking";
      }
    case "hide":
      return last.part.kind === "letter" ? "letters" : last.part.kind === "spawn" ? "starts" : "sessions";
    default:
      return "thinking";
  }
}

/**
 * The room a row keeps above it: `turn` between turns, `part` between the things of one turn, `member` between the
 * lines of an opened run, `foot` above the working line and the question card, `none` first.
 */
export type RowGap = "none" | "turn" | "part" | "member" | "foot";

/** A row's own part: what it draws, by its kind (`ConversationRows` in `thread-view.tsx`, and the phone's list). */
type RowOf =
  /** The operator's words, or an interrupted turn's one quiet word; `handed` when Desk handed the session its task. */
  | { kind: "user"; line: UserLine; handed: boolean }
  | { kind: "letter"; letter: Letter; open: boolean }
  | { kind: "text"; line: AgentLine; live: boolean }
  | { kind: "run"; run: ToolRun; open: boolean }
  /** A line of an opened run, along its rail under the gray line. */
  | { kind: "work"; line: ThreadLine }
  | { kind: "part"; line: HideLine }
  | { kind: "views"; lines: ViewLine[] }
  | { kind: "question"; line: QuestionLine }
  /** Where the working line and the question card stand; `live` is the running turn's call still running, drawn under the owl. */
  | { kind: "foot"; live: ToolLine | null };

export type ConversationRow = { key: string; turn: string; gap: RowGap } & RowOf;

const running = (line: ThreadLine): line is ToolLine => line.kind === "tool" && (line.status === "pending" || line.status === "in_progress");

/**
 * The rows of a conversation's turns (mockup v3 sections 1, 2, 5), one at a time so a long turn is never one row
 * (issue 3101). Each turn draws its message (a letter as its band), then its items in order: each piece of words, each
 * run as its gray line with its lines under it while open, each letter as its band and its whole text in it while
 * open, a new session or a mission, the views it opened as one row of tiles, and each question line.
 *
 * `open` holds what the operator unfolded, runs and letters by key, outside any row, so a fold outlives its rows
 * leaving the viewport. While the session works (`working`), the running turn's call still running leaves its run's
 * gray line for the foot, under the owl, and a run left with nothing draws no gray line; the foot stands while
 * `foot`. A line's row is keyed by the line's id, a run's by its first line's, so nothing moves as the session writes.
 */
export function conversationRows(turns: readonly ConversationTurn[], { working, open, handed, foot }: { working: boolean; open: ReadonlySet<string>; handed: boolean; foot: boolean }): ConversationRow[] {
  const rows: ConversationRow[] = [];
  let live: ToolLine | null = null;
  turns.forEach((turn, index) => {
    let first = true;
    const push = (row: { key: string } & RowOf, member = false) => {
      const gap: RowGap = first ? (rows.length === 0 ? "none" : "turn") : member ? "member" : "part";
      first = false;
      rows.push({ ...row, turn: turn.key, gap });
    };
    if (turn.user) {
      if (turn.user.origin === "letter") {
        const letter = receivedLetter(turn.user);
        push({ key: letter.key, kind: "letter", letter, open: open.has(letter.key) });
      } else push({ key: turn.user.id, kind: "user", line: turn.user, handed: handed && index === 0 });
    }
    const current = working && index === turns.length - 1;
    turn.items.forEach((item, at) => {
      switch (item.kind) {
        case "text":
          push({ key: item.line.id, kind: "text", line: item.line, live: current });
          return;
        case "run": {
          let run = item.run;
          if (current && at === turn.items.length - 1) {
            const newest = run.lines.findLast((line) => line.kind === "tool");
            if (newest && running(newest)) {
              live = newest;
              const rest = run.lines.filter((line) => line !== newest);
              if (rest.length === 0) return;
              run = { ...runOf(turn.key, rest), key: run.key };
            }
          }
          const unfolded = open.has(run.key);
          push({ key: run.key, kind: "run", run, open: unfolded });
          if (unfolded) run.lines.forEach((line) => push({ key: line.id, kind: "work", line }, true));
          return;
        }
        case "letter":
          push({ key: item.letter.key, kind: "letter", letter: item.letter, open: open.has(item.letter.key) });
          return;
        case "part":
          push({ key: item.line.id, kind: "part", line: item.line });
          return;
        case "views":
          push({ key: `${item.lines[0]!.id}~views`, kind: "views", lines: item.lines });
          return;
        case "question":
          push({ key: item.line.id, kind: "question", line: item.line });
          return;
      }
    });
  });
  if (foot) rows.push({ key: "~foot", turn: "", gap: rows.length ? "foot" : "none", kind: "foot", live });
  return rows;
}

/** How tall a row is taken to be before it is drawn and measured, in pixels: close enough that a jump lands near. */
export function rowEstimate(row: ConversationRow | undefined): number {
  if (!row) return 0;
  const gap = { none: 0, turn: 16, part: 8, member: 4, foot: 12 }[row.gap];
  switch (row.kind) {
    case "user":
      return gap + 22 * Math.max(1, Math.ceil(row.line.text.length / 60)) + 16;
    case "text": {
      const lines = row.line.text.split("\n").reduce((sum, line) => sum + Math.max(1, Math.ceil(line.length / 80)), 0);
      return gap + 26 * lines;
    }
    case "letter":
      return gap + 28 + (row.open ? 24 * Math.max(1, row.letter.text.split("\n").length) : 0);
    case "views":
      return gap + 124;
    case "question":
      return gap + 22 + 22 * row.line.questions.length;
    case "foot":
      return gap + 22 + (row.live ? 22 : 0);
    default:
      return gap + 22;
  }
}
