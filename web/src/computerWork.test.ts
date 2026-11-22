import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { handedFailure, SENT_CAP, startWork, takeComputerWork } from "./computerWork";
import { TAKE_WITHIN_MS, type RemoteWork, type RemoteWorkResult } from "./remoteView";
import type { SnapshotRest, Workspace } from "./snapshot";
import { START_ANSWER_TIMEOUT_MS } from "./startAnswer";
import type { StartTarget } from "./startTargets";

const APP = "/home/me/app";

function project(path: string, isGit = true): Workspace {
  return { id: `w:${path}`, label: path.split("/").pop(), path, is_git: isGit, checkouts: [{ id: `c:${path}`, path, exists: true, is_primary: true }] } as unknown as Workspace;
}

type Extra = { last_error?: unknown; task_operation?: unknown; project_adds?: unknown };
function snapshot(workspaces: Workspace[], extra: Extra = {}): SnapshotRest {
  return {
    navigator: { workspaces, devices: [{ id: "host", kind: "local" }] },
    status: { last_error: extra.last_error ?? null, project_adds: extra.project_adds ?? [] },
    task_operation: extra.task_operation ?? null,
  } as unknown as SnapshotRest;
}

const ready = (request: string, kind = "agent_start", agent = "started", id = 7) => ({ task_operation: { id, request_id: request, kind, phase: "ready", pane_id: "w1:p2", agent_phase: agent, agent_reason: agent === "failed" || agent === "unknown" ? "exited" : null, agent_line: null } });

