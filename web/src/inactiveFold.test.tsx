import { emptyScope } from "../test/legacyAgentScope";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, expect, it, vi } from "vitest";
import tokensText from "../../design/tokens.json?raw";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { Sidebar } from "./sidebar";
import type { Checkout, SnapshotRest, Workspace } from "./snapshot";
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

// The Checkouts fold's click reaches the core as one `inactive_checkouts_toggle` naming the project, and the fold
// draws whatever `expanded` the core says back. What the core does with the event is owned by
// `inactive_fold_events_toggle_project_path_and_device_state_independently` in
// `hide-core/src/runtime/tests/projects.rs`; the e2e flows no longer click this fold.
it("sends the project's path when the Checkouts fold is clicked and draws the fold the core says is open", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  // jsdom loads no stylesheet, and the sidebar reads its width tokens from the document: take them from the token source.
  const tokens = (JSON.parse(tokensText) as { tokens: Record<string, { value: number }> }).tokens;
  const widthTokens = ["--size-sidebar-ideal", "--size-sidebar-min", "--size-sidebar-max"];
  for (const name of widthTokens) document.documentElement.style.setProperty(name, `${tokens[name]?.value}px`);
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  const row = (id: string, branch: string, primary: boolean) =>
    ({ agent_scope: emptyScope(), id, workspace_id: "project", label: branch, path: `/fixture/${id}`, branch, is_primary: primary, is_worktree: !primary, exists: true, active_tab_id: null, tabs: [], strip: [] }) as unknown as Checkout;
  const project = (expanded: boolean): Workspace => ({ agent_scope: emptyScope(),
    id: "project", label: "Studio", path: "/fixture", device_id: "local", is_git: true, registered: true, temporary: false, pinned: false,
    checkouts: [row("main", "main", true), row("old", "feature/old", false)],
    inactive_checkouts: { expanded, checkout_ids: ["old"] },
  });
  const catalog = (expanded: boolean) =>
    ({ navigator: { devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }], workspaces: [project(expanded)], agents: [], focused_device_id: "local" } }) as unknown as SnapshotRest;
  useShellStore.setState({ rest: catalog(false), agents: [], connection: "live" });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const fold = () => container.querySelector<HTMLButtonElement>('[data-other-checkouts="project"]');
  try {
    await act(async () => root.render(<TooltipProvider><Sidebar actions={actions} /></TooltipProvider>));
    expect(fold()?.getAttribute("aria-expanded")).toBe("false");
    expect(container.textContent).not.toContain("feature/old");

    await act(async () => fold()?.click());
    expect(events.filter((event) => event.kind === "inactive_checkouts_toggle")).toEqual([
      { schema_version: 2, kind: "inactive_checkouts_toggle", payload: { project_path: "/fixture" } },
    ]);
    // The click changes nothing by itself: the fold waits for the core's answer.
    expect(fold()?.getAttribute("aria-expanded")).toBe("false");

    await act(async () => useShellStore.setState({ rest: catalog(true) }));
    expect(fold()?.getAttribute("aria-expanded")).toBe("true");
    expect(container.textContent).toContain("feature/old");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    for (const name of widthTokens) document.documentElement.style.removeProperty(name);
    vi.unstubAllGlobals();
  }
});
