// The text a drag selection copies.
//
// A pane holder's frames redraw every row in place and padded to the full
// width (`CSI row;1H` then `width` cells, autowrap off), so xterm.js never
// soft-wraps a row itself: every padding space is a written cell its own
// `getSelection()` keeps, and no row is ever `isWrapped`. The copy is
// therefore assembled here the way a native terminal would:
// trailing whitespace trimmed from every row, and a row whose last cell is
// non-blank read as continuing onto the next row, joined with no break.
// Where the pane's holder said which rows soft-wrap (`paneRowWraps` in
// `terminalWrap.ts`, the same answer a link's join reads), a full row the
// holder said ends with a line break is kept apart; without it, and in an
// agent's pane, a hard line that exactly fills the width is joined too. The copy is then dedented: the
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
 * whether each soft-wraps, or is null when the last-column rule holds. A
 * rectangular selection is not represented.
 */
export function selectionToText(rows: CellRow[], startColumn: number, endColumn: number, wraps: boolean[] | null = null): string {
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
 * How far the viewport's rows moved under a selection. `selected` is the
 * text of the selection's rows (trailing whitespace trimmed) when it was
 * made, `first` the screen row of the first, `screen` every row's text now,
 * `expected` the shift the wheel asked for (positive moves text down). The
 * answer is the shift whose rows still on screen all read as they did,
 * nearest `expected` first and never against its direction, so repeated rows
 * resolve to the shift the wheel asked for. A selection with text needs a row of text to overlap: blank
 * rows alone match anywhere. Null when none of it is on screen any more or
 * no shift reads the same.
 */
export function followScroll(selected: string[], first: number, screen: string[], expected: number): number | null {
  const reach = screen.length + selected.length;
  const shifts: number[] = [];
  for (let shift = -reach; shift <= reach; shift += 1) shifts.push(shift);
  shifts.sort((a, b) => Math.abs(a - expected) - Math.abs(b - expected) || Math.abs(a) - Math.abs(b));
  const hasText = selected.some((text) => text !== "");
  for (const shift of shifts) {
    // Text moves the way the wheel turned; a repeated row never says otherwise.
    if (expected !== 0 && Math.sign(shift) === -Math.sign(expected)) continue;
    let overlap = 0;
    let overlapText = 0;
    let same = true;
    for (const [index, text] of selected.entries()) {
      const row = first + shift + index;
      if (row < 0 || row >= screen.length) continue;
      overlap += 1;
      if (text !== "") overlapText += 1;
      if (screen[row] !== text) {
        same = false;
        break;
      }
    }
    if (same && overlap > 0 && (!hasText || overlapText > 0)) return shift;
  }
  return null;
}

/**
 * The `select(column, row, length)` arguments for a selection from `start` to
 * `end` (exclusive) moved down by `shift` rows on a screen of `rows` x
 * `cols`: the part still on screen, a cut top starting at column 0 and a cut
 * bottom running to the last column. Null when none of it is on screen.
 */
export function movedSelection(
  start: { x: number; y: number },
  end: { x: number; y: number },
  shift: number,
  rows: number,
  cols: number,
): { column: number; row: number; length: number } | null {
  const first = start.y + shift;
  const last = end.y + shift;
  if (last < 0 || first > rows - 1) return null;
  const top = Math.max(first, 0);
  const bottom = Math.min(last, rows - 1);
  const startX = first < 0 ? 0 : start.x;
  const endX = last > rows - 1 ? cols : end.x;
  const length = (bottom - top) * cols + endX - startX;
  return length > 0 ? { column: startX, row: top, length } : null;
}
