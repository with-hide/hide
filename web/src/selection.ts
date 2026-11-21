// The text a drag selection copies.
//
// A pane holder's frames redraw every row in place and padded to the full
// width (`CSI row;1H` then `width` cells, autowrap off), so xterm.js never
// soft-wraps a row itself: every padding space is a written cell its own
// `getSelection()` keeps, and no row is ever `isWrapped`. The copy is
// therefore assembled here the way a native terminal would:
// trailing whitespace trimmed from every row, and a row whose last cell is
// non-blank read as continuing onto the next row, joined with no break.
// Where the pane's holder said a row does not run on (`paneRowWraps` in
// `terminalWrap.ts`, the same answer a link's join reads: a line break in a
// pane with no agent, or a link that ends at the row's edge), a full row is
// kept apart; where nothing was said, as for an agent's own line break, a
// hard line that exactly fills the width is joined too. The copy is then dedented: the
// leading whitespace every non-blank line shares is a margin the program
// drew, not text, while the indentation lines have relative to one another
// is kept (user: "그 줄 앞에 공백있으면 앞에서 trim까지 해줘"). A single line
// therefore loses its leading spaces entirely. A drag that starts past
// column 0 cuts its first line short, so that line's leading whitespace is
// not a margin and does not decide one: the other lines do, and the first
// line gives up whatever part of the margin it still has.

/**
 * One row as xterm reports its cells: the chars of each column, `""` for a
 * cell nothing was written to, and `null` for the trailing half of a wide
 * glyph.
 */
export type CellRow = (string | null)[];

/** The part of xterm's buffer line a row is read from. */
export type BufferLineCells = { getCell(x: number): { getWidth(): number; getChars(): string } | undefined };

/** A buffer line as cells: its chars, `""` where nothing was written, `null` for a wide glyph's trailing half. */
export function bufferRow(line: BufferLineCells | undefined, cols: number): CellRow {
  const cells: CellRow = [];
  for (let x = 0; x < cols; x += 1) {
    const cell = line?.getCell(x);
    cells.push(cell ? (cell.getWidth() === 0 ? null : cell.getChars()) : "");
  }
  return cells;
}

function rowText(cells: CellRow, from: number, to: number): string {
  let text = "";
  for (let column = from; column < to && column < cells.length; column += 1) {
    const chars = cells[column];
    if (chars === null) continue;
    text += chars === "" ? " " : chars;
  }
  return text.trimEnd();
}

/** Whether the row's content reaches its last column, so the next row may continue it. */
export function rowContinues(cells: CellRow): boolean {
  for (let column = cells.length - 1; column >= 0; column -= 1) {
    const chars = cells[column];
    if (chars === null) continue;
    return chars !== undefined && chars.trim() !== "";
  }
  return false;
}

/**
 * The copied text for `rows`, the selected rows top to bottom, taking the
 * first row from `startColumn` and the last row up to `endColumn`
 * (exclusive); a row between is taken whole. `wraps`, beside `rows`, says
 * whether each runs on into the next (null for a row, or for all of them,
 * where the last-column rule holds). A rectangular selection is not
 * represented.
 */
export function selectionToText(rows: CellRow[], startColumn: number, endColumn: number, wraps: (boolean | null)[] | null = null): string {
  let text = "";
  rows.forEach((cells, index) => {
    const from = index === 0 ? startColumn : 0;
    const to = index === rows.length - 1 ? endColumn : cells.length;
    if (index > 0 && !(rowContinues(rows[index - 1] ?? []) && wraps?.[index - 1] !== false)) text += "\n";
    text += rowText(cells, from, to);
  });
  return dedent(text, startColumn > 0);
}

function leading(line: string): string {
  return line.slice(0, line.length - line.trimStart().length);
}

function sharedPrefix(a: string, b: string): string {
  let shared = 0;
  while (shared < a.length && shared < b.length && a[shared] === b[shared]) shared += 1;
  return a.slice(0, shared);
}

/**
 * Removes the leading whitespace every non-blank line shares; blank lines
 * stay blank. With `firstLinePartial` the first line was cut at the
 * selection start, so it has no say in the margin unless it is the only
 * line, and it loses only as much of the margin as it still carries.
 */
export function dedent(text: string, firstLinePartial = false): string {
  const lines = text.split("\n");
  const partial = (index: number) => firstLinePartial && index === 0;
  let margin: string | null = null;
  for (const [index, line] of lines.entries()) {
    if (line.trim() === "" || partial(index)) continue;
    margin = margin === null ? leading(line) : sharedPrefix(margin, leading(line));
    if (margin === "") break;
  }
  if (margin === null && firstLinePartial && lines[0]?.trim() !== "") margin = leading(lines[0] ?? "");
  if (!margin) return text;
  const width = margin.length;
  return lines
    .map((line, index) => {
      if (line.trim() === "") return line;
      return line.slice(partial(index) ? Math.min(width, leading(line).length) : width);
    })
    .join("\n");
}

/**
 * A point between cells in a pane's history: the history row, counted from
 * the oldest row the pane's holder keeps (`terminalView.ts`), and the column
 * boundary it stands before, 0 to the width. A drag selects the text between
 * its press and its pointer, in reading order.
 */
export type HistoryPoint = { row: number; column: number };

/** The two points in reading order. */
export function orderPoints(a: HistoryPoint, b: HistoryPoint): [HistoryPoint, HistoryPoint] {
  return a.row < b.row || (a.row === b.row && a.column <= b.column) ? [a, b] : [b, a];
}

/**
 * The `select(column, row, length)` arguments that draw the part of the
 * selection from `start` to `end` (exclusive) shown on a screen of `rows` x
 * `cols` whose top row is history row `top`: a top cut off starts at column 0
 * and a bottom cut off runs to the last column. Null when none of it shows.
 */
export function drawnSelection(
  start: HistoryPoint,
  end: HistoryPoint,
  top: number,
  rows: number,
  cols: number,
): { column: number; row: number; length: number } | null {
  const first = start.row - top;
  const last = end.row - top;
  if (last < 0 || first > rows - 1) return null;
  const from = Math.max(first, 0);
  const to = Math.min(last, rows - 1);
  const startX = first < 0 ? 0 : start.column;
  const endX = last > rows - 1 ? cols : end.column;
  const length = (to - from) * cols + endX - startX;
  return length > 0 ? { column: startX, row: from, length } : null;
}

/**
 * The rows one autoscroll step moves for a pointer `distance` pixels past the
 * screen's edge: one, and one more for every row of distance, so a pointer
 * held far away moves quickly, never more than a screen less a row, so no
 * row passes unseen.
 */
export function autoscrollRows(distance: number, rowHeight: number, rows: number): number {
  const far = rowHeight > 0 ? Math.floor(distance / rowHeight) : 0;
  return Math.max(1, Math.min(1 + far, rows - 1));
}
