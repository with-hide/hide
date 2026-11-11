// The doorbell's lines on a pane's screen, drawn dim.
//
// Hide types `🦉 <sender> <kind>: <summary>` into a Claude Code or Codex pane
// to wake it for a letter (`docs/delivery.md`, The bell line). The agent echoes
// it as a prompt row, `❯ 🦉 …` in Claude Code and `› 🦉 …` in Codex, and it is
// a letter between agents, not something the operator has to act on; what the
// operator owes an answer to is Needs You. The row is dimmed, never hidden or
// rewritten: the agent's own TUI draws it, so the shell only lays a foreground
// color over its cells, and the text, the selection and the copy stay as drawn.
//
// The pane keeps no scrollback and the holder redraws its screen rows in place,
// so a marker on a row says nothing about what the row shows next. After a
// frame the screen is read again and the decorations are replaced only when the
// rows that carry a bell line moved.

import type { IBufferLine, IDecoration, Terminal } from "@xterm/xterm";

/**
 * A prompt glyph, the owl and a space, as Claude Code, Codex and Grok echo the
 * line (Grok leaves a margin before the glyph); group 1 is what stands before
 * the owl, whose width is the indent each row the line wraps onto carries.
 */
const BELL_ROW = /^(\s*[❯›>]\s*)🦉\s/u;

/** What starts a row of an agent's own chrome, which never carries a bell line on. */
const CHROME = /[\s⎿⏺└├│•›❯>◆◈┃]/u;

/** A wrapped row leaves this many cells unused at most when the next word fits no better. */
const WRAP_SLACK = 16;

/** The most rows one bell line may run over: about 130 cells, in a pane a few words wide. */
const MAX_ROWS = 6;

/** One screen row as the scan reads it: its text and how many cells it uses. */
export type ScreenRow = { text: string; used: number };

/** Whether a row starts with exactly the bell line's indent and then text rather than chrome. */
function carriesOn(text: string, indent: string): boolean {
  if (!text.startsWith(indent)) return false;
  const [first] = Array.from(text.slice(indent.length, indent.length + 2));
  return first !== undefined && !CHROME.test(first);
}

/** The rows, counted from the top of `rows`, that carry a bell line. */
export function bellRows(rows: readonly ScreenRow[], cols: number): number[] {
  const found: number[] = [];
  for (let at = 0; at < rows.length; at += 1) {
    const row = rows[at]!;
    const bell = row.text.includes("🦉") ? BELL_ROW.exec(row.text) : null;
    if (!bell) continue;
    found.push(at);
    const indent = " ".repeat(bell[1]!.length);
    let last = row;
    for (let extra = 1; extra < MAX_ROWS; extra += 1) {
      const next = rows[at + extra];
      if (!next || last.used < cols - WRAP_SLACK || !carriesOn(next.text, indent)) break;
      found.push(at + extra);
      last = next;
    }
    at = found[found.length - 1]!;
  }
  return found;
}

type Drawn = { rows: number[]; color: string; cols: number; decorations: IDecoration[] };

const drawn = new WeakMap<Terminal, Drawn>();
const waiting = new WeakMap<Terminal, number>();

/** The cells up to the last one that shows anything; a wide glyph's second cell counts. */
function usedCells(line: IBufferLine, cols: number): number {
  for (let x = cols - 1; x >= 0; x -= 1) {
    const cell = line.getCell(x);
    if (cell && (cell.getChars() !== "" || cell.getWidth() === 0)) return x + 1;
  }
  return 0;
}

function readRows(term: Terminal): ScreenRow[] {
  const buffer = term.buffer.active;
  const rows: ScreenRow[] = [];
  for (let y = 0; y < term.rows; y += 1) {
    const line: IBufferLine | undefined = buffer.getLine(buffer.viewportY + y);
    if (!line) {
      rows.push({ text: "", used: 0 });
      continue;
    }
    const text = line.translateToString(true);
    // Only a bell line or a row carrying one on is asked how wide it runs.
    rows.push({ text, used: text.includes("🦉") || text.startsWith("  ") ? usedCells(line, term.cols) : 0 });
  }
  return rows;
}

function dispose(term: Terminal) {
  const was = drawn.get(term);
  if (!was) return;
  for (const decoration of was.decorations) decoration.dispose();
  drawn.delete(term);
}

/** Reads the screen now and draws the bell rows in `color`; nothing changes when they did not move. */
export function dimBellLines(term: Terminal, color: string) {
  const rows = bellRows(readRows(term), term.cols);
  const was = drawn.get(term);
  if (
    was &&
    was.color === color &&
    was.cols === term.cols &&
    was.rows.length === rows.length &&
    was.rows.every((row, i) => row === rows[i])
  ) {
    return;
  }
  dispose(term);
  if (rows.length === 0) return;
  const buffer = term.buffer.active;
  const decorations: IDecoration[] = [];
  for (const y of rows) {
    const marker = term.registerMarker(buffer.viewportY + y - (buffer.baseY + buffer.cursorY));
    if (!marker) continue;
    const decoration = term.registerDecoration({ marker, x: 0, width: term.cols, foregroundColor: color, layer: "top" });
    if (decoration) decorations.push(decoration);
    else marker.dispose();
  }
  drawn.set(term, { rows, color, cols: term.cols, decorations });
}

/** Asks for the screen to be read once at the next animation frame, however many frames came before it. */
export function queueBellLines(term: Terminal, color: () => string) {
  if (waiting.has(term)) return;
  waiting.set(term, requestAnimationFrame(() => {
    waiting.delete(term);
    dimBellLines(term, color());
  }));
}

/** Ends the pending read and the decorations of a terminal that is going away. */
export function forgetBellLines(term: Terminal) {
  const frame = waiting.get(term);
  if (frame !== undefined) cancelAnimationFrame(frame);
  waiting.delete(term);
  dispose(term);
}
