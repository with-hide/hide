// Work on another computer from this window (PRD device-connect-iroh D-59,
// D-73, D-78, D-81): a start on a paired computer is that computer's own
// work, so this window hands the start to that computer's own screen, whose
// page makes it on its own core and moves to it; a row of that computer's
// work opens there the same way. Both halves are here: what this window
// hands (`startWork`), and what the other computer's page does with it
// (`takeComputerWork`).

import type { Actions } from "./actions";
import type { AgentKind } from "./agentPicker";
import type { RemoteBridge, RemotePlace, RemoteWork } from "./remoteView";
import type { SnapshotRest, Workspace } from "./snapshot";
import { localDeviceId } from "./snapshot";
import type { StartTarget } from "./startTargets";

/**
 * How long a start whose folder is no project there waits for the add to be
 * listed: an add is listed within a second or two, and one that never is
 * (refused, or the folder gone) must not start work later, unseen.
 */
export const ADD_WITHIN_MS = 30_000;

type Start = Extract<RemoteWork, { op: "start" }>;

/** The start the panel would make, as the other computer's page makes it. */
export function startWork(target: StartTarget, start: { request: string; provider: string; model: string | null; prompt: string | null; profile: string | null; branch: string; base: string | null }): Start {
  const place: RemotePlace = target.kind === "worktree" ? { repository: target.repositoryRoot, branch: start.branch, base: start.base } : { checkout: target.checkoutPath };
  return { op: "start", request: start.request, place, provider: start.provider, model: start.model, prompt: start.prompt, profile: start.profile };
}

function folderName(path: string): string {
  const names = path.split("/").filter(Boolean);
  return names[names.length - 1] ?? path;
}

/** The folder a place starts in, which has to be this computer's project for the start to go. */
function placeFolder(place: RemotePlace): string {
  return "checkout" in place ? place.checkout : place.repository;
}

/** Whether this computer lists `place` as one of its own projects: the checkout itself, or the repository a worktree is made from. */
export function placeListed(workspaces: readonly Workspace[], place: RemotePlace): boolean {
  if ("checkout" in place) return workspaces.some((workspace) => workspace.checkouts.some((checkout) => checkout.exists && checkout.path === place.checkout));
  return workspaces.some((workspace) => workspace.is_git && workspace.path === place.repository);
}

function makeStart(actions: Pick<Actions, "startAgent" | "createWorktree">, rest: SnapshotRest, work: Start): void {
  const common = { model: work.model, prompt: work.prompt, profile: work.profile };
  if ("checkout" in work.place) {
    actions.startAgent({ target: { checkoutPath: work.place.checkout }, provider: work.provider as AgentKind, requestId: work.request, ...common });
    return;
  }
  actions.createWorktree({ requestId: work.request, deviceId: localDeviceId(rest), repositoryRoot: work.place.repository, branch: work.place.branch, baseBranch: work.place.base, agentKind: work.provider, purpose: null, ...common });
}

export type ComputerWorkDeps = {
  actions: Pick<Actions, "openListedAgent" | "startAgent" | "createWorktree" | "createWorkspace">;
  rest: () => SnapshotRest | null;
  /** Calls back on each snapshot; returns the unsubscribe. */
  subscribe: (listener: () => void) => () => void;
  note: (message: string) => void;
  now?: () => number;
};

/**
 * The other computer's page: what its own window's operator asked of this
 * screen, done here as this computer's own work (D-59). Before the page
 * holds a snapshot the ask waits for it; a start whose folder is no project
 * here adds it first and starts once the add is listed (D-73), within
 * `ADD_WITHIN_MS`. One ask waits at a time, the latest. Returns the stop.
 */
export function takeComputerWork(bridge: Pick<RemoteBridge, "onWork">, deps: ComputerWorkDeps): () => void {
  const now = deps.now ?? Date.now;
  let waiting: { work: RemoteWork; at: number; added: boolean } | null = null;
  let unsubscribe: (() => void) | null = null;
  const settle = () => {
    waiting = null;
    unsubscribe?.();
    unsubscribe = null;
  };
  const step = () => {
    if (!waiting) return;
    const rest = deps.rest();
    if (!rest) return;
    const { work } = waiting;
    if (work.op === "show") {
      settle();
      deps.actions.openListedAgent(work.pane);
      return;
    }
    if (placeListed(rest.navigator?.workspaces ?? [], work.place)) {
      settle();
      makeStart(deps.actions, rest, work);
      return;
    }
    if (waiting.added) {
      if (now() - waiting.at <= ADD_WITHIN_MS) return;
      settle();
      deps.note(`start from another computer: ${placeFolder(work.place)} was not listed as a project in time; nothing started`);
      return;
    }
    // The folder is no project here: the start adds it (D-73), and goes once the core lists it.
    waiting.added = true;
    waiting.at = now();
    const folder = placeFolder(work.place);
    deps.actions.createWorkspace(folder, folderName(folder));
  };
  let on = true;
  bridge.onWork?.((work) => {
    if (!on) return;
    settle();
    waiting = { work, at: now(), added: false };
    unsubscribe = deps.subscribe(step);
    step();
  });
  return () => {
    on = false;
    settle();
  };
}
