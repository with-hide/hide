import { describe, expect, it } from "vitest";
import { autoscrollRows, dedent, drawnSelection, orderPoints, rowContinues, selectionToText, type CellRow } from "./selection";

/** A row of `width` cells holding `text`, padded with written spaces the way a holder's frame draws it. */
function row(text: string, width: number): CellRow {
  const cells: CellRow = [];
  for (const char of text) cells.push(char);
  while (cells.length < width) cells.push(" ");
  return cells;
}

describe("selectionToText", () => {
  it("trims the padding a frame writes to the column edge", () => {
    expect(selectionToText([row("short", 12)], 0, 12)).toBe("short");
  });

  // hide-private issue 312: the copy reads the same wraps a link's join does.
  it("keeps a full row the holder says ends with a line break apart from the next", () => {
    const rows = [row("http://127.0.0.1:41811/a.html", 29), row("osc8 label", 29)];
    expect(selectionToText(rows, 0, 29, [false, false])).toBe("http://127.0.0.1:41811/a.html\nosc8 label");
    expect(selectionToText(rows, 0, 29, [true, false])).toBe("http://127.0.0.1:41811/a.htmlosc8 label");
    expect(selectionToText(rows, 0, 29, null)).toBe("http://127.0.0.1:41811/a.htmlosc8 label");
  });

  // hide-private issue 591: Claude Code's rows for a URL it broke and one
  // that fills its row, with the link end the holder says for row 4 (as
  // `terminalLinkProvider.test.ts` draws them).
  it("keeps a full row whose link ended apart and joins a URL an agent broke", () => {
    const rows = [
      row("⏺ Long: https://example.com/agent-broke-this-url/aaaaaaaaaa/", 60),
      row("  bbbbbbbbbb/cccccccccc/dddddddddd/end", 60),
      row("", 60),
      row("  Full:", 60),
      row("  https://example.com/fills-the-row-exactly-xxxxxxxxxxxxxxxx", 60),
      row("  nextword after it", 60),
    ];
    const wraps = [null, null, null, null, false, null];
    expect(selectionToText(rows.slice(3), 0, 60, wraps.slice(3))).toBe("Full:\nhttps://example.com/fills-the-row-exactly-xxxxxxxxxxxxxxxx\nnextword after it");
    expect(selectionToText(rows.slice(0, 2), 0, 60, wraps.slice(0, 2))).not.toContain("\n");
  });

  it("joins a row that fills its width onto the next without a break", () => {
    const rows = [row("abcdefghijkl", 12), row("mnop", 12), row("next", 12)];
    expect(selectionToText(rows, 0, 12)).toBe("abcdefghijklmnop\nnext");
  });

  it("keeps an empty row as a blank line", () => {
    const rows = [row("one", 8), row("", 8), row("two", 8)];
    expect(selectionToText(rows, 0, 8)).toBe("one\n\ntwo");
  });

  it("takes the first row from the start column and the last row up to the end column", () => {
    const rows = [row("hello world", 12), row("second line", 12)];
    expect(selectionToText(rows, 6, 6)).toBe("world\nsecond");
  });

  it("reads a wide glyph in the last column as a full row and never copies its trailing half", () => {
    const wide: CellRow = ["a", "b", "가", null];
    expect(rowContinues(wide)).toBe(true);
    expect(selectionToText([wide, row("c", 4)], 0, 4)).toBe("ab가c");
  });

  it("removes the margin every line shares and keeps the indentation between lines", () => {
    const rows = [row("  fn main() {", 20), row("      body();", 20), row("", 20), row("  }", 20)];
    expect(selectionToText(rows, 0, 20)).toBe("fn main() {\n    body();\n\n}");
  });

  it("strips a single line's leading spaces entirely", () => {
    expect(selectionToText([row("    indented", 16)], 0, 16)).toBe("indented");
  });

  it("dedents by the other lines when the drag starts past the first line's margin", () => {
    const rows = [row("  fn main() {", 20), row("      body();", 20), row("  }", 20)];
    expect(selectionToText(rows, 2, 20)).toBe("fn main() {\n    body();\n}");
    expect(selectionToText(rows, 5, 20)).toBe("main() {\n    body();\n}");
  });

  it("takes only what the cut first line still has of the margin", () => {
    const rows = [row("  fn main() {", 20), row("      body();", 20), row("  }", 20)];
    expect(selectionToText(rows, 1, 20)).toBe("fn main() {\n    body();\n}");
  });

  it("treats a cut first row that continues onto the next as one cut line", () => {
    const rows = [row("  abcdefghij", 12), row("kl", 12), row("  end", 12)];
    expect(selectionToText(rows, 4, 12)).toBe("cdefghijkl\nend");
  });

  it("strips a lone cut line's leading spaces entirely", () => {
    expect(selectionToText([row("    indented", 16)], 2, 16)).toBe("indented");
  });

  it("keeps a selection whose lines share no margin as it is", () => {
    expect(dedent("a\n  b")).toBe("a\n  b");
    expect(dedent("")).toBe("");
  });

  it("treats a cell nothing was written to as a space inside a row and as nothing at its end", () => {
    const rows: CellRow = ["a", "", "b", "", ""];
    expect(selectionToText([rows], 0, 5)).toBe("a b");
    expect(rowContinues(rows)).toBe(false);
  });
});

describe("orderPoints", () => {
  it("puts the earlier point first, by row and then by column", () => {
    const a = { row: 7, column: 3 };
    const b = { row: 5, column: 9 };
    expect(orderPoints(a, b)).toEqual([b, a]);
    expect(orderPoints({ row: 5, column: 4 }, { row: 5, column: 2 })).toEqual([{ row: 5, column: 2 }, { row: 5, column: 4 }]);
  });
});

describe("drawnSelection", () => {
  const start = { row: 104, column: 3 };
  const end = { row: 106, column: 7 };

  it("draws the whole selection where the screen shows it", () => {
    expect(drawnSelection(start, end, 102, 20, 10)).toEqual({ column: 3, row: 2, length: 24 });
  });

  it("starts at column 0 when the top is above the screen and ends at the last column when the bottom is below", () => {
    expect(drawnSelection(start, end, 105, 20, 10)).toEqual({ column: 0, row: 0, length: 17 });
    expect(drawnSelection(start, end, 86, 20, 10)).toEqual({ column: 3, row: 18, length: 17 });
  });

  it("draws nothing once every row is off the screen", () => {
    expect(drawnSelection(start, end, 107, 20, 10)).toBeNull();
    expect(drawnSelection(start, end, 84, 20, 10)).toBeNull();
  });
});

describe("autoscrollRows", () => {
  it("moves one row at the edge, one more per row of distance, and never a whole screen", () => {
    expect(autoscrollRows(3, 16, 40)).toBe(1);
    expect(autoscrollRows(40, 16, 40)).toBe(3);
    expect(autoscrollRows(10_000, 16, 40)).toBe(39);
  });
});
