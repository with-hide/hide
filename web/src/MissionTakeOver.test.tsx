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
import { START_ANSWER_TIMEOUT_MS } from "./startAnswer";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); document.body.innerHTML = ""; });

const gone = (session: Mission["conductor"]["session"]): Mission => ({ id: "mission-1", title: "Fix the bug", workspace_id: "w", project: "/repo", checkout_path: "/repo", device_id: "mac", status: "running", scope: "pr", cap: 4, exempt: false, strength: null, conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session, present: false, takes_answers: true }, nodes: [], merge_order: [], landing_order: [], stack_waiting: 0, check: null, github: null, decision: null, question: null, answer: null, answer_failed: null, delay: null, report: null, created_at_unix_ms: 1, started_at_unix_ms: 1, closed_at_unix_ms: null, running: 0, stale: false, github_stale: false, github_read_at_unix_ms: null });

async function mount(mission: Mission, originalExists?: boolean) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const base = { path: "/repo", exists: true, is_primary: true };
  const workspaces = originalExists === undefined ? [] : [
    { device_id: "wrong-device", path: "/repo", checkouts: [base, { path: "/gone", exists: true }] },
    { device_id: mission.device_id, path: "/repo", checkouts: [base, { path: "/gone", exists: originalExists }] },
  ];
  useShellStore.setState({ rest: { navigator: { devices: [{ id: "mac", kind: "local", label: "This Mac" }], workspaces: [] }, missions: { items: [mission] }, status: { remote: [{ target_id: mission.device_id, state: "connected", session: { workspaces } }] } } as unknown as SnapshotRest });
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
  const beforeFailure = (requestId: unknown, pending: boolean) => {
    const checkout = workspaces[1]?.checkouts.find((checkout) => checkout.path === (pending ? "/repo" : "/gone"));
    if (checkout && pending) Object.assign(checkout, { agent_groups: { start_receipts: [{ request_id: requestId, state: "pending" }] } });
    if (checkout && !pending) checkout.exists = false;
    return act(async () => {
      useShellStore.setState((state) => ({ rest: { ...state.rest!, task_operation: { id: 1, request_id: requestId, kind: "agent_start", phase: "failed", failure: "task_operation.failed" } } as SnapshotRest }));
    });
  };
  return { button, starts, refuse, beforeFailure, unmount: async () => { await act(async () => root.unmount()); } };
}

it("resumes the gone Conductor's own conversation first, with no prompt", async () => {
  const view = await mount(gone({ device_id: "mac", agent: "claude", session_id: "native-1" }));
  try {
    await act(async () => view.button().click());
    expect(view.starts()).toHaveLength(1);
    expect(view.starts()[0]).toMatchObject({ checkout_path: "/repo", provider: "claude", resume_session_id: "native-1" });
    expect(view.starts()[0]!.prompt).toBeUndefined();
    // While the start is on its way the button says so and takes no second press.
    expect(view.button().textContent).toBe("Taking over…");
    expect(view.button().disabled).toBe(true);
  } finally { await view.unmount(); }
});

it.each([[false, "/repo"], [true, "/gone"]] as const)("resumes the same remote conversation with originalExists=%s", async (exists, checkout) => {
  const mission = { ...gone({ device_id: "device", agent: "claude", session_id: "native-1" }), device_id: "device", checkout_path: "/gone" };
  const view = await mount(mission, exists);
  try {
    await act(async () => view.button().click());
    expect(view.starts()).toHaveLength(1);
    expect(view.starts()[0]).toMatchObject({ checkout_path: checkout, device_id: "device", provider: "claude", resume_session_id: "native-1" });
    expect(view.starts()[0]!.prompt).toBeUndefined();
  } finally { await view.unmount(); }
});

it("confirms the same resume after a silent connection without starting a new agent", async () => {
  vi.useFakeTimers();
  const view = await mount(gone({ device_id: "mac", agent: "claude", session_id: "native-1" }));
  try {
    await act(async () => view.button().click());
    await act(async () => vi.advanceTimersByTime(START_ANSWER_TIMEOUT_MS));
    await act(async () => view.button().click());
    expect(view.starts()).toHaveLength(2);
    expect(view.starts()[1]).toMatchObject({ checkout_path: "/repo", request_id: view.starts()[0]!.request_id, confirm_only: true });
    await view.refuse(view.starts()[0]!.request_id);
    expect(view.starts()).toHaveLength(2);
    expect(view.button().getAttribute("data-mission-takeover")).toBe("failed");
  } finally { await view.unmount(); }
});

it("checks a pending creation receipt despite the task's failed confirmation", async () => {
  const mission = { ...gone({ device_id: "device", agent: "claude", session_id: "native-1" }), device_id: "device" };
  const view = await mount(mission, true);
  try {
    await act(async () => view.button().click());
    const requestId = view.starts()[0]!.request_id;
    await view.beforeFailure(requestId, true);
    expect(view.starts()).toHaveLength(1);
    await act(async () => view.button().click());
    expect(view.starts()[1]).toMatchObject({ request_id: requestId, confirm_only: true, resume_session_id: "native-1" });
  } finally { await view.unmount(); }
});

it("uses the primary checkout when the worktree disappears during a refused resume", async () => {
  const mission = { ...gone({ device_id: "device", agent: "claude", session_id: "native-1" }), device_id: "device", checkout_path: "/gone" };
  const view = await mount(mission, true);
  try {
    await act(async () => view.button().click());
    const requestId = view.starts()[0]!.request_id;
    await view.beforeFailure(requestId, false);
    expect(view.starts()).toHaveLength(2);
    expect(view.starts()[1]).toMatchObject({ checkout_path: "/repo", resume_session_id: "native-1" });
    expect(view.starts()[1]!.request_id).not.toBe(requestId);
    expect(view.starts()[1]!.prompt).toBeUndefined();
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
    expect(view.button().textContent).toBe("Resume command failed · Retry");
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