/** The other computer's page: its bridge, its store and its actions, each a fake that records. */
function page(initial: SnapshotRest | null) {
  let deliver: ((work: RemoteWork) => void) | null = null;
  let rest = initial;
  let live = true;
  const listeners = new Set<() => void>();
  const actions = { openListedAgent: vi.fn(), startAgent: vi.fn(), createWorktree: vi.fn(), createWorkspace: vi.fn() };
  const results: RemoteWorkResult[] = [];
  const note = vi.fn();
  const stop = takeComputerWork(
    { onWork: (listener) => (deliver = listener), workResult: (result) => results.push(result) },
    {
      actions: actions as never,
      rest: () => rest,
      subscribe: (listener) => {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
      connected: () => live,
      note,
    },
  );
  return {
    actions,
    results,
    note,
    stop,
    ask: (work: RemoteWork) => deliver!(work),
    snapshot: (next: SnapshotRest) => {
      rest = next;
      for (const listener of [...listeners]) listener();
    },
    listening: () => listeners.size,
    /** The page's connection to its core drops, then comes back with `next`. */
    drop: () => {
      live = false;
      for (const listener of [...listeners]) listener();
    },
    back: (next: SnapshotRest) => {
      live = true;
      rest = next;
      for (const listener of [...listeners]) listener();
    },
  };
}

const START: Extract<RemoteWork, { op: "start" }> = { op: "start", request: "r-1", place: { checkout: APP }, provider: "claude", model: null, prompt: "fix the build", profile: null };

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

// PRD device-connect-iroh D-73, D-78: the page of another computer does what
// that computer's own window's operator asked of it, as its own work, and
// says how each start ended.
describe("work asked of another computer's screen", () => {
  it("starts in a checkout it lists and says it started once its core answers", () => {
    const view = page(snapshot([project(APP)]));
    view.ask(START);
    expect(view.actions.startAgent).toHaveBeenCalledWith({ target: { checkoutPath: APP }, provider: "claude", requestId: "r-1", model: null, prompt: "fix the build", profile: null });
    expect(view.actions.createWorkspace).not.toHaveBeenCalled();
    expect(view.results).toEqual([]);
    view.snapshot(snapshot([project(APP)], ready("r-1")));
    expect(view.results).toEqual([{ request: "r-1", outcome: "started" }]);
    expect(view.listening()).toBe(0);
  });

  it("makes the new worktree from the repository it lists, answered as a worktree", () => {
    const view = page(snapshot([project(APP)]));
    view.ask({ ...START, place: { repository: APP, branch: "fix", base: "main" } });
    expect(view.actions.createWorktree).toHaveBeenCalledWith({ requestId: "r-1", deviceId: "host", repositoryRoot: APP, branch: "fix", baseBranch: "main", agentKind: "claude", purpose: null, model: null, prompt: "fix the build", profile: null });
    view.snapshot(snapshot([project(APP)], ready("r-1")));
    expect(view.results).toEqual([]);
    view.snapshot(snapshot([project(APP)], ready("r-1", "worktree_create")));
    expect(view.results).toEqual([{ request: "r-1", outcome: "started" }]);
  });

  it("passes on its core's refusal, failure and an agent that did not start", () => {
    const refused = page(snapshot([project(APP)]));
    refused.ask(START);
    refused.snapshot(snapshot([project(APP)], { last_error: { request_id: "r-1", kind: "start_refused" } }));
    expect(refused.results).toEqual([{ request: "r-1", outcome: "refused", kind: "start_refused" }]);

    const failed = page(snapshot([project(APP)]));
    failed.ask(START);
    failed.snapshot(snapshot([project(APP)], { task_operation: { id: 7, request_id: "r-1", kind: "agent_start", phase: "failed", failure: "spawn_failed" } }));
    expect(failed.results).toEqual([{ request: "r-1", outcome: "failed", code: "spawn_failed" }]);

    const agent = page(snapshot([project(APP)]));
    agent.ask(START);
    agent.snapshot(snapshot([project(APP)], ready("r-1", "agent_start", "failed")));
    expect(agent.results).toEqual([{ request: "r-1", outcome: "agent_failed", reason: "exited", line: null }]);
  });

  it("says its core never answered once the start's bound passed", () => {
    const view = page(snapshot([project(APP)]));
    view.ask(START);
    vi.advanceTimersByTime(START_ANSWER_TIMEOUT_MS);
    expect(view.results).toEqual([{ request: "r-1", outcome: "no_answer" }]);
    expect(view.listening()).toBe(0);
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

  it("starts nothing when its first snapshot, project listed, came after the bound", () => {
    const view = page(null);
    view.ask(START);
    vi.setSystemTime(Date.now() + TAKE_WITHIN_MS + 1);
    view.snapshot(snapshot([project(APP)]));
    expect(view.actions.startAgent).not.toHaveBeenCalled();
    expect(view.results).toEqual([{ request: "r-1", outcome: "late" }]);
  });

  it("ends the wait for a first snapshot that never comes", () => {
    const view = page(null);
    view.ask(START);
    vi.advanceTimersByTime(TAKE_WITHIN_MS);
    expect(view.results).toEqual([{ request: "r-1", outcome: "late" }]);
    expect(view.listening()).toBe(0);
  });

  it("adds a folder that is no project there, then starts once the project is listed", () => {
    const view = page(snapshot([]));
    view.ask(START);
    expect(view.actions.createWorkspace).toHaveBeenCalledWith(APP, "app", "host", false, "r-1-add");
    expect(view.actions.startAgent).not.toHaveBeenCalled();
    // A snapshot that does not list it yet adds nothing again.
    view.snapshot(snapshot([]));
    expect(view.actions.createWorkspace).toHaveBeenCalledTimes(1);
    view.snapshot(snapshot([project(APP)]));
    expect(view.actions.startAgent).toHaveBeenCalledTimes(1);
  });

  it("starts nothing when the added project is first listed after the bound", () => {
    const view = page(snapshot([]));
    view.ask(START);
    vi.setSystemTime(Date.now() + TAKE_WITHIN_MS + 1);
    view.snapshot(snapshot([project(APP)]));
    expect(view.actions.startAgent).not.toHaveBeenCalled();
    expect(view.results).toEqual([{ request: "r-1", outcome: "late" }]);
    expect(view.note).toHaveBeenCalledTimes(1);
  });

  it("says the folder was not added when its core refused the add", () => {
    const view = page(snapshot([]));
    view.ask(START);
    view.snapshot(snapshot([], { project_adds: [{ request_id: "r-1-add", device_id: "host", path: APP, state: "refused", kind: "workspace.create_refused", message: "no" }] }));
    expect(view.actions.startAgent).not.toHaveBeenCalled();
    expect(view.results).toEqual([{ request: "r-1", outcome: "not_added", kind: "workspace.create_refused" }]);
  });

  // The tab exists while its agent is still coming up: that is not the end (checker letter-43817).
  it("says the agent is starting, then how its start ended, with no bound of its own", () => {
    for (const end of ["failed", "unknown"]) {
      const view = page(snapshot([project(APP)]));
      view.ask(START);
      view.snapshot(snapshot([project(APP)], ready("r-1", "agent_start", "starting")));
      expect(view.results).toEqual([{ request: "r-1", outcome: "starting" }]);
      vi.advanceTimersByTime(10 * START_ANSWER_TIMEOUT_MS);
      view.snapshot(snapshot([project(APP)], ready("r-1", "agent_start", "starting")));
      expect(view.results).toHaveLength(1);
      view.snapshot(snapshot([project(APP)], ready("r-1", "agent_start", end)));
      expect(view.results).toEqual([{ request: "r-1", outcome: "starting" }, { request: "r-1", outcome: "agent_failed", reason: "exited", line: null }]);
      expect(view.listening()).toBe(0);
    }
    const worktree = page(snapshot([project(APP)]));
    worktree.ask({ ...START, place: { repository: APP, branch: "fix", base: null } });
    worktree.snapshot(snapshot([project(APP)], ready("r-1", "worktree_create", "starting")));
    worktree.snapshot(snapshot([project(APP)], ready("r-1", "worktree_create", "started")));
    expect(worktree.results.map((result) => result.outcome)).toEqual(["starting", "started"]);
  });

  it("reads a slot the core cleared after the agent answered as started", () => {
    const view = page(snapshot([project(APP)]));
    view.ask(START);
    view.snapshot(snapshot([project(APP)], ready("r-1", "agent_start", "starting")));
    view.snapshot(snapshot([project(APP)]));
    expect(view.results.map((result) => result.outcome)).toEqual(["starting", "started"]);
  });

  // A start once made is followed to its own end whatever comes after it (checker letter-43817).
  it("still answers a made start after another work is shown", () => {
    const view = page(snapshot([project(APP)]));
    view.ask(START);
    expect(view.actions.startAgent).toHaveBeenCalledTimes(1);
    view.ask({ op: "show", pane: "w1:p9" });
    expect(view.actions.openListedAgent).toHaveBeenCalledWith("w1:p9");
    expect(view.results).toEqual([]);
    view.snapshot(snapshot([project(APP)], ready("r-1")));
    expect(view.results).toEqual([{ request: "r-1", outcome: "started" }]);
  });

  it("answers each made start on its own when a second comes while the first is starting", () => {
    const view = page(snapshot([project(APP)]));
    view.ask(START);
    view.snapshot(snapshot([project(APP)], ready("r-1", "agent_start", "starting")));
    view.ask({ ...START, request: "r-2" });
    expect(view.actions.startAgent).toHaveBeenCalledTimes(2);
    view.snapshot(snapshot([project(APP)], { ...ready("r-1", "agent_start", "starting"), last_error: { request_id: "r-2", kind: "task_operation_busy" } }));
    view.snapshot(snapshot([project(APP)], ready("r-1", "agent_start", "started")));
    expect(view.results).toEqual([
      { request: "r-1", outcome: "starting" },
      { request: "r-2", outcome: "refused", kind: "task_operation_busy" },
      { request: "r-1", outcome: "started" },
    ]);
  });

  it("makes a start asked again once, and none past the starts it follows", () => {
    const view = page(snapshot([project(APP)]));
    view.ask(START);
    view.ask(START);
    expect(view.actions.startAgent).toHaveBeenCalledTimes(1);
    for (let n = 2; n <= SENT_CAP + 1; n += 1) view.ask({ ...START, request: `r-${n}` });
    expect(view.actions.startAgent).toHaveBeenCalledTimes(SENT_CAP);
    expect(view.results).toEqual([{ request: `r-${SENT_CAP + 1}`, outcome: "busy" }]);
  });

  // A page that lost its core can no longer read how a made start ended (checker letter-43821).
  it("says a made start can no longer be confirmed once its core is lost, and makes nothing meanwhile", () => {
    const view = page(snapshot([project(APP)]));
    view.ask(START);
    view.snapshot(snapshot([project(APP)], ready("r-1", "agent_start", "starting")));
    view.drop();
    expect(view.results).toEqual([{ request: "r-1", outcome: "starting" }, { request: "r-1", outcome: "unconfirmed" }]);
    expect(view.listening()).toBe(0);

    const waiting = page(snapshot([project(APP)]));
    waiting.drop();
    waiting.ask({ ...START, request: "r-2" });
    expect(waiting.actions.startAgent).not.toHaveBeenCalled();
    waiting.back(snapshot([project(APP)]));
    expect(waiting.actions.startAgent).toHaveBeenCalledTimes(1);
  });

  it("keeps only the latest ask, answering a start it replaced, and none after it stops", () => {
    const view = page(null);
    view.ask(START);
    view.ask({ op: "show", pane: "w1:p2" });
    expect(view.results).toEqual([{ request: "r-1", outcome: "late" }]);
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

  it("words why it did not go, naming the computer where its core never had it", () => {
    expect(handedFailure({ request: "r", outcome: "started" }, "mini")).toBeNull();
    expect(handedFailure({ request: "r", outcome: "not_shown" }, "mini")).toEqual({ key: "startPanel.handOff.notShown", values: { device: "mini" } });
    expect(handedFailure({ request: "r", outcome: "late" }, "mini")).toEqual({ key: "startPanel.handOff.late", values: { device: "mini" } });
    expect(handedFailure({ request: "r", outcome: "not_added", kind: "workspace.create_refused" }, "mini")).toEqual({ key: "startPanel.handOff.notAdded", values: { device: "mini" }, code: "workspace.create_refused" });
    expect(handedFailure({ request: "r", outcome: "no_answer" }, "mini")).toEqual({ key: "shell.startTimeout" });
    expect(handedFailure({ request: "r", outcome: "agent_failed", reason: null, line: "boom" }, "mini")).toMatchObject({ line: "boom" });
  });
});
