import { describe, expect, it } from "vitest";
import { CJK_FONTS, cellBox, cjkScriptOrder, cssFamily, fitScale, fontSystemOf, primaryFamily } from "./terminalFonts";

describe("terminal fonts", () => {
  it("names the system whose fonts a platform has", () => {
    expect(fontSystemOf("darwin")).toBe("mac");
    expect(fontSystemOf("MacIntel")).toBe("mac");
    expect(fontSystemOf("macOS")).toBe("mac");
    expect(fontSystemOf("win32")).toBe("windows");
    expect(fontSystemOf("Win32")).toBe("windows");
    expect(fontSystemOf("Windows")).toBe("windows");
    expect(fontSystemOf("linux")).toBe("linux");
    expect(fontSystemOf("Linux x86_64")).toBe("linux");
    expect(fontSystemOf("")).toBe("linux");
  });

  it("puts the first CJK language's script first and keeps the rest in the default order", () => {
    expect(cjkScriptOrder(["en-US"])).toEqual(["sc", "ja", "tc", "kr"]);
    expect(cjkScriptOrder([])).toEqual(["sc", "ja", "tc", "kr"]);
    expect(cjkScriptOrder(["ko-KR", "en"])).toEqual(["kr", "sc", "ja", "tc"]);
    expect(cjkScriptOrder(["en", "ja"])).toEqual(["ja", "sc", "tc", "kr"]);
    expect(cjkScriptOrder(["zh-TW"])).toEqual(["tc", "sc", "ja", "kr"]);
    expect(cjkScriptOrder(["zh-Hant-HK"])).toEqual(["tc", "sc", "ja", "kr"]);
    expect(cjkScriptOrder(["zh-CN"])).toEqual(["sc", "ja", "tc", "kr"]);
    expect(cjkScriptOrder(["zh"])).toEqual(["sc", "ja", "tc", "kr"]);
    // Look-alike tags are other languages: Konkani, Javanese, Zhuang.
    expect(cjkScriptOrder(["kok", "jv", "za"])).toEqual(["sc", "ja", "tc", "kr"]);
  });

  it("names each system's own font for every script", () => {
    for (const fonts of Object.values(CJK_FONTS)) {
      expect(fonts.map((font) => font.script).sort()).toEqual(["ja", "kr", "sc", "tc"]);
    }
  });

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

  it("fits a CJK glyph to two cells unless that would rise out of the row", () => {
    const cell = { width: 8, above: 13.2, below: 3.3 };
    // Apple SD Gothic Neo's 가 is 0.865 em wide and rises 0.8 em: two cells
    // would take 132 %, but 13.2 px of room above the baseline allows 117.8 %.
    expect(fitScale(cell, { advance: 0.865, ascent: 0.8, descent: 0.05 }, 14, 2)).toBe(1.178);
    // A shallow glyph is held by the width alone.
    expect(fitScale(cell, { advance: 0.865, ascent: 0.6, descent: 0.05 }, 14, 2)).toBe(1.321);
  });

  it("shrinks a 1 em icon to one cell", () => {
    expect(fitScale({ width: 8, above: 13.2, below: 3.3 }, { advance: 1, ascent: 0.8, descent: 0.2 }, 14, 1)).toBe(0.571);
  });
});
