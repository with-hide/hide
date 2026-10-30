// @vitest-environment jsdom
// 이어받기 (PRD mission-tool D-11, B14, B15): the gone Conductor's own
// conversation first, a new agent asked to take the mission when it cannot
// be resumed, and a failed new start said on the button.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { initializeInterfaceI18n } from "./i18n/instance";
import { MissionTakeOver } from "./MissionTakeOver";
import type { Mission, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

afterEach(() => { vi.unstubAllGlobals(); document.body.innerHTML = ""; });

const gone = (session: Mission["conductor"]["session"]): Mission => ({ id: "mission-1", title: "Fix the bug", workspace_id: "w", project: "/repo", checkout_path: "/repo", device_id: "mac", status: "running", scope: "pr", cap: 4, exempt: false, strength: null, other_vendor: null, conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session, present: false, takes_answers: true }, nodes: [], merge_order: [], check: null, github: null, decision: null, question: null, answer: null, answer_failed: null, delay: null, report: null, created_at_unix_ms: 1, started_at_unix_ms: 1, closed_at_unix_ms: null, running: 0, stale: false, github_stale: false, github_read_at_unix_ms: null });

async function mount(mission: Mission) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  useShellStore.setState({ rest: { navigator: { devices: [{ id: "mac", kind: "local", label: "This Mac" }], workspaces: [] }, missions: { items: [mission] }, status: {} } as unknown as SnapshotRest });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<MissionTakeOver mission={mission} actions={actions} />));
  const button = () => container.querySelector<HTMLButtonElement>("[data-mission-takeover]")!;
  const starts = () => events.filter((event) => event.kind === "agent_start_in_checkout").map((event) => event.payload as Record<string, unknown>);
  const refuse = async (requestId: unknown) => {
    await act(async () => useShellStore.setState((state) => ({ rest: { ...state.rest!, status: { ...state.rest!.status, last_error: { kind: "agent_start.invalid_resume", message: "This session cannot be resumed", request_id: requestId } } } as unknown as SnapshotRest })));
  };
  return { button, starts, refuse, unmount: async () => { await act(async () => root.unmount()); } };
}

it("resumes the gone Conductor's own conversation first, with no prompt", async () => {
  const view = await mount(gone({ device_id: "mac", agent: "claude", session_id: "native-1" }));
  try {
    await act(async () => view.button().click());
    expect(view.starts()).toHaveLength(1);
    expect(view.starts()[0]).toMatchObject({ checkout_path: "/repo", provider: "claude", resume_session_id: "native-1" });
    expect(view.starts()[0]!.prompt).toBeUndefined();
  } finally { await view.unmount(); }
});

it("starts a new agent asked to take the mission once the resume is refused, and says so when that start fails too", async () => {
  const view = await mount(gone({ device_id: "mac", agent: "claude", session_id: "native-1" }));
  try {
    await act(async () => view.button().click());
    await view.refuse(view.starts()[0]!.request_id);
    expect(view.starts()).toHaveLength(2);
    expect(String(view.starts()[1]!.prompt)).toContain("hide mission take mission-1");
    expect(view.starts()[1]!.resume_session_id).toBeUndefined();
    await view.refuse(view.starts()[1]!.request_id);
    expect(view.button().getAttribute("data-mission-takeover")).toBe("failed");
    expect(view.button().textContent).toBe("Take over failed · Retry");
  } finally { await view.unmount(); }
});

it("starts a new agent at once for a Conductor with no conversation to resume", async () => {
  const view = await mount(gone(null));
  try {
    await act(async () => view.button().click());
    expect(view.starts()).toHaveLength(1);
    expect(String(view.starts()[0]!.prompt)).toContain("hide mission take mission-1");
  } finally { await view.unmount(); }
});
