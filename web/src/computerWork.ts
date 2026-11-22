// Work on another computer from this window (PRD device-connect-iroh D-59,
// D-73, D-78, D-81): a start on a paired computer is that computer's own
// work, so this window hands the start to that computer's own screen, whose
// page makes it on its own core and moves to it; a row of that computer's
// work opens there the same way. Both halves are here: what this window
// hands (`startWork`), and what the other computer's page does with it
// (`takeComputerWork`), which says how each start ended so the window that
// handed it keeps the operator's words until it went.

import type { Actions } from "./actions";
import type { AgentKind } from "./agentPicker";
import { HAND_WITHIN_MS, TAKE_WITHIN_MS, type RemoteBridge, type RemotePlace, type RemoteWork, type RemoteWorkResult } from "./remoteView";
import type { SnapshotRest, Workspace } from "./snapshot";
import { agentFailure, startFailure, startRefusal, type StartFailure } from "./startDraft";
import { localDeviceId } from "./snapshot";
import { START_ANSWER_TIMEOUT_MS, startAnswer } from "./startAnswer";
import type { StartTarget } from "./startTargets";

/**
 * How long the window that handed a start waits for how it ended: longer
 * than the desktop's hold, the page's wait and the core's answer together,
 * so a start that still goes is never one the window already gave back.
 */
export const HANDED_ANSWER_MS = HAND_WITHIN_MS + TAKE_WITHIN_MS + START_ANSWER_TIMEOUT_MS + 10_000;

type Start = Extract<RemoteWork, { op: "start" }>;

/** The start the panel would make, as the other computer's page makes it. */
export function startWork(target: StartTarget, start: { request: string; provider: string; model: string | null; prompt: string | null; profile: string | null; branch: string; base: string | null }): Start {
  const place: RemotePlace = target.kind === "worktree" ? { repository: target.repositoryRoot, branch: start.branch, base: start.base } : { checkout: target.checkoutPath };
  return { op: "start", request: start.request, place, provider: start.provider, model: start.model, prompt: start.prompt, profile: start.profile };
}

/**
 * Why a handed start did not go, worded for the start panel, or null when it
 * went. That computer's core's own answer reads as one of this computer's
 * would; what kept it from reaching that core names the computer.
 */
