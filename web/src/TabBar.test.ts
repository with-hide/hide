// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { closingSuffix } from "./TabBar";
import type { AsyncOperation } from "./snapshot";
import type { TabFit } from "./areaLayout";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

function op(kind: string, target_id: string, phase: string): AsyncOperation {
  return { id: `${kind}:${target_id}`, kind, target_id, scope_id: "c1", phase, stage: "", message: null, retryable: false };
}

describe("closingSuffix", () => {
  it("shows while the close is in flight and not after it settles", () => {
    expect(closingSuffix("t1", "tab.close", [op("tab.close", "t1", "transmitting")])).toBe(true);
    expect(closingSuffix("t1", "tab.close", [op("tab.close", "t1", "awaiting_topology")])).toBe(true);
    expect(closingSuffix("t1", "tab.close", [op("tab.close", "t1", "unknown")])).toBe(true);
    expect(closingSuffix("t1", "tab.close", [op("tab.close", "t1", "completed")])).toBe(false);
    expect(closingSuffix("t1", "tab.close", [op("tab.close", "t1", "failed")])).toBe(false);
    expect(closingSuffix("t1", "tab.close", [op("pane.close", "t1", "transmitting")])).toBe(false);
    expect(closingSuffix("t2", "tab.close", [op("tab.close", "t1", "transmitting")])).toBe(false);
  });
});

describe("tab composition", () => {
  it("uses the focused pane's provenance when two panes have the same title", async () => {
    const { act, createElement } = await import("react");
    const { createRoot } = await import("react-dom/client");
    const { AgentTab } = await import("./TabBar");
    const { TooltipProvider } = await import("./components/ui/tooltip");
    const { useShellStore } = await import("./store");
    const { legacyAgentRow } = await import("../test/legacyAgentRow");
    const saved = useShellStore.getState();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    const container = document.createElement("div");
    const root = createRoot(container);
    const agents = [
      legacyAgentRow({ pane_id: "p1", identity_label: "Same title", record_connection: "unconfirmed" }),
      legacyAgentRow({ pane_id: "p2", identity_label: "Same title", record_connection: "confirmed" }),
    ];
    useShellStore.setState({ agents });
    const render = async (focused: string) => {
      const checkout = {
        id: "c1", next_tab_label: "Tab 2",
        strip: [{ id: "herdr:t1", kind: "herdr", source_id: "t1", label: "Same title", preview: false }],
        tabs: [{ id: "t1", agent: null, sidebar: { focused_pane_id: focused }, panes: agents.map(({ pane_id, identity_label }) => ({ id: pane_id, identity_label })) }],
      } as unknown as import("./snapshot").Checkout;
      await act(async () => root.render(createElement(TooltipProvider, { children: createElement(AgentTab, { checkout, entry: checkout.strip[0]!, interaction: { selected: true, fit: "titled", areaActive: true, dragging: false, press: () => {}, select: () => {} }, renaming: false, onCancelRename: () => {}, actions: {} as import("./actions").Actions }) })));
    };
    try {
      await render("p2");
      expect(container.textContent).toContain("Same title");
      expect(container.querySelector("[data-record-connection]")).toBeNull();
      await render("p1");
      expect(container.querySelector("[data-record-connection]")?.getAttribute("data-record-connection")).toBe("unconfirmed");
    } finally {
      await act(async () => root.unmount());
      useShellStore.setState(saved, true);
      vi.unstubAllGlobals();
    }
  });

  it("renders the core's representative mark and the title, with no provider logo, and names the product", async () => {
    const { createElement } = await import("react");
    const { renderToStaticMarkup } = await import("react-dom/server");
    const { AgentTab } = await import("./TabBar");
    const { TooltipProvider } = await import("./components/ui/tooltip");
    const checkout = {
      id: "c1", next_tab_label: "Tab 2",
      strip: [{ id: "herdr:t1", kind: "herdr", source_id: "t1", label: "작업 제목", preview: false }],
      tabs: [{ id: "t1", panes: [], agent: { state: { mark_tone: { kind: "warning", read: false } }, agent_kind: "claude", mark: "turn", demand: "approval", activity: "stopped", emphasized: true, status_code: "approval" } }],
    } as unknown as import("./snapshot").Checkout;
    const markup = renderToStaticMarkup(createElement(TooltipProvider, { children: createElement(AgentTab, { checkout, entry: checkout.strip[0]!, interaction: { selected: true, fit: "titled", areaActive: true, dragging: false, press: () => {}, select: () => {} }, renaming: false, onCancelRename: () => {}, actions: {} as import("./actions").Actions }) }));
    const mark = markup.indexOf('data-tab-status="approval"');
    const title = markup.indexOf('작업 제목', mark);
    expect(mark).toBeGreaterThan(0);
    expect(title).toBeGreaterThan(mark);
    expect(markup).not.toContain('<img');
    expect(markup).not.toContain('data-agent-mark');
    expect(markup).toContain('Claude Code');
  });
});

