import { afterEach, describe, expect, it, vi } from "vitest";
import { Terminal } from "@xterm/xterm";
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

  it("leaves an owl glued to a word, or with no prompt glyph before it, alone", () => {
    const rows = screen(["❯ 🦉가 뭐야?", "❯ fix the bell", "🦉 no prompt glyph in front", "● 🦉 reply"]);
    expect(bellRows(rows, 100)).toEqual([]);
  });

  // Accepted limits (PR review): the scan reads the row's shape, never who typed it.
  it("also dims a line a person typed in the same shape, such as a composer draft", () => {
    expect(bellRows(screen(["❯ 🦉 hello", "› 🦉 hello"]), 100)).toEqual([0, 1]);
  });

  it("does not match a Grok row its keyboard focus frames", () => {
    expect(bellRows(screen(["│   ❯ 🦉 a 보고: 끝       │"]), 100)).toEqual([]);
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
        it(`${echo.id}: a bell line led by ${JSON.stringify(margin + glyph)} wraps onto indented rows`, () => {
          const indent = " ".repeat((margin + glyph + " ").length);
          const rows = screen([
            `${margin}${glyph} 🦉 sender-agent-long-name-xyz 보고: 시험 줄입니다`,
            `${indent}아무것도 하지 말고 한 줄로만 답하세요 감사합니다`,
            `${indent}· hide inbox`,
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
    registerMarker: (offset: number) => {
      counts.markers += 1;
      return { line: offset, isDisposed: false, dispose: () => {} };
    },
    registerDecoration: () => {
      counts.decorations += 1;
      return { isDisposed: false, dispose: () => (counts.disposed += 1) };
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

// Rows as the agents drew them, not built from a declaration, so a declaration
// that drifts from the screen fails here. Claude Code's are from its captured
// screen `hide-agent-adapter/tests/fixtures/composer/claude-2.1.314-coloured-0-answered.ansi`
// (the `⎿` row the agent draws under a prompt row), Codex's from a Codex pane
// (`└` under a prompt row), Grok's from a Grok pane's unframed letter rows.
describe("rows as the agents drew them", () => {
  it("Claude Code: the `⎿` row under a full-width letter row is its own output, not the letter's", () => {
    const rows = screen(["❯ 🦉 a 진행: 끝", "  ⎿  Session color set to: green", "", "❯ Reply with only the word ok", "⏺ ok"]);
    expect(bellRows(rows, 24)).toEqual([0]);
  });

  it("Codex: the `└` row under a full-width letter row is its own output, not the letter's", () => {
    const rows = screen(["› 🦉 a 보고: 끝", '  └ {"type":"workspace_result"}', "", "• ok"]);
    expect(bellRows(rows, 24)).toEqual([0]);
  });

  it("Grok: the wrapped rows of an unframed letter row, and a `◆` row at their indent under the last one", () => {
    // The letter's rows fill the width, as a Grok pane's did; its own `◆` rows sit
    // at a smaller indent in a capture, so this one is placed at the wrap indent
    // with no blank row between, which only the declared chrome ends the line at.
    const rows = screen([
      "     ❯ 🦉 m58-conductor 보고: 시험 줄입니다. 아무것도 하지 말고 ok 한 단어로만 답하세요. 이 줄은 pane 폭을 넘겨 여",
      "       러 줄로 꺾이도록 길게 씁니다 길게 길게 길게 길게 길게 길게 길게 길게 길게 길게 길게 길게 길게 길게 길게 길게",
      `       ${"길게 ".repeat(17)}· hide inbox`,
      "       ◆ Thought for 0.6s",
      "",
      "     ok",
    ]);
    expect(bellRows(rows, rows[1]!.used)).toEqual([0, 1, 2]);
  });
});

/** Each marker xterm holds for the terminal, which a decoration's disposal does not release. */
function markersHeld(term: Terminal): number {
  return (term as unknown as { _core: { buffer: { markers: unknown[] } } })._core.buffer.markers.length;
}

const written = (term: Terminal, data: string) => new Promise<void>((done) => term.write(data, done));

describe("what is drawn on a real terminal", () => {
  it("holds one marker for the row however often its color changes, and none once forgotten", async () => {
    const term = new Terminal({ rows: 10, cols: 80, scrollback: 0, allowProposedApi: true });
    await written(term, "\x1b[3;1H❯ 🦉 a 진행: 끝\x1b[10;1H");
    for (let i = 0; i < 100; i += 1) dimBellLines(term, i % 2 === 0 ? "#666666" : "#777777");
    expect(markersHeld(term)).toBe(1);
    forgetBellLines(term);
    expect(markersHeld(term)).toBe(0);
    term.dispose();
  });

  it("draws again after a resize and the holder's redraw leave the same row", async () => {
    const term = new Terminal({ rows: 10, cols: 80, scrollback: 0, allowProposedApi: true });
    await written(term, "\x1b[3;1H❯ 🦉 a 진행: 끝\x1b[10;1H");
    dimBellLines(term, "#666666");
    expect(markersHeld(term)).toBe(1);
    term.resize(80, 5);
    expect(markersHeld(term)).toBe(0);
    await written(term, "\x1bc\x1b[3;1H❯ 🦉 a 진행: 끝\x1b[5;1H");
    dimBellLines(term, "#666666");
    expect(markersHeld(term)).toBe(1);
    forgetBellLines(term);
    term.dispose();
  });

  it("draws again when its marker was disposed under it and the row is the same", async () => {
    const term = new Terminal({ rows: 10, cols: 80, scrollback: 0, allowProposedApi: true });
    await written(term, "\x1b[3;1H❯ 🦉 a 진행: 끝\x1b[10;1H");
    dimBellLines(term, "#666666");
    const [marker] = (term as unknown as { _core: { buffer: { markers: Array<{ dispose(): void }> } } })._core.buffer.markers;
    marker!.dispose();
    expect(markersHeld(term)).toBe(0);
    dimBellLines(term, "#666666");
    expect(markersHeld(term)).toBe(1);
    forgetBellLines(term);
    term.dispose();
  });
});
