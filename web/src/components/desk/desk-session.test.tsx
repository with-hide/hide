// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "../../actions";
import type { DeskJob } from "../../desk";
import type { AgentRow, AgentSleep, MissionNode, SnapshotRest } from "../../snapshot";
import { useShellStore } from "../../store";
import { TooltipProvider } from "../ui/tooltip";
import { SessionHead, SessionSleepingFoot } from "./desk-session";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

// The foot that stands in a sleeping session's input box beside Desk (PRD desk-2-ui B43) says what the sleeping pane
// says (`SleepBody`): asleep, waking, or a wake that failed with its reason and both ways on.

let mounted: ReturnType<typeof createRoot>[] = [];

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
});

afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

async function foot(sleep: AgentSleep | null, state = "unknown") {
  const wake = vi.fn();
  const fresh = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  mounted.push(root);
  await act(async () => root.render(<SessionSleepingFoot title="Agent two" sleep={sleep} state={state} onWake={sleep?.state === "waking" ? null : wake} onFresh={fresh} />));
  const button = (selector: string) => container.querySelector<HTMLButtonElement>(selector);
  return { container, wake, fresh, button };
}

it("offers Wake agent for an agent asleep in its pane", async () => {
  const { container, wake, button } = await foot({ state: "sleeping", since_unix_ms: 0 });
  expect(container.textContent).toContain("Agent two");
  expect(button("[data-desk-session-wake]")?.textContent).toBe("Wake agent");
  expect(button("[data-desk-session-wake-fresh]")).toBeNull();
  await act(async () => button("[data-desk-session-wake]")!.click());
  expect(wake).toHaveBeenCalledTimes(1);
});

it("says a wake is under way and offers nothing to press while it runs", async () => {
  const { container, button } = await foot({ state: "waking", since_unix_ms: Date.now() });
  expect(container.querySelector('[role="status"]')?.textContent).toBe("Resuming the conversation");
  expect(button("[data-desk-session-wake]")).toBeNull();
});

it("says a failed wake, why, and offers Retry and a new session as the pane does", async () => {
  const { container, wake, fresh, button } = await foot({ state: "failed", since_unix_ms: 0, reason: "The conversation file is gone" });
  expect(container.querySelector("[data-desk-session-sleeping]")?.getAttribute("data-desk-session-sleeping")).toBe("failed");
  expect(container.textContent).toContain("Couldn’t resume this conversation");
  expect(container.querySelector('[data-desk-session-wake-reason]')?.textContent).toBe("The conversation file is gone");
  expect(button("[data-desk-session-wake]")?.textContent).toBe("Retry");
  await act(async () => button("[data-desk-session-wake]")!.click());
  await act(async () => button("[data-desk-session-wake-fresh]")!.click());
  expect(wake).toHaveBeenCalledTimes(1);
  expect(fresh).toHaveBeenCalledTimes(1);
});

it("draws a session whose sleep closed its pane by its phase, with Wake agent when the core offers it", async () => {
  const { container, button } = await foot(null, "sleeping");
  expect(container.querySelector("[data-desk-session-sleeping]")?.getAttribute("data-desk-session-sleeping")).toBe("sleeping");
  expect(button("[data-desk-session-wake]")?.textContent).toBe("Wake agent");
});

describe("the session panel's head (Doop H 다, operator decision 2026-11-25)", () => {
  const job = (state: DeskJob["state"]): DeskJob => ({
    id: "pane:w1:p2", agent_id: null, pane_id: "w1:p2", title: "끊긴 뒤 로그인 고치기", agent_kind: "claude", project: { workspace_id: "w", label: "repo" }, location: { kind: "checkout", label: "main" },
    state, queue_position: null, now: null, plan: null, question: null, summary: null, pull_requests: [], outputs: [], joined: false, reason: null, failure: null, ask: null,
    mark: "going", mark_read: false, machine: null, disconnected: false, changed_at_unix_ms: 0, read: true, mission: null, workers: [],
  });

  async function head(machine: string | null, disconnected = false, row: AgentRow | null = null, agentId: string | null = null) {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    mounted.push(root);
    const noop = () => undefined;
    const actions = createActions(() => true);
    await act(async () => root.render(
      <TooltipProvider>
        <SessionHead paneId="w1:p2" title="끊긴 뒤 로그인 고치기" job={{ ...job("working"), agent_id: agentId }} row={row} pane={null} machine={machine} disconnected={disconnected} sleeping={false} explorerOpen={false} filesChord="⇧⌘E" terminalChord="⌘J" deskActions={actions} onFiles={noop} onTerminal={noop} onOwnTab={noop} onClose={noop} />
      </TooltipProvider>,
    ));
    return container;
  }

  it("is one title line with files and terminal as icons when there is nothing more to know", async () => {
    const container = await head(null);
    expect(container.querySelector("[data-desk-session-meta]")).toBeNull();
    expect(container.querySelector("[data-desk-session-files]")?.textContent).toBe("");
    expect(container.querySelector("[data-desk-session-terminal]")?.textContent).toBe("");
  });

  it("adds a quiet second line naming another device, and that it is not connected", async () => {
    const container = await head("Windows 노트북", true);
    const meta = container.querySelector("[data-desk-session-meta]");
    expect(meta?.querySelector("[data-device-chip]")?.textContent).toBe("Windows 노트북");
    expect(meta?.querySelector("[data-desk-session-disconnected]")).not.toBeNull();
  });

  it("names the mission a worker's session works, by its number and name, and settles", async () => {
    const saved = useShellStore.getState();
    const worker = { status: "working", agent_id: "agent-7", pane_id: "w1:p2", machine: null } as Partial<MissionNode>;
    const missions = { items: [{ id: "mission-101", title: "Desk 2차", device_id: "mac", status: "running", nodes: [worker] }] };
    useShellStore.setState({ rest: { missions } as unknown as SnapshotRest });
    try {
      const container = await head(null, false, { id: "row-1", pane_id: "w1:p2", device_id: "mac" } as AgentRow, "agent-7");
      const named = container.querySelector('[data-desk-session-meta] [data-desk-session-mission="mission-101"]');
      expect(named?.textContent).toContain("101");
      expect(named?.textContent).toContain("Desk 2차");
    } finally {
      useShellStore.setState(saved, true);
    }
  });
});
