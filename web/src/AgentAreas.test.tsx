// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { AgentAreas } from "./AgentAreas";
import { InlineStartHost } from "./AgentTabStart";
import { WorkspaceNotices } from "./WorkspaceDialogs";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";
import type { AgentLayout, AgentStartReceipt } from "./agentLayout";
import { TooltipProvider } from "./components/ui/tooltip";
import { inlineStartKey, useInlineStarts } from "./startDraft";
import { receiptTarget } from "./inlineStart";
import type { Checkout, PaneRow, Tab, TaskOperation } from "./snapshot";
import { useShellStore } from "./store";
import { emptyScope, legacyRest } from "../test/legacyAgentScope";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

const pane: PaneRow = { id: "p1", device_id: "local", device_pane_id: "p1", engine_label: "p1", terminal_title: null, cwd: "/fixture", status_code: "approval", requires_close_confirmation: false, requires_close_status_check: false, identity_label: null };
const tab: Tab = { id: "t1", workspace_id: "project", checkout_id: "checkout", label: "Claude", empty: false, delegated: false, panes: [pane], sidebar: { number: 1, manual_name: null, named_by_operator: false, focused_pane_id: "p1" } };
// The start that filled draft `draft-1` with the live tab `t1`, whose pane p1 runs.
const started: AgentStartReceipt = { request_id: "request-1", group_id: null, area_id: "a1", draft_tab_id: "draft-1", live_tab_id: "t1", pane_id: "p1", add_pane: false, state: "ready", agent_kind: "claude", agent_phase: "started" };

let root: Root;
let container: HTMLDivElement;
let saved: ReturnType<typeof useShellStore.getState>;
let ui: ReturnType<typeof useUiStore.getState>;
let events: Parameters<DispatchFn>[0][];
beforeEach(() => {
  saved = useShellStore.getState(); ui = useUiStore.getState(); events = [];
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  // The area tree lays its areas out in the size it measures; jsdom measures none.
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(800);
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(600);
  for (const [name, value] of [["--size-tab-preferred", "180"], ["--size-tab-title-min", "104"], ["--size-tab-selected-max", "240"], ["--size-tab-icon-identity", "40"], ["--size-control-sm", "24"]] as const) document.documentElement.style.setProperty(name, value);
  useInlineStarts.setState({ drafts: {}, retired: new Set() });
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount()); container.remove();
  useInlineStarts.setState({ drafts: {}, retired: new Set() }); useShellStore.setState(saved, true); useUiStore.setState(ui, true);
  document.documentElement.removeAttribute("style");
  vi.restoreAllMocks(); vi.unstubAllGlobals();
});

const unrelated: TaskOperation = { id: 9, request_id: "another-start", kind: "agent_start", phase: "ready", repository_root: null, branch: null, base_branch: null, path: "/fixture", pane_id: "p9", agent_kind: "codex", failure: null, agent_phase: "started", agent_reason: null, agent_line: null };

