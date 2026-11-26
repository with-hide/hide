// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { TooltipProvider } from "./components/ui/tooltip";
import { MarkdownText, type MarkdownChip } from "./MarkdownText";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

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
  const stream = async (step: string) => {
    text += step;
    await act(async () => root.render(<TooltipProvider><MarkdownText text={text} size="chat" chips={chips} copyCode={copy} /></TooltipProvider>));
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
});