// Render the public tab with snapshot identities and child chips from another
// tab. Only separate agent panes in this tab earn the approved count.
async function renderCount(identities: (string | null)[], fit: TabFit = "titled", renaming = false) {
  const { createElement } = await import("react");
  const { renderToStaticMarkup } = await import("react-dom/server");
  const { AgentTab } = await import("./TabBar");
  const { TooltipProvider } = await import("./components/ui/tooltip");
  const { clientI18n } = await import("./i18n/translator");
  await clientI18n.changeLanguage("en");
  const checkout = {
    id: "c1", next_tab_label: "Tab 2",
    strip: [{ id: "herdr:t1", kind: "herdr", source_id: "t1", label: "작업 제목", preview: false }],
    tabs: [
      { id: "t1", agent: null, panes: identities.map((identity_label, index) => ({ id: `device:p${index}`, identity_label, status_code: "unknown", children: identity_label === null ? null : { instrumented: true, uninstrumented_reason: null, uninstrumented_label: null, chips: [{ tone: { kind: "working", read: false }, pane_id: "another-tab:child", label: "Child elsewhere", detail: null, status_word_visible: false, agent_kind: "codex", demand: "none", activity: "working", emphasized: false, mark: "going", status_code: "working", delegated: true }] } })) },
      { id: "other-tab", agent: null, panes: [{ identity_label: "Other tab's agent" }] },
    ],
  } as unknown as import("./snapshot").Checkout;
  return renderToStaticMarkup(createElement(TooltipProvider, { children: createElement(AgentTab, { checkout, entry: checkout.strip[0]!, interaction: { selected: true, fit, areaActive: true, dragging: false, press: () => {}, select: () => {} }, renaming, onCancelRename: () => {}, actions: {} as import("./actions").Actions }) }));
}

describe("agent pane count", () => {
  it.each([
    { identities: [null], count: 0 },
    { identities: ["One agent", null], count: 1 },
    { identities: ["One agent", "Second agent", null], count: 2 },
    { identities: Array.from({ length: 12 }, (_, n) => `Agent ${n}`), count: 12 },
  ])("counts $count agent panes without shells or other tabs", async ({ identities, count }) => {
    const markup = await renderCount(identities);
    if (count >= 2) expect(markup).toContain(`data-agent-pane-count="${count}"`);
    else expect(markup).not.toContain("data-agent-pane-count");
    if (count > 0) expect(markup).toContain(`${count} agent pane`);
    else expect(markup).not.toContain("agent pane");
  });

  it.each(["compact", "marks"] as const)("retains the accessible count at %s density", async (fit) => {
    const markup = await renderCount(["One agent", "Second agent"], fit);
    expect(markup).not.toContain("data-agent-pane-count");
    expect(markup).toContain('aria-label="Terminal tab 작업 제목 · 2 agent panes"');
  });

  it("gives the rename input room while retaining the accessible count", async () => {
    const markup = await renderCount(["One agent", "Second agent"], "titled", true);
    expect(markup).toContain('data-renaming="true"');
    expect(markup).not.toContain("data-agent-pane-count");
    expect(markup).toContain('aria-label="Terminal tab 작업 제목 · 2 agent panes"');
  });
});
