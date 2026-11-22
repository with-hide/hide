import { describe, expect, it, vi } from "vitest";
import { ADD_WITHIN_MS, startWork, takeComputerWork } from "./computerWork";
import type { RemoteWork } from "./remoteView";
import type { SnapshotRest, Workspace } from "./snapshot";
import type { StartTarget } from "./startTargets";

const APP = "/home/me/app";

function project(path: string, isGit = true): Workspace {
  return { id: `w:${path}`, label: path.split("/").pop(), path, is_git: isGit, checkouts: [{ id: `c:${path}`, path, exists: true, is_primary: true }] } as unknown as Workspace;
}

function snapshot(workspaces: Workspace[]): SnapshotRest {
  return { navigator: { workspaces, devices: [{ id: "host", kind: "local" }] } } as unknown as SnapshotRest;
}

/** The other computer's page: its bridge, its store and its actions, each a fake that records. */
function page(initial: SnapshotRest | null) {
  let deliver: ((work: RemoteWork) => void) | null = null;
  let rest = initial;
  const listeners = new Set<() => void>();
  let clock = 0;
  const actions = { openListedAgent: vi.fn(), startAgent: vi.fn(), createWorktree: vi.fn(), createWorkspace: vi.fn() };
  const note = vi.fn();
  const stop = takeComputerWork(
    { onWork: (listener) => (deliver = listener) },
    {
      actions: actions as never,
      rest: () => rest,
      subscribe: (listener) => {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
      note,
      now: () => clock,
    },
  );
  return {
    actions,
    note,
    stop,
    ask: (work: RemoteWork) => deliver!(work),
    snapshot: (next: SnapshotRest) => {
      rest = next;
      for (const listener of [...listeners]) listener();
    },
    wait: (ms: number) => (clock += ms),
    listening: () => listeners.size,
  };
}

const START: RemoteWork = { op: "start", request: "r-1", place: { checkout: APP }, provider: "claude", model: null, prompt: "fix the build", profile: null };

// PRD device-connect-iroh D-73, D-78: the page of another computer does what
// that computer's own window's operator asked of it, as its own work.
describe("work asked of another computer's screen", () => {
  it("starts in a checkout it lists, with the start's own request", () => {
    const view = page(snapshot([project(APP)]));
    view.ask(START);
    expect(view.actions.startAgent).toHaveBeenCalledWith({ target: { checkoutPath: APP }, provider: "claude", requestId: "r-1", model: null, prompt: "fix the build", profile: null });
    expect(view.actions.createWorkspace).not.toHaveBeenCalled();
    expect(view.listening()).toBe(0);
  });

  it("makes the new worktree from the repository it lists", () => {
    const view = page(snapshot([project(APP)]));
    view.ask({ ...START, place: { repository: APP, branch: "fix", base: "main" } });
    expect(view.actions.createWorktree).toHaveBeenCalledWith({ requestId: "r-1", deviceId: "host", repositoryRoot: APP, branch: "fix", baseBranch: "main", agentKind: "claude", purpose: null, model: null, prompt: "fix the build", profile: null });
  });

  it("waits for its first snapshot before it shows or starts anything", () => {
    const view = page(null);
    view.ask({ op: "show", pane: "w1:p2" });
    expect(view.actions.openListedAgent).not.toHaveBeenCalled();
    view.snapshot(snapshot([]));
    expect(view.actions.openListedAgent).toHaveBeenCalledWith("w1:p2");
    view.snapshot(snapshot([]));
    expect(view.actions.openListedAgent).toHaveBeenCalledTimes(1);
  });

  it("adds a folder that is no project there, then starts once the project is listed", () => {
    const view = page(snapshot([]));
    view.ask(START);
    expect(view.actions.createWorkspace).toHaveBeenCalledWith(APP, "app");
    expect(view.actions.startAgent).not.toHaveBeenCalled();
    // A snapshot that does not list it yet adds nothing again.
    view.snapshot(snapshot([]));
    expect(view.actions.createWorkspace).toHaveBeenCalledTimes(1);
    view.snapshot(snapshot([project(APP)]));
    expect(view.actions.startAgent).toHaveBeenCalledTimes(1);
  });

  it("starts nothing once the add was not listed within its bound, and says so in the log", () => {
    const view = page(snapshot([]));
    view.ask(START);
    view.wait(ADD_WITHIN_MS + 1);
    view.snapshot(snapshot([]));
    view.snapshot(snapshot([project(APP)]));
    expect(view.actions.startAgent).not.toHaveBeenCalled();
    expect(view.note).toHaveBeenCalledTimes(1);
    expect(view.listening()).toBe(0);
  });

  it("keeps only the latest ask, and none after it stops", () => {
    const view = page(null);
    view.ask(START);
    view.ask({ op: "show", pane: "w1:p2" });
    view.snapshot(snapshot([project(APP)]));
    expect(view.actions.startAgent).not.toHaveBeenCalled();
    expect(view.actions.openListedAgent).toHaveBeenCalledWith("w1:p2");
    view.stop();
    view.ask({ op: "show", pane: "w1:p3" });
    expect(view.actions.openListedAgent).toHaveBeenCalledTimes(1);
  });
});

describe("a start the panel hands to another computer", () => {
  const start = { request: "r-1", provider: "codex", model: "o4", prompt: null, profile: "work", branch: "fix", base: null };

  it("names the checkout, or the repository and branch of a new worktree", () => {
    const checkout = { kind: "checkout", key: "k", deviceId: "win", label: "app", disabled: null, checkoutPath: APP, workspaceId: "w", checkoutId: "c", branch: "main", primary: true } satisfies StartTarget;
    expect(startWork(checkout, start)).toEqual({ op: "start", request: "r-1", place: { checkout: APP }, provider: "codex", model: "o4", prompt: null, profile: "work" });
    const worktree = { kind: "worktree", key: "k", deviceId: "win", label: "app", disabled: null, repositoryRoot: APP, workspaceId: "w" } satisfies StartTarget;
    expect(startWork(worktree, start).place).toEqual({ repository: APP, branch: "fix", base: null });
  });
});
