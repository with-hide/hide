// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { DeskReply } from "./components/desk/desk-chat";
import { TooltipProvider } from "./components/ui/tooltip";
import { markdownLive } from "./editor/markdownLivePlugin";
import { MarkdownText, type MarkdownChip } from "./MarkdownText";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
// The Live plan is built where the text's parts are swapped in; counting its builds counts the swaps.
vi.mock("./editor/markdownLivePlugin", async (real) => {
  const plugin = await real<typeof import("./editor/markdownLivePlugin")>();
  return { ...plugin, markdownLive: vi.fn(plugin.markdownLive) };
});

let mounted: ReturnType<typeof createRoot>[] = [];

beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

const chips: MarkdownChip[] = [{ token: "@pane(w1:p7 journey)", node: <b data-test-chip="journey">journey</b> }];
const copies: string[] = [];
const copy = async (text: string) => {
  copies.push(text);
  return true;
};

// An answer as an agent streams it, each step adding to the end: a heading and a table, its next row,
// a paragraph naming a session and an open code block, the code's next line.
const steps = [
  "## 미션\n\n| 미션 | 상태 |\n| --- | --- |\n| Desk 2차 | running |\n",
  "| 안드로이드 앱 | planning |\n",
  "\n@pane(w1:p7 journey) 가 맡고 있습니다.\n\n```ts\nconst a = 1;\n",
  "const b = 2;\n",
];

it("keeps every part drawn before a streamed update as the same node, and draws what the update added", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  let text = "";
  let parts = chips;
  const draw = async () => {
    await act(async () => root.render(<TooltipProvider><MarkdownText text={text} size="chat" chips={parts} copyCode={copy} /></TooltipProvider>));
    await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
  };
  const stream = async (step: string) => {
    text += step;
    await act(async () => root.render(<TooltipProvider><MarkdownText text={text} size="chat" chips={parts} copyCode={copy} /></TooltipProvider>));
    // A part the editor let go of is unmounted a microtask later; give that time to show.
    await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
  };
  const table = () => host.querySelector(".cm-md-table");

  await stream(steps[0]!);
  await vi.waitFor(() => expect(table()?.textContent).toContain("Desk 2차"));
  const editor = host.querySelector(".cm-editor");
  const drawnTable = table();

  await stream(steps[1]!);
  expect(table()).toBe(drawnTable);
  await vi.waitFor(() => expect(drawnTable?.textContent).toContain("안드로이드 앱"));

  await stream(steps[2]!);
  await vi.waitFor(() => expect(host.querySelector("[data-copy-words='code block']")).not.toBeNull());
  const chip = host.querySelector("[data-markdown-chip]");
  const copyButton = host.querySelector<HTMLElement>("[data-copy-words='code block']");
  expect(chip?.textContent).toBe("journey");

  await stream(steps[3]!);
  expect(host.querySelector(".cm-editor")).toBe(editor);
  expect(table()).toBe(drawnTable);
  expect(host.querySelector("[data-markdown-chip]")).toBe(chip);
  expect(chip?.querySelector("[data-test-chip]")?.textContent).toBe("journey");
  expect(host.querySelector("[data-copy-words='code block']")).toBe(copyButton);
  await act(async () => copyButton!.click());
  // A fence not closed yet runs to the end of the text, its last line break included.
  expect(copies.at(-1)?.trimEnd()).toBe("const a = 1;\nconst b = 2;");

  // A word changed in the middle, then a line taken off the end, are edits of the same drawing too.
  text = text.replace("| Desk 2차 | running |", "| Desk 2차 | done |");
  await draw();
  expect(table()).toBe(drawnTable);
  await vi.waitFor(() => expect(drawnTable?.textContent).toContain("done"));
  text = text.replace("const b = 2;\n", "");
  await draw();
  expect(host.querySelector("[data-markdown-chip]")).toBe(chip);
  expect(host.querySelector("[data-copy-words='code block']")).toBe(copyButton);
  await act(async () => copyButton!.click());
  expect(copies.at(-1)?.trimEnd()).toBe("const a = 1;");

  // The same session drawn anew (its chip's part changed) redraws inside the chip already there.
  parts = [{ token: chips[0]!.token, node: <b data-test-chip="journey">journey · 작업 중</b> }];
  await draw();
  expect(host.querySelector("[data-markdown-chip]")).toBe(chip);
  expect(chip?.textContent).toBe("journey · 작업 중");
});

