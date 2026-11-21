// The doorbell's lines on a pane's screen, drawn as a pale band of blue-grey words.
//
// Hide types `🦉 <sender> <kind>: <summary>` into a Claude Code or Codex pane
// to wake it for a letter (`docs/delivery.md`, The bell line). The agent echoes
// it as a prompt row, `❯ 🦉 …` in Claude Code and `› 🦉 …` in Codex, and it is
// a letter between agents, not something the operator has to act on; what the
// operator owes an answer to is Needs You. The row is quieted, never hidden or
// rewritten: the agent's own TUI draws it, so the shell only lays a band (bottom layer,
// under the pane's drag selection) and a foreground color (top layer) over its
// cells, and the text and the copy stay as drawn.
//
// The pane keeps no scrollback and the holder redraws its screen rows in place,
// so a marker on a row says nothing about what the row shows next. After a
// frame the screen is read again and the decorations are replaced only when the
// rows that carry a bell line moved or a decoration or marker was disposed.
//
// Only the shape of the row is read, never who typed it, so a line a person
// typed that begins with the prompt glyph and an owl is marked too, and a Grok
// row its keyboard focus frames (`│ ❯ 🦉 … │`) is not. Telling them apart needs
// the core to mark the line it typed; that is a follow-up, not this file.
//
// Third limit: the band stays under a drag selection in the WebGL renderer
// only. xterm's DOM renderer, which the pane falls back to when WebGL fails or
// loses its context, paints a decoration over the selection, so a selected
// letter row shows no selection background there (the copy is unaffected).
// Showing it in both needs an overlay apart from decorations.

import type { IBufferLine, IDecoration, IMarker, Terminal } from "@xterm/xterm";
import { AGENT_ADAPTERS } from "./agentAdapters";

/** The echo each agent that types the line declares (`hide-agent-adapter`, `BellEcho`). */
export type BellEchoShape = { prompt: readonly string[]; chrome: readonly string[] };

const escaped = (glyphs: readonly string[]) => glyphs.map((glyph) => glyph.replace(/[\\\]^-]/g, "\\$&")).join("");

/**
 * What the scan reads, built from the declared echoes: a row begins with one
 * of the prompt glyphs, an owl and a space; group 1 is what stands before the
 * owl (Grok leaves a margin before the glyph), whose width is the indent each
 * row the line wraps onto carries. A row that begins with a prompt glyph or
 * with one of the agents' own output glyphs never carries the line on.
 */
export function bellScan(echoes: readonly BellEchoShape[]): { bellRow: RegExp; chrome: RegExp } {
  const prompts = echoes.flatMap((echo) => echo.prompt);
  const chrome = [...prompts, ...echoes.flatMap((echo) => echo.chrome)];
  return {
    bellRow: new RegExp(`^(\\s*[${escaped(prompts)}]\\s*)🦉\\s`, "u"),
    chrome: new RegExp(`[\\s${escaped(chrome)}]`, "u"),
  };
}

const declared = AGENT_ADAPTERS.flatMap((row) => (row.bell_echo ? [row.bell_echo] : []));
const declaredScan = bellScan(declared);

/** A wrapped row leaves this many cells unused at most when the next word fits no better. */
const WRAP_SLACK = 16;

/** The most rows one bell line may run over: about 130 cells, in a pane a few words wide. */
const MAX_ROWS = 6;

/** One screen row as the scan reads it: its text and how many cells it uses. */
export type ScreenRow = { text: string; used: number };

/** Whether a row starts with exactly the bell line's indent and then text rather than chrome. */
function carriesOn(text: string, indent: string, chrome: RegExp): boolean {
  if (!text.startsWith(indent)) return false;
  const [first] = Array.from(text.slice(indent.length, indent.length + 2));
  return first !== undefined && !chrome.test(first);
}

