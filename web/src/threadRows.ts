// A session thread's conversation as the rows it draws, one at a time (issue 3101): the operator's words, each answer
// message, each line of a turn's work, the fold line, a folded turn's parts and the foot. The view mounts only the
// rows near its scroll viewport, so a row is never a whole turn: one long turn would otherwise stay drawn whole.

import type { ThreadHidePart, ThreadLine, ThreadTurn } from "./desk";

/** Views opened one after another, which share one row of tiles. */
export type ViewLine = Extract<ThreadLine, { kind: "hide" }> & { part: Extract<ThreadHidePart, { kind: "view" }> };

/**
 * The room a row keeps above it, as the conversation spaced the same things when a turn was one block: `turn` between
 * turns, `part` between the parts of one turn, `member` between the lines of one group (a turn's work, what a running
 * turn keeps in view, a folded turn's parts), `foot` above the working line and the question card, `none` first.
 */
export type RowGap = "none" | "turn" | "part" | "member" | "foot";

type Agent = Extract<ThreadLine, { kind: "agent" }>;
type Hide = Extract<ThreadLine, { kind: "hide" }>;

/** A row's own part: what it draws, by its kind. */
type RowOf =
  | { kind: "user"; line: Extract<ThreadLine, { kind: "user" }>; handed: boolean }
  | { kind: "toggle"; of: ThreadTurn; live: boolean; open: boolean }
  /** A line of a turn's work, or views side by side; `rail` while it stands in the unfolded work's rail. */
  | { kind: "work"; row: ThreadLine | ViewLine[]; rail: boolean }
  | { kind: "answer"; line: Agent; live: boolean }
  | { kind: "part"; line: Hide }
  | { kind: "outputs"; lines: Hide[] }
  | { kind: "newest"; call: NonNullable<ThreadTurn["call"]> }
  | { kind: "foot" };

export type ThreadRow = { key: string; turn: string; gap: RowGap } & RowOf;

/** A turn's work as its open fold draws it: views opened one after another share one row of tiles (feedback 2, frame M 나). */
export function workRows(work: readonly ThreadLine[]): (ThreadLine | ViewLine[])[] {
  const rows: (ThreadLine | ViewLine[])[] = [];
  for (const line of work) {
    const last = rows.at(-1);
    if (!isViewLine(line)) rows.push(line);
    else if (Array.isArray(last)) last.push(line);
    else rows.push([line]);
  }
  return rows;
}

function isViewLine(line: ThreadLine): line is ViewLine {
  return line.kind === "hide" && line.part.kind === "view";
}

const workKey = (row: ThreadLine | ViewLine[]) => (Array.isArray(row) ? row[0]!.id : row.id);

/**
 * The rows of a conversation's turns, in the order a turn draws its parts (B25, B26, D-75). A finished turn: the
 * operator's words, `작업 · …` (its work under it while unfolded), the answer messages, and while folded the parts
 * that lead somewhere then its outputs as one row. The running turn (`running`, the last): the words, its work while
 * unfolded or else what it said and the parts it made, the answer, `작업 중 · …` and its newest call while folded.
 * `open` holds the turns whose work the operator unfolded, by key, outside any row, so a fold outlives its rows
 * leaving the viewport; `foot` adds the row the working line and the question card stand in.
 *
 * A line's row is keyed by the line's id whatever part of the turn it stands in, so an answer that becomes work when
 * a call follows it keeps its row.
 */
export function threadRows(turns: readonly ThreadTurn[], { running, open, handed, foot }: { running: boolean; open: ReadonlySet<string>; handed: boolean; foot: boolean }): ThreadRow[] {
  const rows: ThreadRow[] = [];
  turns.forEach((turn, index) => {
    const live = running && index === turns.length - 1;
    const unfolded = open.has(turn.key) && turn.work.length > 0;
    let first = true;
    const push = (row: { key: string } & RowOf, member = false) => {
      const gap: RowGap = first ? (rows.length === 0 ? "none" : "turn") : member ? "member" : "part";
      first = false;
      rows.push({ ...row, turn: turn.key, gap });
    };
    const group = (lines: readonly (ThreadLine | ViewLine[])[], rail: boolean) => {
      lines.forEach((row, at) => push({ key: workKey(row), kind: "work", row, rail }, at > 0));
    };
    const toggle = () => {
      if (turn.work.length) push({ key: `${turn.key}~toggle`, kind: "toggle", of: turn, live, open: unfolded });
    };
    const answer = () => turn.answer.forEach((line) => push({ key: line.id, kind: "answer", line, live }));
    if (turn.user) push({ key: turn.user.id, kind: "user", line: turn.user, handed: handed && index === 0 });
    if (live) {
      if (unfolded) group(workRows(turn.work), true);
      else group(workRows(turn.work.filter((line) => line.kind === "agent" || (line.kind === "hide" && line.part.kind !== "plan"))), false);
      answer();
      toggle();
      if (!unfolded && turn.call) push({ key: `${turn.key}~newest`, kind: "newest", call: turn.call });
      return;
    }
    toggle();
    if (unfolded) group(workRows(turn.work), true);
    answer();
    if (!unfolded && turn.parts.length) {
      const rest = turn.parts.filter((line) => line.part.kind !== "view");
      const outputs = turn.parts.filter((line) => line.part.kind === "view");
      rest.forEach((line, at) => push({ key: line.id, kind: "part", line }, at > 0));
      if (outputs.length) push({ key: `${turn.key}~outputs`, kind: "outputs", lines: outputs }, rest.length > 0);
    }
  });
  if (foot) rows.push({ key: "~foot", turn: "", gap: rows.length ? "foot" : "none", kind: "foot" });
  return rows;
}

/** How tall a row is taken to be before it is drawn and measured, in pixels: close enough that a jump lands near. */
export function rowEstimate(row: ThreadRow | undefined): number {
  if (!row) return 0;
  const gap = { none: 0, turn: 16, part: 8, member: 4, foot: 12 }[row.gap];
  switch (row.kind) {
    case "user":
      return gap + 22 * Math.max(1, Math.ceil(row.line.text.length / 60)) + 16;
    case "answer": {
      const lines = row.line.text.split("\n").reduce((sum, line) => sum + Math.max(1, Math.ceil(line.length / 80)), 0);
      return gap + 26 * lines;
    }
    case "outputs":
      return gap + 124;
    case "work":
      return gap + (Array.isArray(row.row) ? 124 : row.row.kind === "agent" ? 26 * Math.max(1, Math.ceil(row.row.text.length / 80)) : 20);
    default:
      return gap + 22;
  }
}