it("swaps no part of Desk's answer while it streams, though each word draws the reply again", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  const built = vi.mocked(markdownLive).mock.calls.length;
  let text = "";
  for (const step of ["## 미션\n\n| 미션 | 상태 |\n| --- | --- |\n", "| Desk 2차 | running |\n", "\n진행 중입니다.", " 곧 끝납니다."]) {
    text += step;
    await act(async () => root.render(<TooltipProvider><DeskReply text={text} live /></TooltipProvider>));
  }
  await vi.waitFor(() => expect(host.querySelector(".cm-md-table")?.textContent).toContain("Desk 2차"));
  expect(host.textContent).toContain("곧 끝납니다.");
  expect(vi.mocked(markdownLive).mock.calls.length - built).toBe(1);
});

it("draws in a drawn table's cells the chips that arrive while the answer streams, and draws a chip again when its part changes", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  const text = "| 미션 | 상태 |\n| --- | --- |\n| @mission(mission-63 결제) | running |\n";
  const draw = (parts: readonly MarkdownChip[]) => act(async () => root.render(<TooltipProvider><MarkdownText text={text} size="chat" chips={parts} /></TooltipProvider>));
  await draw([]);
  await vi.waitFor(() => expect(host.querySelector(".cm-md-table")?.textContent).toContain("@mission(mission-63 결제)"));
  const table = host.querySelector(".cm-md-table");
  await draw([{ token: "@mission(mission-63 결제)", node: <i data-test-chip="mission-63">M63 결제</i> }]);
  expect(host.querySelector(".cm-md-table")).toBe(table);
  await vi.waitFor(() => expect(table?.querySelector("[data-test-chip='mission-63']")?.textContent).toBe("M63 결제"));
  // A chip whose part changed (its mission moved on) is a changed chip too.
  await draw([{ token: "@mission(mission-63 결제)", node: <i data-test-chip="mission-63">M63 결제 · 3/4</i> }]);
  await vi.waitFor(() => expect(table?.querySelector("[data-test-chip='mission-63']")?.textContent).toBe("M63 결제 · 3/4"));
});

it("keeps room around every chip from the words beside it, and gives Desk's lines the height of a chip and room above and below it", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  const text = "@session(w1:p2 fix) 가 `/a/b.md` 를 고쳤고 @mission(mission-63 결제) 도 돌아요.";
  const chips: MarkdownChip[] = [
    { token: "@session(w1:p2 fix)", node: <b data-test-chip="session">fix</b> },
    { token: "`/a/b.md`", at: [text.indexOf("`")], node: <b data-test-chip="file">b.md</b> },
    { token: "@mission(mission-63 결제)", node: <b data-test-chip="mission">M63 결제</b> },
  ];
  await act(async () => root.render(<TooltipProvider><MarkdownText text={text} size="chat" chips={chips} /></TooltipProvider>));
  await vi.waitFor(() => expect(host.querySelectorAll("[data-markdown-chip]")).toHaveLength(3));
  expect([...host.querySelectorAll("[data-markdown-chip]")].map((chip) => getComputedStyle(chip).margin)).toEqual(Array(3).fill("0 var(--spacing-xxs)"));
  expect(getComputedStyle(host.querySelector(".cm-content")!).lineHeight).toBe("calc(var(--text-subhead) + var(--spacing-md))");
});

it("draws a chip that arrives after the text was parsed over the Live plan's marks it covers", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  const text = "메모는 `note-pi.txt`에 남겼어요.";
  const draw = (parts: readonly MarkdownChip[]) => act(async () => root.render(<TooltipProvider><MarkdownText text={text} size="chat" chips={parts} /></TooltipProvider>));
  await draw([]);
  // The language pack has parsed the text: the inline code is drawn as code.
  await vi.waitFor(() => expect(host.querySelector(".cm-md-code-inline")).not.toBeNull());
  await draw([{ token: "`note-pi.txt`", at: [4], node: <b data-test-file="note-pi.txt">note-pi.txt</b> }]);
  await vi.waitFor(() => expect(host.querySelector("[data-markdown-chip] [data-test-file]")?.textContent).toBe("note-pi.txt"));
});
