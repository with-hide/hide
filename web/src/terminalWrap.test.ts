import { describe, expect, it } from "vitest";
import { followWraps, forgetWraps, parseWraps, terminalWraps, WRAP_OSC, wrapsOf, type OscParser } from "./terminalWrap";

describe("parseWraps", () => {
  it("reads which rows soft-wrap from the holder's digits", () => {
    // Rows 0, 1 and 9 of a 4x12 screen, as hide-holder's encoder writes them.
    const wraps = parseWraps("4;12;302")!;
    expect(wraps.cols).toBe(4);
    expect(wraps.rows).toBe(12);
    expect([...wraps.bits].flatMap((bit, row) => (bit ? [row] : []))).toEqual([0, 1, 9]);
    expect([...parseWraps("29;4;")!.bits]).toEqual([0, 0, 0, 0]);
  });

  it("refuses data the holder does not write", () => {
    expect(parseWraps("")).toBeNull();
    expect(parseWraps("29;4")).toBeNull();
    expect(parseWraps("29;4;G")).toBeNull();
    // More digits than the rows hold.
    expect(parseWraps("29;4;11")).toBeNull();
  });
});

describe("wrapsOf", () => {
  it("answers only for the grid the wraps were said for", () => {
    const wraps = parseWraps("29;4;1");
    expect(wrapsOf(wraps, 29, 4, 0, 2)).toEqual([true, false, false]);
    expect(wrapsOf(wraps, 30, 4, 0, 2)).toBeNull();
    expect(wrapsOf(wraps, 29, 5, 0, 2)).toBeNull();
    expect(wrapsOf(null, 29, 4, 0, 2)).toBeNull();
  });
});

describe("followWraps", () => {
  it("keeps the last wraps the terminal's stream carried until it is reset", () => {
    let handler: ((data: string) => boolean | Promise<boolean>) | null = null;
    const parser: OscParser = {
      registerOscHandler(ident, callback) {
        expect(ident).toBe(WRAP_OSC);
        handler = callback;
        return { dispose() {} };
      },
    };
    const term = { parser };
    followWraps(term);
    expect(terminalWraps(term)).toBeNull();
    expect(handler!("29;4;1")).toBe(true);
    expect(terminalWraps(term)?.bits[0]).toBe(1);
    forgetWraps(term);
    expect(terminalWraps(term)).toBeNull();
  });
});