type Origin = "after a restart" | "beside another start's answer" | "on the page that sent it";
const show = async (receipt: AgentStartReceipt, origin: Origin) => {
  const checkout: Checkout = { id: "checkout", workspace_id: "project", label: "Fixture", path: "/fixture", branch: null, purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, active_tab_id: "t1", strip: [{ id: "s1", kind: "engine", source_id: "t1", label: "Claude", preview: false }], next_tab_label: "2", tabs: [tab], agent_scope: emptyScope(), agent_groups: { selected: null, groups: [{ id: null, name: null, tabs: ["t1"] }], drafts: [], start_receipts: [receipt] } };
  const layout: AgentLayout = { root: { area: { id: "a1", active: "t1", displays: [{ id: "t1" }] } }, active_area: "a1", canvases: {}, group_id: null, groups: [{ id: null, name: null, tabs: ["t1"] }], drafts: [], limits: { areas: 6, depth: 3, displays: 64 }, display_count: 1, start_receipts: [receipt] };
  const rest = legacyRest({ navigator: { devices: [{ id: "local", kind: "local", label: "Fixture", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null, agent_scope: emptyScope() }], workspaces: [{ id: "project", path: "/fixture", device_id: "local", label: "Fixture", registered: true, temporary: false, pinned: false, agent_scope: emptyScope(), inactive_checkouts: { expanded: false, checkout_ids: [] }, checkouts: [checkout] }] } }, []);
  // The page that sent the start still holds its draft, keyed by the draft tab
  // the start filled; after a restart the page holds nothing.
  if (origin === "on the page that sent it") {
    const target = receiptTarget(receipt);
    useInlineStarts.getState().put(inlineStartKey("local", "/fixture", target), { text: "say hi", selection: { kind: "claude", model: null }, request: { id: receipt.request_id, sentAt: Date.now(), provider: "claude", model: null, prompt: "say hi", target }, phase: "pending", taskId: null, failure: null });
  }
  await act(async () => useShellStore.setState({ connection: "live", rest: { ...rest, ...(origin === "beside another start's answer" ? { task_operation: unrelated } : {}), workspace_view: { device_id: "local", path: "/fixture", open: false, tray: { tab_id: "t1", shown: false, count: 0 }, tool: null, document_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view", agent_layout: layout } } as never }));
  await act(async () => {
    const actions = createActions((event) => { events.push(event); return true; });
    root.render(<TooltipProvider><InlineStartHost /><AgentAreas checkout={checkout} actions={actions} /><WorkspaceNotices actions={actions} /></TooltipProvider>);
  });
};

// with-hide/hide issue 2927: a device's Claude stood on its folder-trust question, so
// its first prompt was never confirmed and the start form covered the terminal
// the operator had to answer it in. A tab whose pane is alive shows that pane
// whatever its start's receipt says, and the terminal is where it is answered.
const receipts: [string, Partial<AgentStartReceipt>][] = [
  ["an unconfirmed first prompt", { agent_phase: "unknown" }],
  ["an agent still starting", { agent_phase: "starting" }],
  ["a start still being confirmed", { state: "pending", agent_phase: null }],
  ["an agent that failed to start", { agent_phase: "failed" }],
];
const origins: Origin[] = ["after a restart", "beside another start's answer", "on the page that sent it"];
it.each(receipts.flatMap(([label, receipt]) => origins.map((origin) => [label, origin, receipt] as const)))("a live tab with %s, %s, shows its pane and no start form", async (_label, origin, receipt) => {
  await show({ ...started, ...receipt }, origin);
  expect(container.querySelector('[data-canvas="t1"], [data-canvas="empty"]')).not.toBeNull();
  expect(container.querySelector("[data-agent-tab-start]")).toBeNull();
  // No start form is held for the live tab out of sight either: only the
  // sending page's own draft remains, and a reloaded page restores none.
  expect(Object.keys(useInlineStarts.getState().drafts)).toHaveLength(origin === "on the page that sent it" ? 1 : 0);
});

// The terminal answers what the agent shows, but not why its start failed or
// how to retry it. After a restart, or once another start took the core's one
// task slot, only the saved receipt is left; the start notice draws it and its
// check recovers the original intent in the same pane, whose answer comes back
// as the ordinary task notice: Retry for a failed agent, the terminal otherwise.
it.each((["after a restart", "beside another start's answer"] as const).flatMap((origin) => (["unknown", "failed"] as const).map((phase) => [phase, origin] as const)))("a live tab's %s start, %s, is reported and recovered from the start notice", async (phase, origin) => {
  await show({ ...started, agent_phase: phase }, origin);
  expect(container.querySelector("[data-agent-tab-start]")).toBeNull();
  const notice = container.querySelector(`[data-live-start="${started.request_id}"]`);
  expect(notice?.getAttribute("data-task-agent")).toBe(phase);
  await act(async () => container.querySelector<HTMLButtonElement>("[data-live-start-check]")!.click());
  const check = events.filter((event) => event.kind === "agent_start_in_checkout");
  expect(check).toHaveLength(1);
  expect(check[0]!.payload).toMatchObject({ checkout_path: "/fixture", provider: "claude", request_id: started.request_id, confirm_only: true, target: { group_id: null, area_id: "a1", tab_id: "draft-1", add_pane: false } });

  // The core recovers the original intent and answers it in the one task slot.
  const answer: TaskOperation = { ...unrelated, id: 12, request_id: started.request_id, pane_id: "p1", agent_kind: "claude", agent_phase: phase, agent_reason: phase === "failed" ? "agent_start.pane_restored" : "agent_start.pane_restored_unconfirmed" };
  await act(async () => useShellStore.setState({ rest: { ...useShellStore.getState().rest!, task_operation: answer } }));
  expect(container.querySelector("[data-live-start]")).toBeNull();
  expect(container.querySelector(`[data-task-agent="${phase}"]`)).not.toBeNull();
  const retry = container.querySelector<HTMLButtonElement>("[data-task-agent-retry]");
  if (phase === "failed") {
    await act(async () => retry!.click());
    expect(events.at(-1)).toMatchObject({ kind: "task_agent_retry", payload: { id: 12 } });
  } else expect(retry).toBeNull();
  expect(container.querySelector('[data-canvas="t1"], [data-canvas="empty"]')).not.toBeNull();

  // Dismissed, the answer stays gone for this page, from the task and the receipt alike.
  await act(async () => container.querySelector<HTMLButtonElement>(`[data-task-agent="${phase}"] [aria-label="Dismiss"]`)!.click());
  await act(async () => useShellStore.setState({ rest: { ...useShellStore.getState().rest!, task_operation: unrelated } }));
  expect(container.querySelector("[data-task-agent]")).toBeNull();
});

// A check can itself fail (the original terminal could not be proven, the
// device dropped) or be refused; the notice keeps the start, says why at once,
// and offers the same intent's check again, while the terminal stays in view.
it.each(["unknown", "failed"] as const)("a live tab's %s start keeps its notice through a check that runs, fails and is refused", async (phase) => {
  await show({ ...started, agent_phase: phase }, "after a restart");
  const notice = () => container.querySelector(`[data-live-start="${started.request_id}"]`);
  const check = () => container.querySelector<HTMLButtonElement>("[data-live-start-check]")!;
  const shows = () => [...notice()!.querySelectorAll("button")].some((button) => button.textContent === "Show pane");
  const checks = () => events.filter((event) => event.kind === "agent_start_in_checkout");
  const answer = async (patch: Record<string, unknown>) => act(async () => useShellStore.setState({ rest: { ...useShellStore.getState().rest!, ...patch } }));
  const working: TaskOperation = { ...unrelated, id: 13, request_id: started.request_id, phase: "working", pane_id: null, agent_kind: "claude", agent_phase: null };

  await act(async () => check().click());
  await answer({ task_operation: working });
  expect(notice()?.getAttribute("data-live-start-checking")).toBe("true");
  expect(check().disabled).toBe(true);
  expect(shows()).toBe(true);

  await answer({ task_operation: { ...working, phase: "failed", failure: "agent_start.terminal_unchecked" } });
  expect(notice()?.querySelector("[data-live-start-failure]")?.getAttribute("data-live-start-failure")).toBe("agent_start.terminal_unchecked");
  expect(check().disabled).toBe(false);
  expect(shows()).toBe(true);
  await act(async () => check().click());

  // The next check is refused while the failed one still holds the task slot: the refusal is the newer answer.
  const refusal = (kind: string) => ({ status: { ...useShellStore.getState().rest!.status, last_error: { kind, message: "", retryable: true, occurred_at: 1, request_id: started.request_id } } });
  await answer(refusal("agent_start.device_unavailable"));
  expect(notice()?.querySelector("[data-live-start-failure]")?.getAttribute("data-live-start-failure")).toBe("agent_start.device_unavailable");
  expect(check().disabled).toBe(false);
  expect(shows()).toBe(true);
  await act(async () => check().click());

  // Refused again after another start took the slot.
  await answer({ task_operation: unrelated, ...refusal("agent_start.confirmation_unavailable") });
  expect(notice()?.querySelector("[data-live-start-failure]")?.getAttribute("data-live-start-failure")).toBe("agent_start.confirmation_unavailable");
  expect(check().disabled).toBe(false);
  expect(shows()).toBe(true);
  await act(async () => check().click());

  // Every check is the original intent, confirmed only, at its original target.
  expect(checks()).toHaveLength(4);
  for (const event of checks()) expect(event.payload).toMatchObject({ request_id: started.request_id, confirm_only: true, provider: "claude", target: { tab_id: "draft-1", area_id: "a1", add_pane: false } });
  expect(container.querySelector("[data-agent-tab-start]")).toBeNull();
  expect(container.querySelector('[data-canvas="t1"], [data-canvas="empty"]')).not.toBeNull();
});
