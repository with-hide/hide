import { emptyScope, legacyRest } from "../test/legacyAgentScope";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import type { Checkout, SnapshotRest, WorktreeRow } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { WorkspaceDialogs } from "./WorkspaceDialogs";
import type { DispatchFn } from "./ws";

// No terminal is rendered in this dialog. Answer xterm's browser capability
// probe without substituting the confirmation, actions or snapshot store.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

const names = ["node_modules/vendor/beta", "target/deep/alpha"];
function row(): WorktreeRow {
  return {
    path: "/projects/example/task", branch: "feature/delete", head_sha: "abc",
    is_main: false, missing: false, dirty: false, changed_file_count: 0, pane_count: 0,
    lock_reason: null, ignored_repositories: names, ignored_scan_unavailable: null,
    deletion_gate: {
      blocked: null, warnings: [], pane_count: 0, can_delete_branch: true, branch_loss: null,
      discard: { changed_files: null, nested: false, status_unknown: false, ignored_repositories: names },
    },
  };
}
function catalog(worktree: WorktreeRow): SnapshotRest {
  const checkout: Checkout = { agent_scope: emptyScope(),
    id: "checkout:task", workspace_id: "project:example", label: "task", path: worktree.path,
    branch: worktree.branch, purpose: null, is_worktree: true, exists: true, has_panes: false,
    pull_request: null, active_tab_id: null, strip: [], next_tab_label: "1", tabs: [], worktree,
  };
  return legacyRest({
    navigator: { workspaces: [{ agent_scope: emptyScope(),
      id: "project:example", label: "Example", path: "/projects/example", device_id: "local",
      registered: true, temporary: false, pinned: false, checkouts: [checkout],
      inactive_checkouts: { expanded: false, checkout_ids: [] },
    }] },
  }, []);
}

describe("worktree Discard confirmation", () => {
  let container: HTMLDivElement;
  let root: Root;
  let shell: ReturnType<typeof useShellStore.getState>;
  let ui: ReturnType<typeof useUiStore.getState>;
  let events: Parameters<DispatchFn>[0][];

  beforeEach(() => {
    shell = useShellStore.getState();
    ui = useUiStore.getState();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.stubGlobal("ResizeObserver", class BrowserResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
    });
    events = [];
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    useShellStore.setState({ connection: "live", rest: catalog(row()) });
    useUiStore.getState().setWorkspaceDialog({ kind: "delete_worktree", workspaceId: "project:example", checkoutId: "checkout:task" });
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    vi.unstubAllGlobals();
  });
  const discard = () => document.querySelector<HTMLButtonElement>("[data-delete-discard]")!;
  const confirm = () => document.querySelector<HTMLButtonElement>("[data-delete-confirm]");

  it.each(["repository list", "lock", "unavailable scan"])("requires a new choice after %s changes and returns to its previous value", async (change) => {
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => root.render(<WorkspaceDialogs actions={actions} />));
    expect(confirm()?.disabled).toBe(true);
    await act(async () => discard().click());
    expect(discard().getAttribute("aria-checked")).toBe("true");
    expect(confirm()?.disabled).toBe(false);

    const changed = row();
    if (change === "repository list") {
      changed.ignored_repositories = [...names, "target/late"];
      changed.deletion_gate.discard = { changed_files: null, nested: false, status_unknown: false, ignored_repositories: changed.ignored_repositories };
    } else {
      if (change === "lock") changed.lock_reason = "Review in progress";
      else changed.ignored_scan_unavailable = "Check repository metadata and refresh";
      changed.deletion_gate.blocked = change === "lock" ? { code: "locked", name: "feature/delete", reason: "Review in progress" } : { code: "ignored_scan_unavailable" };
    }
    await act(async () => useShellStore.setState({ rest: catalog(changed) }));
    expect(confirm() === null || confirm()?.disabled).toBe(true);
    await act(async () => useShellStore.setState({ rest: catalog(row()) }));

    // The original measured names have returned, but the user has not
    // accepted them again. A disabled click cannot send removal.
    expect(discard().getAttribute("aria-checked")).toBe("false");
    expect(confirm()?.disabled).toBe(true);
    await act(async () => confirm()?.click());
    expect(events.filter((event) => event.kind === "remove_worktree")).toHaveLength(0);
    await act(async () => discard().click());
    await act(async () => confirm()?.click());
    expect(events.filter((event) => event.kind === "remove_worktree")).toEqual([
      expect.objectContaining({ payload: expect.objectContaining({ discard_changes: true, expected_ignored_repositories: names }) }),
    ]);
  });
  it("keeps the completed result after a checkout with panes leaves the catalog", async () => {
    const before = catalog(row());
    before.navigator!.workspaces![0]!.checkouts[0]!.tabs = [{ id: "tab", panes: [{ id: "root", title: "Shell" }] }] as unknown as Checkout["tabs"];
    useShellStore.setState({ rest: legacyRest(before, []) });
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => root.render(<WorkspaceDialogs actions={actions} />));
    await act(async () => discard().click());
    await act(async () => confirm()?.click());
    const after = catalog(row());
    after.navigator!.workspaces![0]!.checkouts = [];
    after.worktree_removal = { id: 1, device_id: "local", repository_root: "/projects/example", checkout_path: row().path, branch: "feature/delete", delete_branch: false, phase: "finished", notice: { code: "removed", branch: "kept" } };
    await act(async () => useShellStore.setState({ rest: legacyRest(after, []) }));
    expect(document.querySelector('[data-delete-result="finished"]')?.textContent).toBe("Worktree removed. Its local branch was kept.");
    expect(document.querySelector('[data-delete-confirm]')).toBeNull();
  });

  it("keeps a removed project's result after its nonempty target disappears", async () => {
    const before = catalog(row());
    before.navigator!.workspaces![0]!.registered = false;
    before.navigator!.workspaces![0]!.checkouts[0]!.tabs = [{ id: "tab", panes: [{ id: "root", title: "Shell" }] }] as unknown as Checkout["tabs"];
    useShellStore.setState({ rest: legacyRest(before, []) });
    useUiStore.getState().setWorkspaceDialog({ kind: "remove_project", workspaceId: "project:example" });
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => root.render(<WorkspaceDialogs actions={actions} />));
    await act(async () => document.querySelector<HTMLButtonElement>('[data-remove-confirm]')!.click());
    const after = catalog(row()); after.navigator!.workspaces = [];
    await act(async () => useShellStore.setState({ rest: legacyRest(after, []) }));
    expect(document.querySelector('[data-remove-result="finished"]')).not.toBeNull();
    const sent = events.length;
    await act(async () => document.querySelector<HTMLButtonElement>('[data-remove-cancel]')!.click());
    expect(events).toHaveLength(sent);
  });

});
