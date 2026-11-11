import { describe, expect, it } from "vitest";
import { bellRows, type ScreenRow } from "./terminalBellLines";

const row = (text: string): ScreenRow => ({ text, used: Array.from(text).reduce((n, c) => n + (c.charCodeAt(0) > 0x2e80 || c === "🦉" ? 2 : 1), 0) });

describe("bellRows", () => {
  it("finds the echo Claude Code and Codex draw for a bell line", () => {
    const rows = [
      "❯ 🦉 push-quiet-2 진행: 재실행에서 Linux 빌드가 복구됐습니다.",
      "",
      "⏺ 외부 사이트가 돌아왔어요.",
      "› 🦉 review-rest-switch 보고: 방향에 동의합니다. 아래 기존 복구 경로와 동시 실…",
      "  └ {\"type\":\"workspace_result\"}",
    ].map(row);
    expect(bellRows(rows, 100)).toEqual([0, 3]);
  });

  it("leaves a person's own words alone", () => {
    const rows = ["❯ 🦉가 뭐야?", "❯ fix the bell", "🦉 no prompt glyph in front", "● 🦉 reply"].map(row);
    expect(bellRows(rows, 100)).toEqual([]);
  });

  it("follows a bell line a narrow pane broke across rows", () => {
    const rows = [
      "❯ 🦉 push-quiet-2 진행: 재실행에서 Linux 빌드가",
      "  복구됐습니다.",
      "",
      "⏺ done",
    ].map(row);
    expect(bellRows(rows, 40)).toEqual([0, 1]);
  });

  it("stops at a tool result drawn under a short bell line", () => {
    const rows = ["❯ 🦉 a 진행: 끝", "  ⎿ Running hook", "  plain text"].map(row);
    expect(bellRows(rows, 40)).toEqual([0]);
  });
});
