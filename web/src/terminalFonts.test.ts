import { describe, expect, it } from "vitest";
import { cellBox, cssFamily, fitScale, primaryFamily } from "./terminalFonts";

describe("terminal fonts", () => {
  it("puts the operator's font, quoted, ahead of the token", () => {
    const token = "ui-monospace, SFMono-Regular, Menlo, Monaco, monospace";
    expect(primaryFamily("", token)).toBe(token);
    expect(primaryFamily("  ", token)).toBe(token);
    expect(primaryFamily("JetBrains Mono", token)).toBe(`"JetBrains Mono", ${token}`);
    expect(cssFamily('Odd "Name", Mono\\')).toBe('"Odd \\"Name\\", Mono\\\\"');
  });

  it("lays a cell out the way the WebGL renderer does", () => {
    // Menlo at 14px on a 2x display: W is 8.43 wide, so a cell is 16 device
    // pixels; the box is ceil(16.3 * 2) = 33 high, the baseline 6.6 above its bottom.
    const menlo = cellBox({ width: 8.4287, ascent: 13, descent: 3.3 }, 2);
    expect(menlo.width).toBe(8);
    expect(menlo.above).toBeCloseTo(13.2);
    expect(menlo.below).toBeCloseTo(3.3);
    // A taller line centres the box: 17 high in a cell of 20, two pixels down.
    const tall = cellBox({ width: 8.43, ascent: 13, descent: 3.3 }, 1, 1.2);
    expect(tall.width).toBe(8);
    expect(tall.above).toBeCloseTo(15.7);
    expect(tall.below).toBeCloseTo(4.3);
  });

  it("holds a tall glyph inside its row", () => {
    // Two cells would take 132 %, but 13.2 px of room above the baseline allows 117.8 %.
    expect(fitScale({ width: 8, above: 13.2, below: 3.3 }, { advance: 0.865, ascent: 0.8, descent: 0.05 }, 14, 2)).toBe(1.178);
  });

  it("shrinks a 1 em icon to one cell", () => {
    expect(fitScale({ width: 8, above: 13.2, below: 3.3 }, { advance: 1, ascent: 0.8, descent: 0.2 }, 14, 1)).toBe(0.571);
  });
});