export function handedFailure(result: RemoteWorkResult, device: string): StartFailure | null {
  switch (result.outcome) {
    case "started":
      return null;
    case "not_shown":
      return { key: "startPanel.handOff.notShown", values: { device } };
    case "late":
      return { key: "startPanel.handOff.late", values: { device } };
    case "not_added":
      return { key: "startPanel.handOff.notAdded", values: { device }, code: result.kind };
    case "no_answer":
      return { key: "shell.startTimeout" };
    case "refused":
      return startRefusal(result.kind);
    case "failed":
      return startFailure(result.code);
    case "agent_failed":
      return agentFailure(result.reason, result.line);
  }
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

/** A start's own answer from this computer's core, as the result the window that handed it reads; null while there is none. */
function startResult(rest: SnapshotRest, work: Start): RemoteWorkResult | null {
  const answer = startAnswer(rest, work.request, "checkout" in work.place ? "agent_start" : "worktree_create");
  const request = work.request;
  if (answer.phase === "pending") return null;
  if (answer.phase === "refused") return { request, outcome: "refused", kind: answer.kind };
  if (answer.phase === "failed") return { request, outcome: "failed", code: answer.message };
  if (answer.agentPhase === "failed") return { request, outcome: "agent_failed", reason: answer.agentReason, line: answer.agentLine };
  return { request, outcome: "started" };
}

export type ComputerWorkDeps = {
  actions: Pick<Actions, "openListedAgent" | "startAgent" | "createWorktree" | "createWorkspace">;
  rest: () => SnapshotRest | null;
  /** Calls back on each snapshot; returns the unsubscribe. */
  subscribe: (listener: () => void) => () => void;
  note: (message: string) => void;
};

type Waiting =
  /** Before anything is made: the first snapshot, then the folder listed or added. */
  | { work: RemoteWork; until: number; phase: "before" }
  /** The folder is being added as a project under the request `add`. */
  | { work: Start; until: number; phase: "adding"; add: string }
  /** The start went to this computer's core, which answers it by its request id. */
  | { work: Start; until: number; phase: "starting" };

/**
 * The other computer's page: what its own window's operator asked of this
 * screen, done here as this computer's own work (D-59). An ask waits for the
 * page's first snapshot, and a start whose folder is no project here adds it
 * first and goes once the add is listed (D-73); both waits together end
 * `TAKE_WITHIN_MS` after the ask arrived, read before anything is made, so
 * nothing starts late even when the snapshot that would allow it arrives
 * late. A start's own answer then has `START_ANSWER_TIMEOUT_MS`. Every start
 * ends with one result to the window that handed it. One ask at a time, the
 * latest; a start it replaces is answered `late`. Returns the stop.
 */
export function takeComputerWork(bridge: Pick<RemoteBridge, "onWork" | "workResult">, deps: ComputerWorkDeps): () => void {
  let waiting: Waiting | null = null;
  let unsubscribe: (() => void) | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const settle = () => {
    waiting = null;
    unsubscribe?.();
    unsubscribe = null;
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };
  const arm = (until: number) => {
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(step, Math.max(0, until - Date.now()));
  };
  const end = (result: RemoteWorkResult | null) => {
    const work = waiting?.work;
    settle();
    if (!work || work.op !== "start" || !result) return;
    if (result.outcome !== "started") deps.note(`start from another computer ${result.outcome}: ${placeFolder(work.place)}`);
    bridge.workResult?.(result);
  };
  const step = () => {
    const current = waiting;
    if (!current) return;
    const rest = deps.rest();
    if (current.phase === "starting") {
      // The core's answer is a fact, read even past the bound; only nothing more is waited for then.
      const result = rest ? startResult(rest, current.work) : null;
      if (result) return end(result);
      if (Date.now() >= current.until) end({ request: current.work.request, outcome: "no_answer" });
      return;
    }
    // Nothing is made once the bound passed, whatever the snapshot now allows.
    if (Date.now() >= current.until) {
      if (current.work.op === "show") deps.note(`show from another computer: ${current.work.pane} came too late; nothing opened`);
      return end(current.work.op === "start" ? { request: current.work.request, outcome: "late" } : null);
    }
    if (!rest) return;
    const work = current.work;
    if (work.op === "show") {
      settle();
      deps.actions.openListedAgent(work.pane);
      return;
    }
    if (placeListed(rest.navigator?.workspaces ?? [], work.place)) {
      makeStart(deps.actions, rest, work);
      const until = Date.now() + START_ANSWER_TIMEOUT_MS;
      waiting = { work, until, phase: "starting" };
      arm(until);
      return;
    }
    if (current.phase === "adding") {
      const receipt = rest.status?.project_adds?.findLast((row) => row.request_id === current.add);
      if (receipt?.state === "refused") end({ request: work.request, outcome: "not_added", kind: receipt.kind });
      return;
    }
    // The folder is no project here: the start adds it (D-73), and goes once the core lists it.
    const add = `${work.request}-add`;
    waiting = { work, until: current.until, phase: "adding", add };
    const folder = placeFolder(work.place);
    deps.actions.createWorkspace(folder, folderName(folder), localDeviceId(rest), false, add);
  };
  let on = true;
  bridge.onWork?.((work) => {
    if (!on) return;
    end(waiting?.work.op === "start" ? { request: waiting.work.request, outcome: "late" } : null);
    const until = Date.now() + TAKE_WITHIN_MS;
    waiting = { work, until, phase: "before" };
    unsubscribe = deps.subscribe(step);
    arm(until);
    step();
  });
  return () => {
    on = false;
    settle();
  };
}
