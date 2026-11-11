import { describe, expect, it } from "vitest";
import { bellRows, type ScreenRow } from "./terminalBellLines";

/** A row as the scan reads it: wide glyphs and the owl take two cells. */
function row(text: string): ScreenRow {
  let used = 0;
  for (const char of Array.from(text)) used += (char.codePointAt(0) ?? 0) > 0x2e80 || char === "🦉" ? 2 : 1;
  return { text, used };
}

const screen = (lines: string[]) => lines.map(row);

describe("bellRows", () => {
  it("finds the echo Claude Code and Codex draw for a bell line", () => {
    const rows = screen([
      "❯ 🦉 push-quiet-2 진행: 재실행에서 Linux 빌드가 복구됐습니다.",
      "",
      "⏺ 외부 사이트가 돌아왔어요.",
      "› 🦉 review-rest-switch 보고: 방향에 동의합니다. 아래 기존 복구 경로와 동시 실…",
      "  └ {\"type\":\"workspace_result\"}",
    ]);
    expect(bellRows(rows, 100)).toEqual([0, 3]);
  });

  it("finds Grok's echo, which stands in a margin and ends in the time", () => {
    const rows = screen(["     ❯ 🦉 ci-lead 요청: CI 다시 봐줘 · hide inbox                 8:55 PM", "", "     ◆ Thought for 0.3s"]);
    expect(bellRows(rows, 100)).toEqual([0]);
  });

  it("leaves a person's own words alone", () => {
    const rows = screen(["❯ 🦉가 뭐야?", "❯ fix the bell", "🦉 no prompt glyph in front", "● 🦉 reply"]);
    expect(bellRows(rows, 100)).toEqual([]);
  });

  // The three below are the screens a 52-column pane showed when the line
  // `🦉 sender-agent-long-name-xyz 보고: … · hide inbox` was submitted.
  it("follows Claude Code's line across the rows it indents", () => {
    const rows = screen([
      "❯ 🦉 sender-agent-long-name-xyz 보고: 시험 줄입니다",
      "  아무것도 하지 말고 한 줄로만 답하세요 감사합니다",
      "  · hide inbox",
      "",
      "⏺ Login expired · Please run /login",
    ]);
    expect(bellRows(rows, 52)).toEqual([0, 1, 2]);
  });

  it("follows Codex's line across the rows it indents", () => {
    const rows = screen([
      "› 🦉 sender-agent-long-name-xyz 보고: 시험 줄입니다",
      "  아무것도 하지 말고 한 줄로만 답하세요 감사합니다",
      "  · hide inbox",
      "",
      "• Working",
    ]);
    expect(bellRows(rows, 52)).toEqual([0, 1, 2]);
  });

  it("follows Grok's line across the rows it indents past its margin", () => {
    const rows = screen([
      "     ❯ 🦉 sender-agent-long-name-xyz     8:56 PM",
      "       보고: 시험 줄입니다 아무것도 하",
      "       지 말고 한 줄로만 답하세요 감사",
      "       합니다 · hide inbox",
      "",
      "     ◆ Thought for 1.8s",
    ]);
    expect(bellRows(rows, 52)).toEqual([0, 1, 2, 3]);
  });

  it("stops at chrome drawn under a short bell line", () => {
    const rows = screen(["❯ 🦉 a 진행: 끝", "  ⎿ Running hook", "  plain text"]);
    expect(bellRows(rows, 40)).toEqual([0]);
  });

  it("does not take the answer under the line for part of it", () => {
    const rows = screen(["❯ 🦉 sender-agent-long-name-xyz 보고: 시험 줄입니다 ok", "", "  아무것도 하지 말고 한 줄로만 답하세요 감사합니다"]);
    expect(bellRows(rows, 52)).toEqual([0]);
  });
});
