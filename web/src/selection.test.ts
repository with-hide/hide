import { describe, expect, it } from "vitest";
import { dedent, followScroll, movedSelection, rowContinues, selectionToText, type CellRow } from "./selection";

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

describe("followScroll", () => {
  const screen = ["a", "b", "c", "d", "e", "f"];

  it("finds the rows where the wheel moved the text", () => {
    // The text scrolled up by two rows: what was rows 3-4 is now rows 1-2.
    expect(followScroll(["d", "e"], 3, screen.slice(2).concat(["x", "y"]), -2)).toBe(-2);
    // It scrolled down by one row.
    expect(followScroll(["b", "c"], 1, ["z"].concat(screen.slice(0, 5)), 1)).toBe(1);
  });

  it("finds a shift smaller than the wheel asked for at the end of the history", () => {
    expect(followScroll(["d", "e"], 3, screen.slice(1).concat(["x"]), -5)).toBe(-1);
  });

  it("reads blank rows as the shift the wheel asked for", () => {
    expect(followScroll(["", ""], 2, ["a", "", "", "", "", ""], 1)).toBe(1);
  });

  it("keeps the part of the selection still on screen", () => {
    // The first row left the top; the second is now row 0.
    expect(followScroll(["a", "b"], 0, ["b", "c", "d", "e", "f", "g"], -1)).toBe(-1);
  });

  it("gives up when the text is no longer on screen", () => {
    expect(followScroll(["a", "b"], 0, ["m", "n", "o", "p", "q", "r"], 3)).toBeNull();
  });

  it("does not take blank rows alone for the text it followed", () => {
    expect(followScroll(["foo", ""], 10, ["", "a", "b", "c", "d", "e"], -50)).toBeNull();
  });

  it("never takes a shift against the direction the wheel turned", () => {
    const rule = "----";
    expect(followScroll([rule], 2, [rule, "a", "b", "c", "d", "e"], 2)).toBeNull();
  });

  it("takes repeated rows at the shift the wheel asked for", () => {
    const rule = "----";
    expect(followScroll([rule], 2, [rule, "a", rule, "b", rule, "c"], -2)).toBe(-2);
    expect(followScroll([rule], 2, [rule, "a", rule, "b", rule, "c"], 2)).toBe(2);
    expect(followScroll([rule], 2, [rule, "a", rule, "b", rule, "c"], 3)).toBe(2);
  });
});

describe("movedSelection", () => {
  const start = { x: 3, y: 4 };
  const end = { x: 7, y: 6 };

  it("moves the whole selection when it stays on screen", () => {
    expect(movedSelection(start, end, 2, 20, 10)).toEqual({ column: 3, row: 6, length: 24 });
    expect(movedSelection(start, end, -2, 20, 10)).toEqual({ column: 3, row: 2, length: 24 });
  });

  it("starts at column 0 when the top scrolled off and ends at the last column when the bottom did", () => {
    expect(movedSelection(start, end, -5, 20, 10)).toEqual({ column: 0, row: 0, length: 17 });
    expect(movedSelection(start, end, 14, 20, 10)).toEqual({ column: 3, row: 18, length: 17 });
  });

  it("selects nothing once every row left the screen", () => {
    expect(movedSelection(start, end, -7, 20, 10)).toBeNull();
    expect(movedSelection(start, end, 16, 20, 10)).toBeNull();
  });
});
