// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { threadLines, type AcpSessionUpdate } from "../../desk";
import { LetterBand } from "./letter-band";
import { QuestionLineView } from "./question-line";
import { conversationTurns, type Letter, type QuestionLine, type ToolRun } from "./rows";
import { ToolRunLine } from "./tool-lines";

let mounted: ReturnType<typeof createRoot>[] = [];

beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

async function render(node: React.ReactNode): Promise<HTMLElement> {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  await act(async () => root.render(node));
  return host;
}

/** The one run a turn of these calls folds into. */
function runOf(updates: AcpSessionUpdate[]): ToolRun {
  const item = conversationTurns(threadLines(updates))[0]!.items.find((one) => one.kind === "run");
  if (item?.kind !== "run") throw new Error("no run");
  return item.run;
}

const command = (id: string, status: "completed" | "failed"): AcpSessionUpdate => ({ sessionUpdate: "tool_call", toolCallId: id, title: "ls", kind: "execute", status, rawInput: { command: "ls" } });

it("says a run's counts in one gray line, a failure alone in red, and opens or folds it when pressed", async () => {
  const run = runOf([command("c1", "completed"), command("c2", "failed"), { sessionUpdate: "tool_call", toolCallId: "e", title: "Write a.md", kind: "edit", status: "completed", content: [{ type: "diff", path: "a.md", oldText: "a", newText: "a\nb" }] }]);
  const toggle = vi.fn();
  const host = await render(<ToolRunLine run={run} open={false} onToggle={toggle} />);
  const line = host.querySelector<HTMLElement>("[data-conversation-run]")!;
  expect(line.textContent).toBe("Ran 2 commands · 1 failed · Changed 1 file+1");
  expect(line.getAttribute("aria-expanded")).toBe("false");
  expect(host.querySelector("[data-tool-failed]")?.className).toContain("text-destructive");
  expect(host.querySelector("[data-tool-mark]")?.getAttribute("data-tool-mark")).toBe("edit");
  await act(async () => line.click());
  expect(toggle).toHaveBeenCalledTimes(1);
});

const letter: Letter = { key: "l1", direction: "received", peer: { name: "m110-mini-journey", mark: "going", agent_kind: "claude", pane_id: "w1:p2" }, kind: "reply", first: "1) 지웠어요", text: "1) 지웠어요\n2) 다시 돌렸어요", command: null };

it("draws a letter's band by its arrow, name, kind and first line, opens the whole letter on a press, and leaves the name to open its session", async () => {
  const toggle = vi.fn();
  const visit = vi.fn();
  const name = <button type="button" data-name onClick={visit}>m110-mini-journey</button>;
  const folded = await render(<LetterBand letter={letter} open={false} onToggle={toggle} name={name} />);
  const band = folded.querySelector<HTMLElement>("[data-conversation-letter]")!;
  expect(band.getAttribute("data-conversation-letter")).toBe("received");
  expect(band.textContent).toBe("←m110-mini-journeyReply1) 지웠어요");
  expect(folded.querySelector("[data-letter-text]")).toBeNull();
  await act(async () => folded.querySelector<HTMLElement>("[data-name]")!.click());
  expect(visit).toHaveBeenCalledTimes(1);
  expect(toggle).not.toHaveBeenCalled();
  await act(async () => band.querySelector<HTMLElement>("button[aria-expanded]")!.click());
  expect(toggle).toHaveBeenCalledTimes(1);
  // Open, the whole letter stands in the band and the first line leaves the head.
  const open = await render(<LetterBand letter={{ ...letter, direction: "sent", kind: null }} open onToggle={toggle} />);
  expect(open.querySelector("[data-letter-text]")?.textContent).toBe("1) 지웠어요\n2) 다시 돌렸어요");
  expect(open.querySelector("[data-conversation-letter] > div")?.textContent).toBe("→m110-mini-journeyLetter");
  // A reply whose letter the record does not hold names no one.
  const nameless = await render(<LetterBand letter={{ ...letter, direction: "sent", kind: "reply", peer: null }} open={false} onToggle={toggle} />);
  expect(nameless.querySelector("[data-letter-peer]")).toBeNull();
  expect(nameless.querySelector("[data-conversation-letter] > div")?.textContent).toBe("→Reply1) 지웠어요");
});

it("draws each question with its answer under it, or that it waits while asked and was not answered once it ended", async () => {
  const line = (status: QuestionLine["status"], answer: string | null): QuestionLine => ({ kind: "question", id: "q", status, questions: [{ text: "어느 폴더에 둘까요?", choices: ["새 폴더", "기존 폴더"], answer }] });
  const answer = async (status: QuestionLine["status"], given: string | null) => (await render(<QuestionLineView line={line(status, given)} />)).querySelector("[data-question-answer]")?.textContent;
  expect(await answer("completed", "새 폴더")).toBe("새 폴더");
  expect(await answer("pending", null)).toBe("Waiting for an answer");
  expect(await answer("completed", null)).toBe("Not answered");
});
