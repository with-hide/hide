// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "../../actions";
import type { DeskJob } from "../../desk";
import type { AgentRow, MissionNode, SnapshotRest } from "../../snapshot";
import { useShellStore } from "../../store";
import { TooltipProvider } from "../ui/tooltip";
import { SessionHead } from "./desk-session";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

// The session head beside Desk.

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

describe("the session panel's head (Doop H 다, operator decision 2026-11-25)", () => {
  const job = (activity: DeskJob["activity"]): DeskJob => ({
    id: "pane:w1:p2", agent_id: null, pane_id: "w1:p2", title: "끊긴 뒤 로그인 고치기", agent_kind: "claude", project: { workspace_id: "w", label: "repo" }, location: { kind: "checkout", label: "main" },
    state: "open", activity, queue_position: null, now: null, plan: null, question: null, summary: null, pull_requests: [], outputs: [], joined: false, reason: null, failure: null, ask: null,
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
        <SessionHead paneId="w1:p2" title="끊긴 뒤 로그인 고치기" job={{ ...job("working"), agent_id: agentId }} row={row} pane={null} machine={machine} disconnected={disconnected} sleeping={false} explorerOpen={false} filesChord="⇧⌘E" terminalChord="⌘J" deskActions={actions} onFiles={noop} onTerminal={noop} onOwnTab={noop} />
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

  it("draws the mission it conducts as the two-line chip with the pane's hover card, which a press still opens as the map", async () => {
    const saved = useShellStore.getState();
    const mission = { id: "mission-101", title: "Desk 2차", device_id: "mac", status: "running", nodes: [], conductor: { pane_id: "w1:p2", present: true, session: null }, decision: null, question: null, delay: null, closed_at_unix_ms: null, created_at_unix_ms: 1 };
    useShellStore.setState({ rest: { missions: { items: [mission] } } as unknown as SnapshotRest, agents: [{ pane_id: "w1:p2", device_id: "mac" } as AgentRow] });
    try {
      const container = await head(null);
      const chip = container.querySelector<HTMLElement>("[data-desk-session-meta] [data-mission-chip]")!;
      expect([chip.dataset.missionChipCard, chip.textContent]).toEqual(["true", "M101Desk 2차0/0"]);
      expect(document.body.querySelector("[data-pane-mission]")).toBeNull();
      await act(async () => chip.dispatchEvent(new MouseEvent("pointerover", { bubbles: true, relatedTarget: null })));
      const card = document.body.querySelector<HTMLElement>("[data-pane-mission='mission-101']")!;
      // Radix keeps a card with no anchor above the screen (`translate(0, -200%)`).
      await vi.waitFor(() => expect(card.parentElement!.style.transform).toMatch(/^translate\([^%]*\)$/));
    } finally {
      useShellStore.setState(saved, true);
    }
  });
});
