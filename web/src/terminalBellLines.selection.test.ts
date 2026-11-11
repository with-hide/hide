import { beforeAll, describe, expect, it } from "vitest";
import { Terminal } from "@xterm/xterm";
import { dimBellLines, forgetBellLines } from "./terminalBellLines";

const SELECTION = 0x456789;
const WORDS = 0x8fb4c8;
const BAND = 0x1a252a;
const COLORS = { foreground: "#8fb4c8", background: "#1a252a" };

type Cell = object;
type Core = {
  buffer: { lines: { get(y: number): { loadCell(x: number, cell: Cell): Cell } } };
  _decorationService: unknown;
};
type Resolver = { resolve(cell: Cell, x: number, y: number, deviceCellWidth: number): void; result: { fg: number; bg: number } };

// The addon's own resolver: the code that turns a cell's colors, its decorations and the
// selection into what the WebGL renderer paints. `vite.config.ts` aliases the xterm
// internals it imports; it is loaded by a computed path because those internals have no
// types of their own, so the type check must not follow it.
let CellColorResolver: new (...args: unknown[]) => Resolver;
let CellData: new () => Cell;
beforeAll(async () => {
  const resolver = "../node_modules/@xterm/addon-webgl/src/CellColorResolver";
  const cellData = "common/buffer/CellData";
  CellColorResolver = ((await import(/* @vite-ignore */ resolver)) as { CellColorResolver: typeof CellColorResolver }).CellColorResolver;
  CellData = ((await import(/* @vite-ignore */ cellData)) as { CellData: typeof CellData }).CellData;
});

/** What the WebGL renderer paints for a cell of the first screen row, with or without it selected. */
function painted(term: Terminal, x: number, y: number, selected: boolean): { fg: number; bg: number } {
  const core = (term as unknown as { _core: Core })._core;
  const colors = {
    selectionBackgroundOpaque: { rgba: ((SELECTION << 8) | 0xff) >>> 0 },
    selectionInactiveBackgroundOpaque: { rgba: ((SELECTION << 8) | 0xff) >>> 0 },
    selectionForeground: undefined,
    ansi: [],
    foreground: { rgba: 0xffffffff },
    background: { rgba: 0x000000ff },
  };
  const resolver = new CellColorResolver(
    term,
    { rawOptions: { fontSize: 15 } } as never,
    { isCellSelected: () => selected } as never,
    core._decorationService as never,
    { dpr: 1, isFocused: true } as never,
    { colors } as never,
  );
  const cell = core.buffer.lines.get(y)!.loadCell(x, new CellData());
  resolver.resolve(cell, x, y, 8);
  return { fg: resolver.result.fg & 0xffffff, bg: resolver.result.bg & 0xffffff };
}

const written = (term: Terminal, data: string) => new Promise<void>((done) => term.write(data, done));

describe("a letter row and the selection, as the WebGL renderer resolves them", () => {
  it("shows the band and the words until it is selected, and the selection color once it is", async () => {
    const term = new Terminal({ rows: 6, cols: 40, scrollback: 0, allowProposedApi: true });
    await written(term, "\x1b[2;1H❯ 🦉 a 진행: 끝\x1b[6;1H");
    dimBellLines(term, COLORS);
    // Row index 1 is the letter row; a cell with text, and one past its end.
    for (const x of [3, 30]) {
      expect(painted(term, x, 1, false)).toMatchObject({ bg: BAND });
      expect(painted(term, x, 1, true)).toMatchObject({ bg: SELECTION });
    }
    expect(painted(term, 3, 1, false).fg).toBe(WORDS);
    // Selecting does not take the words away either.
    expect(painted(term, 3, 1, true).fg).toBe(WORDS);
    // A row with no letter keeps what the agent drew, and its selection shows too.
    expect(painted(term, 3, 3, false).bg).not.toBe(BAND);
    expect(painted(term, 3, 3, true).bg).toBe(SELECTION);
    forgetBellLines(term);
    term.dispose();
  });
});