/** The rows, counted from the top of `rows`, that carry a bell line. */
export function bellRows(rows: readonly ScreenRow[], cols: number, scan = declaredScan): number[] {
  const found: number[] = [];
  for (let at = 0; at < rows.length; at += 1) {
    const row = rows[at]!;
    const bell = row.text.includes("🦉") ? scan.bellRow.exec(row.text) : null;
    if (!bell) continue;
    found.push(at);
    const indent = " ".repeat(bell[1]!.length);
    let last = row;
    for (let extra = 1; extra < MAX_ROWS; extra += 1) {
      const next = rows[at + extra];
      if (!next || last.used < cols - WRAP_SLACK || !carriesOn(next.text, indent, scan.chrome)) break;
      found.push(at + extra);
      last = next;
    }
    at = found[found.length - 1]!;
  }
  return found;
}

/** The words and the band behind a letter's rows; the band is opaque, as xterm's background color has no alpha. */
export type BellColors = { foreground: string; background: string };

/** A decoration and the marker it hangs on: disposing a decoration leaves its marker registered in the buffer. */
type Item = { y: number; marker: IMarker; decorations: IDecoration[] };

type Drawn = { rows: number[]; colors: BellColors; cols: number; height: number; items: Item[] };

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
  for (const { marker, decorations } of was.items) {
    for (const decoration of decorations) decoration.dispose();
    marker.dispose();
  }
  drawn.delete(term);
}

/**
 * Whether what was drawn still holds for the rows now: the same rows, width,
 * height and color, and every decoration and marker alive on the line it was
 * made for. A resize, a reset the holder redraws after, or a trim disposes a
 * marker, and the rows read afterwards can be the same ones.
 */
function holds(term: Terminal, was: Drawn, rows: readonly number[], colors: BellColors): boolean {
  if (was.colors.foreground !== colors.foreground || was.colors.background !== colors.background || was.cols !== term.cols || was.height !== term.rows) return false;
  if (was.rows.length !== rows.length || !was.rows.every((row, i) => row === rows[i])) return false;
  const top = term.buffer.active.viewportY;
  return was.items.every(({ y, marker, decorations }) => decorations.every((decoration) => !decoration.isDisposed) && !marker.isDisposed && marker.line === top + y);
}

/** Reads the screen now and draws the bell rows in `colors`; nothing changes while what is drawn holds. */
export function dimBellLines(term: Terminal, colors: BellColors) {
  const rows = bellRows(readRows(term), term.cols);
  const was = drawn.get(term);
  if (was && holds(term, was, rows, colors)) return;
  dispose(term);
  if (rows.length === 0) return;
  const buffer = term.buffer.active;
  const items: Item[] = [];
  for (const y of rows) {
    const marker = term.registerMarker(buffer.viewportY + y - (buffer.baseY + buffer.cursorY));
    if (!marker) continue;
    // The band is on the bottom layer, under the selection the pane draws with
    // `term.select`, and the words on the top layer: a top layer that carries a
    // background color is painted over the selection, a foreground-only one is not.
    const band = term.registerDecoration({ marker, x: 0, width: term.cols, backgroundColor: colors.background, layer: "bottom" });
    const words = term.registerDecoration({ marker, x: 0, width: term.cols, foregroundColor: colors.foreground, layer: "top" });
    const decorations = [band, words].filter((decoration): decoration is IDecoration => decoration !== undefined);
    if (decorations.length === 2) items.push({ y, marker, decorations });
    else {
      for (const decoration of decorations) decoration.dispose();
      marker.dispose();
    }
  }
  drawn.set(term, { rows, colors, cols: term.cols, height: term.rows, items });
}

/** Asks for the screen to be read once at the next animation frame, however many frames came before it. */
export function queueBellLines(term: Terminal, colors: () => BellColors) {
  if (waiting.has(term)) return;
  waiting.set(term, requestAnimationFrame(() => {
    waiting.delete(term);
    dimBellLines(term, colors());
  }));
}

/** Ends the pending read and the decorations of a terminal that is going away. */
export function forgetBellLines(term: Terminal) {
  const frame = waiting.get(term);
  if (frame !== undefined) cancelAnimationFrame(frame);
  waiting.delete(term);
  dispose(term);
}
