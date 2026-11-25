// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { AgentAreas } from "./AgentAreas";
import { InlineStartHost } from "./AgentTabStart";
import type { AgentLayout, AgentStartReceipt } from "./agentLayout";
import { TooltipProvider } from "./components/ui/tooltip";
import { useInlineStarts } from "./startDraft";
import type { Checkout, PaneRow, Tab } from "./snapshot";
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
const started: AgentStartReceipt = { request_id: "start-1", group_id: null, area_id: "a1", draft_tab_id: "draft-1", live_tab_id: "t1", pane_id: "p1", add_pane: false, state: "ready", agent_kind: "claude", agent_phase: "started" };

let root: Root;
let container: HTMLDivElement;
let saved: ReturnType<typeof useShellStore.getState>;
beforeEach(() => {
  saved = useShellStore.getState();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  // The area tree lays its areas out in the size it measures; jsdom measures none.
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(800);
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(600);
  for (const [name, value] of [["--size-tab-preferred", "180px"], ["--size-tab-title-min", "104px"], ["--size-tab-selected-max", "240px"], ["--size-tab-icon-identity", "40px"], ["--size-control-sm", "24px"]] as const) document.documentElement.style.setProperty(name, value);
  useInlineStarts.setState({ drafts: {}, retired: new Set() });
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount()); container.remove();
  useInlineStarts.setState({ drafts: {}, retired: new Set() }); useShellStore.setState(saved, true);
  document.documentElement.removeAttribute("style");
  vi.restoreAllMocks(); vi.unstubAllGlobals();
});

const show = async (receipt: AgentStartReceipt) => {
  const checkout: Checkout = { id: "checkout", workspace_id: "project", label: "Fixture", path: "/fixture", branch: null, purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, active_tab_id: "t1", strip: [{ id: "s1", kind: "engine", source_id: "t1", label: "Claude", preview: false }], next_tab_label: "2", tabs: [tab], agent_scope: emptyScope(), agent_groups: { selected: null, groups: [{ id: null, name: null, tabs: ["t1"] }], drafts: [], start_receipts: [receipt] } };
  const layout: AgentLayout = { root: { area: { id: "a1", active: "t1", displays: [{ id: "t1" }] } }, active_area: "a1", canvases: {}, group_id: null, groups: [{ id: null, name: null, tabs: ["t1"] }], drafts: [], limits: { areas: 6, depth: 3, displays: 64 }, display_count: 1, start_receipts: [receipt] };
  const rest = legacyRest({ navigator: { devices: [{ id: "local", kind: "local", label: "Fixture", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null, agent_scope: emptyScope() }], workspaces: [{ id: "project", path: "/fixture", device_id: "local", label: "Fixture", registered: true, temporary: false, pinned: false, agent_scope: emptyScope(), inactive_checkouts: { expanded: false, checkout_ids: [] }, checkouts: [checkout] }] } }, []);
  await act(async () => useShellStore.setState({ connection: "live", rest: { ...rest, workspace_view: { device_id: "local", path: "/fixture", open: false, tray: { tab_id: "t1", shown: false, count: 0 }, tool: null, document_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view", agent_layout: layout } } as never }));
  await act(async () => root.render(<TooltipProvider><InlineStartHost /><AgentAreas checkout={checkout} actions={createActions(() => true)} /></TooltipProvider>));
};

// with-hide/hide#2927: a device's Claude stood on its folder-trust question, so
// its first prompt was never confirmed and the start form covered the terminal
// the operator had to answer it in. A tab whose pane is alive shows that pane
// whatever its start's receipt says.
it.each<[string, Partial<AgentStartReceipt>]>([
  ["an unconfirmed first prompt", { agent_phase: "unknown" }],
  ["an agent still starting", { agent_phase: "starting" }],
  ["a start still being confirmed", { state: "pending", agent_phase: null }],
  ["an agent that failed to start", { agent_phase: "failed" }],
])("a live tab with %s shows its pane and no start form", async (_label: string, receipt: Partial<AgentStartReceipt>) => {
  await show({ ...started, ...receipt });
  // The receipt is one the page restores a start form for, so a form drawn
  // anywhere for this tab would be found here.
  expect(Object.keys(useInlineStarts.getState().drafts)).toHaveLength(1);
  expect(container.querySelector('[data-canvas="t1"], [data-canvas="empty"]')).not.toBeNull();
  expect(container.querySelector("[data-agent-tab-start]")).toBeNull();
});
