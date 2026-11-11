import { afterEach, describe, expect, it, vi } from "vitest";
import type { Terminal } from "@xterm/xterm";
import { AGENT_ADAPTERS } from "./agentAdapters";
import { bellRows, bellScan, dimBellLines, forgetBellLines, queueBellLines, type ScreenRow } from "./terminalBellLines";

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

// What each agent declares about its echo (`hide-agent-adapter`, `BellEcho`)
// is what the scan reads, so a declaration that does not fit the rows the agent
// draws fails here, and an agent that types the line without declaring one
// fails the Rust contract test (`bell_echo`).
describe("the echo each agent declares", () => {
  const echoes = AGENT_ADAPTERS.flatMap((adapter) => (adapter.bell_echo ? [{ id: adapter.id, ...adapter.bell_echo }] : []));

  it("is declared by the agents that type the line", () => {
    expect(echoes.map((echo) => echo.id).sort()).toEqual(["claude-code", "codex", "grok"]);
  });

  for (const echo of echoes) {
    for (const margin of ["", "     "]) {
      for (const glyph of echo.prompt) {
        it(`${echo.id}: a bell line led by ${JSON.stringify(margin + glyph)} wraps onto indented rows and stops at its chrome`, () => {
          const indent = " ".repeat((margin + glyph + " ").length);
          const rows = screen([
            `${margin}${glyph} 🦉 sender-agent-long-name-xyz 보고: 시험 줄입니다`,
            `${indent}아무것도 하지 말고 한 줄로만 답하세요 감사합니다`,
            `${indent}· hide inbox`,
            ...echo.chrome.map((chrome) => `${indent}${chrome} the agent's own output`),
          ]);
          expect(bellRows(rows, 52)).toEqual([0, 1, 2]);
        });
      }
    }
  }

  it("finds nothing for an echo no agent declared", () => {
    const rows = screen(["◉ 🦉 a 진행: 끝"]);
    expect(bellRows(rows, 40)).toEqual([]);
    expect(bellRows(rows, 40, bellScan([{ prompt: ["◉"], chrome: [] }]))).toEqual([0]);
  });
});

/** A terminal that counts what the shell asks of it: reads of its rows, markers and decorations. */
function fakeTerminal(text: string[]) {
  const counts = { reads: 0, markers: 0, decorations: 0, disposed: 0 };
  const term = {
    rows: text.length,
    cols: 100,
    buffer: {
      active: {
        viewportY: 0,
        baseY: 0,
        cursorY: 0,
        getLine: (y: number) => {
          counts.reads += 1;
          const row = text[y] ?? "";
          return {
            translateToString: () => row,
            getCell: (x: number) => ({ getChars: () => (x < row.length ? "x" : ""), getWidth: () => 1 }),
          };
        },
      },
    },
    registerMarker: () => {
      counts.markers += 1;
      return { dispose: () => {} };
    },
    registerDecoration: () => {
      counts.decorations += 1;
      return { dispose: () => (counts.disposed += 1) };
    },
  };
  return { term: term as unknown as Terminal, counts, text };
}

describe("reading the screen", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("reads the screen once per frame however many writes the frame brought", () => {
    const frames: Array<() => void> = [];
    vi.stubGlobal("requestAnimationFrame", (run: () => void) => frames.push(run));
    vi.stubGlobal("cancelAnimationFrame", () => {});
    const { term, counts } = fakeTerminal(["❯ 🦉 a 진행: 끝", "", "⏺ ok"]);
    for (let write = 0; write < 40; write += 1) queueBellLines(term, () => "#666");
    expect(frames).toHaveLength(1);
    frames[0]!();
    expect(counts.reads).toBe(3);
    queueBellLines(term, () => "#666");
    expect(frames).toHaveLength(2);
    forgetBellLines(term);
  });

  it("makes no marker or decoration for a screen without the owl", () => {
    const { term, counts } = fakeTerminal(["$ ls", "⏺ output", "❯ fix the bell"]);
    dimBellLines(term, "#666");
    expect(counts).toMatchObject({ markers: 0, decorations: 0 });
  });

  it("draws the rows once and leaves them alone while they do not move", () => {
    const { term, counts } = fakeTerminal(["❯ 🦉 a 진행: 끝", "⏺ ok"]);
    dimBellLines(term, "#666");
    dimBellLines(term, "#666");
    expect(counts).toMatchObject({ markers: 1, decorations: 1, disposed: 0 });
    dimBellLines(term, "#777");
    expect(counts).toMatchObject({ decorations: 2, disposed: 1 });
    forgetBellLines(term);
    expect(counts.disposed).toBe(2);
  });
});
