// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { conversationSleep } from "../sleep";
import type { AgentSleep, SleepingSession } from "../snapshot";
import { SleepCard } from "./sleep-card";

// The card a sleeping session draws wherever it is shown, in place of its pane's terminal and over its conversation
// (PRD agent-sleep B11-B14): what a caller sees in each state, and which press dispatches what.

let mounted: ReturnType<typeof createRoot>[] = [];

beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));

afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

async function card(from: Parameters<typeof conversationSleep>[1]) {
  const actions = { wakeAgent: vi.fn(), wakeSleepingSession: vi.fn() };
  const woke = vi.fn();
  const view = conversationSleep("w1:p2", from, actions, woke);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  mounted.push(root);
  await act(async () => root.render(view ? <SleepCard paneId="w1:p2" view={view} /> : null));
  const button = (selector: string) => container.querySelector<HTMLButtonElement>(selector);
  return { container, actions, woke, button };
}

const inPane = (sleep: AgentSleep) => ({ row: { sleep } });

it("says Sleeping with the last progress line and wakes the agent in its pane", async () => {
  const { container, actions, button } = await card(inPane({ state: "sleeping", since_unix_ms: 0, progress: "e2e 기다리는 방식 바꾸는 중" }));
  expect(container.querySelector("[data-pane-sleep]")?.getAttribute("data-pane-sleep")).toBe("sleeping");
  expect(container.textContent).toContain("Sleeping");
  expect(container.querySelector("[data-pane-sleep-progress]")?.textContent).toBe("e2e 기다리는 방식 바꾸는 중");
  expect(button("[data-agent-wake-fresh]")).toBeNull();
  await act(async () => button('[data-agent-wake="w1:p2"]')!.click());
  expect(actions.wakeAgent).toHaveBeenCalledWith("w1:p2");
});

it("says a wake is under way, how old the conversation is, and offers nothing to press", async () => {
  const { container } = await card(inPane({ state: "waking", since_unix_ms: Date.now() - 3 * 24 * 60 * 60 * 1000 }));
  expect(container.querySelector('[role="status"]')?.textContent).toBe("Waking…");
  expect(container.textContent).toContain("Resuming the conversation from 3d ago");
  expect(container.querySelector("button")).toBeNull();
});

it("says a failed wake and why, and offers Retry and a new session", async () => {
  const { container, actions, button } = await card(inPane({ state: "failed", since_unix_ms: 0, reason: "The conversation file is gone" }));
  expect(container.textContent).toContain("Couldn’t resume this conversation");
  expect(container.querySelector('[role="alert"]')?.textContent).toBe("The conversation file is gone");
  await act(async () => button("[data-agent-wake-retry]")!.click());
  await act(async () => button("[data-agent-wake-fresh]")!.click());
  expect(actions.wakeAgent.mock.calls).toEqual([["w1:p2"], ["w1:p2", true]]);
});

const session = (extra: Partial<SleepingSession>): SleepingSession => ({
  sleep_id: "sleep-1", node_id: "mac", checkout_path: "/repo", kind: "pi", identity_label: "Agent two", group: "seen",
  phase: "sleeping", since_unix_ms: 0, reason: null, wake_available: true, checking: false, ...extra,
});

it("wakes a session whose sleep closed its pane in a new pane, and tells the caller to follow it", async () => {
  const { actions, woke, button } = await card({ row: null, session: session({}) });
  await act(async () => button("[data-agent-wake]")!.click());
  expect(actions.wakeSleepingSession).toHaveBeenCalledWith("sleep-1");
  expect(actions.wakeAgent).not.toHaveBeenCalled();
  expect(woke).toHaveBeenCalledTimes(1);
});

it("draws a closed-pane session's wake under way and its failure by its phase, Retry waking it again", async () => {
  const waking = await card({ row: null, session: session({ phase: "starting" }) });
  expect(waking.container.querySelector("[data-pane-sleep]")?.getAttribute("data-pane-sleep")).toBe("waking");
  expect(waking.container.querySelector("button")).toBeNull();
  const failed = await card({ row: null, session: session({ phase: "failed", reason: "Pi could not start" }) });
  expect(failed.container.querySelector('[role="alert"]')?.textContent).toBe("Pi could not start");
  // A sleeping session wakes only as it was; a new session is its pane's choice, and it has no pane.
  expect(failed.button("[data-agent-wake-fresh]")).toBeNull();
  await act(async () => failed.button("[data-agent-wake-retry]")!.click());
  expect(failed.actions.wakeSleepingSession).toHaveBeenCalledWith("sleep-1");
});

it("offers no wake while the core does not, and draws a Desk card that reads asleep before anything says more", async () => {
  const unoffered = await card({ row: null, session: session({ wake_available: false }) });
  expect(unoffered.container.querySelector("[data-pane-sleep]")?.getAttribute("data-pane-sleep")).toBe("sleeping");
  expect(unoffered.container.querySelector("button")).toBeNull();
  const job = await card({ row: null, job: { activity: "sleeping" } });
  expect(job.container.querySelector("[data-pane-sleep]")?.getAttribute("data-pane-sleep")).toBe("sleeping");
  expect(job.container.querySelector("button")).toBeNull();
});

it("is nothing for an awake session, and a live row outranks a sleeping session of the same name", async () => {
  expect(conversationSleep("w1:p2", { row: { sleep: null }, job: { activity: "working" }, session: session({}) }, { wakeAgent: vi.fn(), wakeSleepingSession: vi.fn() })).toBeNull();
});
