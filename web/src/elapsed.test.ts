import { describe, expect, it } from "vitest";
import { formatElapsed, formatSpan } from "./components/elapsed";

describe("formatElapsed", () => {
  it("moves at the second, the minute, the hour and the day", () => {
    const cases: [number, string][] = [
      [0, "0s"],
      [59_999, "59s"],
      [60_000, "1m"],
      [3_599_999, "59m"],
      [3_600_000, "1h"],
      [86_399_999, "23h"],
      [86_400_000, "1d"],
      [3 * 86_400_000, "3d"],
    ];
    for (const [ms, text] of cases) expect(formatElapsed("en", ms)).toBe(text);
  });

  it("reads a moment from the future as no time at all", () => {
    expect(formatElapsed("en", -5_000)).toBe("0s");
  });

  it("writes the unit in the selected language", () => {
    expect([42_000, 180_000, 7_200_000, 86_400_000].map((ms) => formatElapsed("ko", ms))).toEqual(["42초", "3분", "2시간", "1일"]);
    expect([42_000, 180_000, 7_200_000, 86_400_000].map((ms) => formatElapsed("zh-CN", ms))).toEqual(["42秒", "3分钟", "2小时", "1天"]);
  });
});

describe("formatSpan", () => {
  it("writes a node's time split as `hide mission show` does, the same in every language", () => {
    const cases: [number, string][] = [
      [-5_000, "0m"],
      [40 * 60_000, "40m"],
      [3_599_999, "59m"],
      [3_600_000, "1h"],
      [90 * 60_000, "1.5h"],
      [(9 * 60 - 2) * 60_000, "9h"],
      [(9 * 60 + 3) * 60_000, "9.1h"],
    ];
    for (const [ms, text] of cases) expect(formatSpan(ms)).toBe(text);
  });
});
