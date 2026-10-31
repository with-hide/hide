// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DeskJob, DeskMention, DeskSessionRef, SessionFacts } from "../../desk";
import { TooltipProvider } from "../ui/tooltip";
import { DeskReply } from "./desk-chat";
import { SessionCard, SessionChipProvider } from "./session-chip";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

const card: DeskJob = {
  id: "pane:w1:p7", agent_id: null, pane_id: "w1:p7", title: "agent-journey", agent_kind: "codex", project: { workspace_id: "w", label: "herdr-ide" }, location: { kind: "worktree", label: "journey-agy" },
  state: "working", queue_position: null, now: "Linux e2e 결과 기다리는 중", plan: { done: 3, total: 5 }, question: null, summary: null, pull_requests: [{ number: 1921, state: "pending" }],
  outputs: [{ id: "o1", kind: "image", label: "a.png", thumb: null, seen: true, pr_state: null, device_id: null, checkout: null, path: null, url: null }, { id: "o2", kind: "doc", label: "b.md", thumb: null, seen: true, pr_state: null, device_id: null, checkout: null, path: null, url: null }],
  joined: false, reason: null, failure: null, ask: null, mark: "going", mark_read: false, machine: null, disconnected: false, changed_at_unix_ms: 0, read: false, mission: null, workers: [],
};

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
  await act(async () => root.render(<TooltipProvider>{node}</TooltipProvider>));
  // The reply's chips are drawn in roots of their own, mounted as its Markdown is laid out.
  await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
  return host;
}

it("draws every session a reply names as a chip, the seventeenth on too, and dims one no longer listed (B50)", async () => {
  const mentions: DeskMention[] = Array.from({ length: 20 }, (_, index) => ({ kind: "session", id: `w1:p${index + 1}`, name: `job-${index + 1}`, mark: "going", agent_kind: "claude" }));
  const text = mentions.map((mention) => `@session(${mention.id} ${mention.name})`).join(" ");
  const facts = (session: DeskSessionRef): SessionFacts | null =>
    session.pane_id === "w1:p20" ? { listed: false, mark: null, agent_kind: null, card: null, since: null, parent: null } : { listed: true, mark: "going", agent_kind: "claude", card: null, since: null, parent: null };
  const host = await render(<SessionChipProvider source={{ facts, onOpen: () => undefined }}><DeskReply text={text} mentions={mentions} /></SessionChipProvider>);
  const chips = [...host.querySelectorAll("[data-desk-session-chip]")].map((chip) => chip.getAttribute("data-desk-session-chip"));
  expect(chips).toEqual(mentions.map((mention) => mention.name));
  expect(host.textContent).not.toContain("@session(");
  expect([...host.querySelectorAll('[data-desk-session-gone="true"]')].map((chip) => chip.textContent)).toEqual(["job-20"]);
});

it("shows a session's card with its place, line, plan, pull request, outputs and parent, and opens its thread (D-86)", async () => {
  const opened: string[] = [];
  const parent: DeskSessionRef = { name: "agent-14586", mark: "turn", agent_kind: "claude", pane_id: "w1:p0" };
  const host = await render(<SessionCard job={card} since={null} parent={parent} onOpen={() => opened.push("thread")} onParent={(session) => opened.push(session.name)} />);
  const words = host.textContent ?? "";
  for (const fact of ["agent-journey", "herdr-ide · journey-agy", "Linux e2e 결과 기다리는 중", "3/5", "agent-14586"]) expect(words).toContain(fact);
  expect(host.querySelector("[data-desk-session-card-pr]")?.getAttribute("data-desk-session-card-pr")).toBe("1921");
  expect(host.querySelector("[data-desk-session-card-outputs]")?.getAttribute("data-desk-session-card-outputs")).toBe("2");
  await act(async () => host.querySelector<HTMLElement>("[data-desk-session-card-open]")!.click());
  await act(async () => host.querySelector<HTMLElement>('[data-desk-session-card-parent] [data-desk-session-chip]')!.click());
  expect(opened).toEqual(["thread", "agent-14586"]);
});
