// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, expect, it, vi } from "vitest";
import tokensText from "../../design/tokens.json?raw";
import { emptyScope, legacyRest } from "../test/legacyAgentScope";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { Sidebar } from "./sidebar";
import type { Checkout, Device, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { useGroupFolds } from "./groupTabs";
import type { DispatchFn } from "./ws";

const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

it("opens groups and drafts in a checkout with no live tabs or agents, and folding a group does not focus it", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  const tokens = (JSON.parse(tokensText) as { tokens: Record<string, { value: number }> }).tokens;
  for (const name of ["--size-sidebar-ideal", "--size-sidebar-min", "--size-sidebar-max"]) document.documentElement.style.setProperty(name, `${tokens[name]?.value}px`);
  const local: Device = { id: "local", label: "This Mac", kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null, agent_scope: emptyScope() };
  const checkout: Checkout = { id: "checkout", workspace_id: "project", path: "/fixture", label: "Fixture", branch: null, purpose: null, is_worktree: false, exists: true, has_panes: false, pull_request: null, active_tab_id: null, strip: [], next_tab_label: "1", tabs: [], agent_scope: emptyScope(), agent_groups: { selected: "work", drafts: ["draft-1"], groups: [{ id: null, name: null, tabs: [] }, { id: "empty", name: "Preparation", tabs: [] }, { id: "work", name: "Current task", tabs: ["draft-1"] }] } };
  const workspace: Workspace = { id: "project", label: "Fixture", path: "/fixture", device_id: "local", registered: true, temporary: false, pinned: false, is_git: false, agent_scope: emptyScope(), inactive_checkouts: { expanded: false, checkout_ids: [] }, checkouts: [checkout] };
  const shell = useShellStore.getState(), ui = useUiStore.getState(), folds = useGroupFolds.getState();
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
  try {
    useGroupFolds.setState({ closed: new Set() });
    useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false });
    useShellStore.setState({ connection: "live", agents: [], rest: legacyRest({ navigator: { devices: [local], workspaces: [workspace], focused_device_id: "local", focused_checkout_id: checkout.id } }, []) });
    await act(async () => root.render(<TooltipProvider><Sidebar actions={actions} /></TooltipProvider>));
    expect(container.querySelector('[data-checkout-toggle="checkout"]')).not.toBeNull();
    expect(container.querySelector('[data-sidebar-group-select="empty"]')?.textContent).toContain("Preparation");
    const selectedGroup = () => container.querySelector('[data-sidebar-group-select][aria-current="true"]');
    expect(selectedGroup()?.getAttribute("data-sidebar-group-select")).toBe("work");
    const rest = useShellStore.getState().rest!;
    await act(async () => useShellStore.setState({ rest: { ...rest, navigator: { ...rest.navigator, workspaces: [{ ...workspace, checkouts: [{ ...checkout, agent_groups: { ...checkout.agent_groups!, selected: "empty" } }] }] } } }));
    expect(selectedGroup()?.getAttribute("data-sidebar-group-select")).toBe("empty");
    expect(selectedGroup()?.classList.contains("font-medium")).toBe(true);
    expect(selectedGroup()?.querySelector("svg")?.classList.contains("text-foreground")).toBe(true);
    expect(container.querySelector('[data-sidebar-group-select="work"]')?.hasAttribute("aria-current")).toBe(false);
    const group = container.querySelector('[data-sidebar-named-group="work"]')!;
    expect(group.querySelector('[data-sidebar-tab="draft-1"]')).not.toBeNull();
    const before = events.length;
    await act(async () => group.querySelector<HTMLButtonElement>("[aria-expanded]")!.click());
    expect(group.querySelector('[data-sidebar-tab="draft-1"]')).toBeNull();
    expect(events).toHaveLength(before);
    await act(async () => group.querySelector<HTMLButtonElement>("[aria-expanded]")!.click());
    await act(async () => group.querySelector<HTMLButtonElement>('[data-sidebar-tab="draft-1"]')!.click());
    expect(events.at(-1)).toMatchObject({ kind: "agent_group", payload: { workspace: { device_id: "local", path: "/fixture" }, action: "select", group_id: "work", tab_id: "draft-1" } });
  } finally {
    await act(async () => root.unmount()); container.remove();
    useShellStore.setState(shell, true); useUiStore.setState(ui, true); useGroupFolds.setState(folds, true); vi.unstubAllGlobals();
    for (const name of ["--size-sidebar-ideal", "--size-sidebar-min", "--size-sidebar-max"]) document.documentElement.style.removeProperty(name);
  }
});
