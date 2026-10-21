import { emptyScope, legacyRest } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, expect, it, vi } from "vitest";
import tokensText from "../../design/tokens.json?raw";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { Sidebar } from "./sidebar";
import type { AgentRow, SnapshotRest, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

// This flow does not render a terminal. Answer xterm's canvas capability probe at the browser boundary, before the
// real sidebar imports it.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

const LOCAL = { id: "local", label: "This Mac", kind: "local", state: "local", message: null };

function checkout(project: string, name: string, panes: string[]) {
  return { agent_scope: emptyScope(),
    id: `checkout:${project}-${name}`, workspace_id: `workspace:${project}`, label: name, path: `/fixture/${project}/${name}`, branch: name,
    is_primary: name === "main", is_worktree: name !== "main", exists: true, active_tab_id: `tab:${project}-${name}`, strip: [],
    worktree: { branch: name, head_sha: null, last_commit_unix_seconds: null },
    tabs: [{ id: `tab:${project}-${name}`, label: name, panes: panes.map((id) => ({ id })) }],
  };
}

/** Alpha runs a parent and its delegated child in main and a root in an Inactive checkout; Beta is folded. */
function catalog(focusedCheckout: string, inactiveOpen = false): { rest: SnapshotRest; agents: AgentRow[] } {
  const alpha = { agent_scope: emptyScope(),
    id: "workspace:alpha", label: "alpha", path: "/fixture/alpha", device_id: "local", is_git: true, registered: true, temporary: false, pinned: false,
    checkouts: [checkout("alpha", "main", ["pane:parent", "pane:child"]), checkout("alpha", "old", ["pane:old"])],
    inactive_checkouts: { expanded: inactiveOpen, checkout_ids: ["checkout:alpha-old"] },
  } as unknown as Workspace;
  const beta = { agent_scope: emptyScope(),
    id: "workspace:beta", label: "beta", path: "/fixture/beta", device_id: "local", is_git: true, registered: true, temporary: false, pinned: false,
    expanded: false, checkouts: [checkout("beta", "main", ["pane:beta"])], inactive_checkouts: { expanded: false, checkout_ids: [] },
  } as unknown as Workspace;
  const agent = (pane: string, label: string, lineage: Partial<AgentRow> = {}) => legacyAgentRow({
    id: `agent:${pane}`, pane_id: pane, identity_label: label, agent_kind: "claude", mark: "going", group: "working",
    status_code: "working", changed_at_unix_ms: null, emphasized: false, unread: false, demand: "none", activity: "working", ...lineage,
  }) as AgentRow;
  const agents = [
    agent("pane:parent", "부모 작업", { lineage_child_pane_ids: ["pane:child"] }),
    agent("pane:child", "위임된 작업", { delegated: true, lineage_parent_pane_id: "pane:parent" }),
    agent("pane:old", "예전 작업"),
    agent("pane:beta", "베타 작업"),
  ];
  const navigator = { devices: [LOCAL], workspaces: [alpha, beta], agents, focused_device_id: "local", focused_checkout_id: focusedCheckout };
  return { rest: legacyRest({ navigator } as unknown as SnapshotRest, agents), agents };
}

// docs/UI_BEHAVIOR.md, Sidebar hierarchy: the row that stands for the focus carries the selection, the list brings it
// into view when the focus moves, and no fold opens by itself.
it("selects and brings into view the row that stands for the focused pane, opening no fold", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  // jsdom loads no stylesheet, and the sidebar reads its width tokens from the document: take them from the token source.
  const tokens = (JSON.parse(tokensText) as { tokens: Record<string, { value: number }> }).tokens;
  const widthTokens = ["--size-sidebar-ideal", "--size-sidebar-min", "--size-sidebar-max"];
  for (const name of widthTokens) document.documentElement.style.setProperty(name, `${tokens[name]?.value}px`);
  // jsdom lays nothing out: the list is a 100-high box, and every row stands where `rowTop` says.
  let rowTop = 500;
  const rect = (top: number, height: number) => ({ top, bottom: top + height, left: 0, right: 0, width: 0, height, x: 0, y: top, toJSON() {} }) as DOMRect;
  const layout = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    return this.matches("[data-project-list]") ? rect(0, 100) : rect(rowTop, 28);
  });
  const scrolled: Element[] = [];
  const scrollIntoView = Element.prototype.scrollIntoView;
  Element.prototype.scrollIntoView = function (this: Element) { scrolled.push(this); };
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const show = async (focusedCheckout: string, pane: string, inactiveOpen = false) => {
    const { rest, agents } = catalog(focusedCheckout, inactiveOpen);
    await act(async () => useShellStore.setState({ rest, agents, focusedPaneId: pane }));
  };
  const current = () => [...container.querySelectorAll('[data-project-list] [aria-current="true"]')];
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false });
    const first = catalog("checkout:alpha-main");
    useShellStore.setState({ rest: first.rest, agents: first.agents, connection: "live", focusedPaneId: "pane:child" });
    await act(async () => root.render(<TooltipProvider><Sidebar actions={actions} /></TooltipProvider>));

    // A delegated child has no row of its own: its parent's row carries the selection and is brought into view.
    const parent = container.querySelector('[data-agent-open="pane:parent"]');
    expect(container.querySelector('[data-agent-open="pane:child"]')).toBeNull();
    expect(parent?.getAttribute("aria-current")).toBe("true");
    expect(scrolled).toEqual([parent]);

    // A snapshot that keeps the focus does not scroll a list the operator may have scrolled.
    await show("checkout:alpha-main", "pane:child");
    expect(scrolled).toHaveLength(1);

    // Beta is folded: its project row stands for its checkout, and the fold stays closed.
    await show("checkout:beta-main", "pane:beta");
    const betaRow = container.querySelector('[data-project-row="workspace:beta"]');
    expect(container.querySelector('[data-agent-open="pane:beta"]')).toBeNull();
    expect(current()).toEqual([betaRow]);
    expect(scrolled.at(-1)).toBe(betaRow);

    // A checkout under the folded Inactive group: the fold that holds it stands for it, and stays closed.
    await show("checkout:alpha-old", "pane:old");
    const inactive = container.querySelector('[data-inactive-checkouts="/fixture/alpha"]');
    expect(inactive?.getAttribute("aria-expanded")).toBe("false");
    expect(current()).toEqual([inactive]);
    expect(scrolled.at(-1)).toBe(inactive);
    expect(events.filter((event) => event.kind.includes("toggle"))).toEqual([]);

    // Another screen in front and back, the focus where it was: the list stays where the operator left it.
    const revealedSoFar = scrolled.length;
    await act(async () => useUiStore.setState({ screen: { kind: "main" } }));
    await act(async () => useUiStore.setState({ screen: { kind: "workspace" } }));
    expect(scrolled).toHaveLength(revealedSoFar);

    // A row already in view stays where it is when the focus moves to it.
    const before = scrolled.length;
    rowTop = 40;
    await show("checkout:alpha-main", "pane:parent");
    expect(container.querySelector('[data-agent-open="pane:parent"]')?.getAttribute("aria-current")).toBe("true");
    expect(scrolled).toHaveLength(before);

    // A section can cover a row before the next paint updates data-stuck (issue 1025).
    // Current geometry, rather than that stale marker, decides visibility.
    const header = container.querySelector<HTMLElement>("[data-section-sticky]")!;
    header.dataset.stuck = "false";
    layout.mockImplementation(function (this: HTMLElement) {
      if (this.matches("[data-project-list]")) return rect(0, 100);
      if (this.matches("[data-section-sticky]")) return rect(0, 26);
      return rect(10, 28);
    });
    await show("checkout:beta-main", "pane:beta");
    expect(scrolled).toHaveLength(before + 1);
    expect(scrolled.at(-1)).toBe(betaRow);

  } finally {
    await act(async () => root.unmount());
    container.remove();
    layout.mockRestore();
    Element.prototype.scrollIntoView = scrollIntoView;
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    for (const name of widthTokens) document.documentElement.style.removeProperty(name);
    vi.unstubAllGlobals();
  }
});
