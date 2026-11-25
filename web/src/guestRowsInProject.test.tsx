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

// Another machine's work in this computer's 프로젝트 list belongs to the project whose folder holds it (B10, operator
// review 2026-11-25, docs/UI_BEHAVIOR.md "Another machine's work on this computer"): one head per machine inside the
// project, which folds itself, hidden with the project's fold and with the 다른 체크아웃 fold its checkout waits in, and
// the fold that hides the pane the center shows is marked as holding it.
it("draws another machine's work inside the project under one head, hides it with a fold, and marks the fold that holds the pane in front", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  const tokens = (JSON.parse(tokensText) as { tokens: Record<string, { value: number }> }).tokens;
  const widthTokens = ["--size-sidebar-ideal", "--size-sidebar-min", "--size-sidebar-max"];
  for (const name of widthTokens) document.documentElement.style.setProperty(name, `${tokens[name]?.value}px`);
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  const row = (id: string, path: string, branch: string, primary: boolean) =>
    ({ agent_scope: emptyScope(), id, workspace_id: "project", label: branch, path, branch, is_primary: primary, is_worktree: !primary, exists: true, active_tab_id: null, tabs: [], strip: [] }) as unknown as Checkout;
  const project = (expanded: boolean, othersOpen: boolean): Workspace => ({ agent_scope: emptyScope(),
    id: "project", label: "Studio", path: "/fixture", device_id: "local", is_git: true, registered: true, temporary: false, pinned: false, expanded,
    checkouts: [row("main", "/fixture", "main", true), row("old", "/fixture.worktrees/old", "feature/old", false)],
    inactive_checkouts: { expanded: othersOpen, checkout_ids: ["old"] },
  }) as Workspace;
  const key = "b".repeat(64);
  const pane = (id: string, folder: string, name: string) => ({ id: `guest:macbook:pane:${id}`, pane: id, since_unix_ms: 1, folder, name, mark: "going", agent: "codex" });
  const catalog = (expanded: boolean, othersOpen: boolean, folded: { device_id: string; folder: string }[] = []) =>
    ({ ui_state: { folded_guest_work: folded }, navigator: {
      devices: [{ id: "local", label: "This Mac", kind: "local", state: "local", agent_scope: {} }], workspaces: [project(expanded, othersOpen)], agents: [], focused_device_id: "local",
      peers: [{ key, device_id: "macbook", name: "MacBook Pro", opens: null, opened_by: "active", connected: true, path: "direct" }],
      guest_panes: [{ device_id: "macbook", key, panes: [pane("w1:p1", "/fixture/src", "결제 재시도 고치기"), pane("w1:p2", "/fixture.worktrees/old", "주소 경계 확인"), pane("w1:p3", "/elsewhere", "로그 모으기")] }],
    } }) as unknown as SnapshotRest;
  useShellStore.setState({ rest: catalog(true, false), agents: [], connection: "live" });
  useUiStore.setState({ screen: { kind: "main" } });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const inProject = () => container.querySelector('[data-project="project"]')!;
  const guestRow = (id: string) => container.querySelector(`[data-guest-row="guest:macbook:pane:${id}"]`);
  const headOf = (place: "project" | "outside") => container.querySelector<HTMLButtonElement>(`[data-guest-machines="${place}"] [data-guest-head="macbook"]`);
  const head = (place: "project" | "outside") => headOf(place)?.textContent;
  try {
    await act(async () => root.render(<TooltipProvider><Sidebar actions={actions} /></TooltipProvider>));
    // The main checkout has no row of its own: its pane stands at the project's end, inside the project, under the machine's one head.
    expect(inProject().contains(guestRow("w1:p1"))).toBe(true);
    expect(head("project")).toBe("MPFrom MacBook Pro 1");
    expect(headOf("project")?.getAttribute("aria-expanded")).toBe("true");
    expect(guestRow("w1:p1")?.querySelector("[data-guest-mark]")).toBeNull();
    expect(guestRow("w1:p1")?.getAttribute("aria-label")).toBe("결제 재시도 고치기 · from MacBook Pro");
    // The worktree waits in the closed 다른 체크아웃 fold: its pane is hidden with it.
    expect(guestRow("w1:p2")).toBeNull();
    // A folder no project holds stands at the list's end, under the same head as every other place.
    expect(inProject().contains(guestRow("w1:p3"))).toBe(false);
    expect(head("outside")).toBe("MPFrom MacBook Pro 1");

    // A head folds as the list's other folds do: the click asks the core, which keeps it by machine and the place's folder.
    await act(async () => headOf("project")?.click());
    expect(events.filter((event) => event.kind === "guest_work_fold_toggle")).toEqual([
      { schema_version: 2, kind: "guest_work_fold_toggle", payload: { device_id: "macbook", folder: "/fixture" } },
    ]);
    expect(guestRow("w1:p1"), "the click changes nothing until the core answers").not.toBeNull();
    // Folded, it shows its count and nothing more; the same machine's head outside the project stays open.
    await act(async () => useShellStore.setState({ rest: catalog(true, false, [{ device_id: "macbook", folder: "/fixture" }]) }));
    expect(guestRow("w1:p1")).toBeNull();
    expect(headOf("project")?.getAttribute("aria-expanded")).toBe("false");
    expect(head("project")).toBe("MPFrom MacBook Pro 1");
    expect(guestRow("w1:p3")).not.toBeNull();
    await act(async () => useShellStore.setState({ rest: catalog(true, false) }));

    // The center shows the hidden pane: the fold that holds it is marked, as for this computer's own checkout.
    const othersFold = () => container.querySelector('[data-other-checkouts="project"]');
    expect(othersFold()?.getAttribute("aria-current")).toBeNull();
    await act(async () => useUiStore.setState({ screen: { kind: "guest", paneId: "guest:macbook:pane:w1:p2" } }));
    expect(othersFold()?.getAttribute("aria-current")).toBe("true");

    // The fold opened: the worktree's row and the pane after it.
    await act(async () => useShellStore.setState({ rest: catalog(true, true) }));
    expect(container.querySelector('[data-checkout-row="old"]')?.contains(guestRow("w1:p2"))).toBe(true);
    expect(othersFold()?.getAttribute("aria-current")).toBeNull();

    // The project folded: none of its panes shows, the outside one still does, and the project's row holds the pane in front.
    await act(async () => useShellStore.setState({ rest: catalog(false, true) }));
    expect([guestRow("w1:p1"), guestRow("w1:p2")]).toEqual([null, null]);
    expect(guestRow("w1:p3")).not.toBeNull();
    expect(container.querySelector('[data-project-row="project"]')?.getAttribute("aria-current")).toBe("true");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    for (const name of widthTokens) document.documentElement.style.removeProperty(name);
    vi.unstubAllGlobals();
  }
});
