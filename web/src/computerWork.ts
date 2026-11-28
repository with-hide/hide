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
import { guestPaneAddress } from "./guestWork";
import { remotePaneAddress } from "./remote";
import type { RemoteScreensBridge } from "./host";
import { HAND_WITHIN_MS, SHOW_ANSWER_MS, TAKE_WITHIN_MS, type RemoteBridge, type RemotePlace, type RemoteShowResult, type RemoteWork, type RemoteWorkResult } from "./remoteView";
import type { SnapshotRest, Workspace } from "./snapshot";
import { agentFailure, startFailure, startRefusal, type StartFailure } from "./startDraft";
import { localDeviceId } from "./snapshot";
import { agentAnswer, START_ANSWER_TIMEOUT_MS, startAnswer } from "./startAnswer";
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
export function handedFailure(result: Exclude<RemoteWorkResult, { outcome: "starting" } | RemoteShowResult>, device: string): StartFailure | null {
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
    case "unconfirmed":
      return { key: "startPanel.handOff.unconfirmed", values: { device } };
    case "busy":
      return { key: "startPanel.handOff.busy", values: { device } };
    case "refused":
      return startRefusal(result.kind);
    case "failed":
      return startFailure(result.code);
    case "agent_failed":
      return agentFailure(result.reason, result.line);
  }
}

/**
 * This computer's own work that runs on a paired machine (D-55): `paneId`,
 * a pane this core lists on a device that is a paired machine (`peer_key`),
 * as that machine names it, `pane`, on the device `deviceId`. Null for a pane
 * of this machine, of an SSH or WSL device, or one this core does not list.
 */
export function ownWorkThere(rest: SnapshotRest | null, paneId: string): { deviceId: string; pane: string } | null {
  const own = localDeviceId(rest);
  const address = remotePaneAddress(rest, paneId);
  if (!own || !address) return null;
  const paired = rest?.navigator?.devices?.some((device) => device.id === address.device && device.peer_key != null) ?? false;
  return paired ? { deviceId: address.device, pane: guestPaneAddress(own, address.devicePane) } : null;
}

/**
 * Follows a show handed with `request` (D-55): once that computer's page
 * answers `shown`, nothing more; any other answer, or none within
 * `SHOW_ANSWER_MS` (a page of an older build answers none), calls
 * `notShown` once with why. Returns the stop, which a later ask calls.
 */
export function followShow(remote: Required<Pick<RemoteScreensBridge, "onWorkResult">>, request: string, notShown: (why: string) => void): () => void {
  let done = false;
  const stop = () => {
    if (done) return;
    done = true;
    unsubscribe();
    clearTimeout(timer);
  };
  const end = (why: string) => {
    if (done) return;
    stop();
    notShown(why);
  };
  const unsubscribe = remote.onWorkResult((result) => {
    if (result.request !== request) return;
    if (result.outcome === "shown") stop();
    else end(result.outcome);
  });
  const timer = setTimeout(() => end("no_answer"), SHOW_ANSWER_MS);
  return stop;
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

const kindOf = (work: Start) => ("checkout" in work.place ? "agent_start" : "worktree_create");

export type ComputerWorkDeps = {
  actions: Pick<Actions, "openListedAgent" | "openGuestPane" | "startAgent" | "createWorktree" | "createWorkspace">;
  rest: () => SnapshotRest | null;
  /** Calls back on each snapshot; returns the unsubscribe. */
  subscribe: (listener: () => void) => () => void;
  /** Whether the page is connected to its core now; a start is made only then, and one made is read only then. */
  connected: () => boolean;
  /**
   * Whether the page's socket is open now, so what it makes is sent. A page
   * whose tile the desktop hid keeps its core as `live` with its socket
   * closed, and the click that shows the tile again hands the ask before the
   * socket reopens: the ask waits for the socket, else it would be dropped.
   */
  open: () => boolean;
  note: (message: string) => void;
};

/** The ask not made yet, the latest only: it waits for the first snapshot, then for its folder to be listed or added (`add`, the add's request). */
type Pending = { work: RemoteWork; until: number; add: string | null };

/** A start made on this computer's core: `until` bounds the core's answer; `taskId` is the task once its tab exists. */
type Sent = { work: Start; until: number; taskId: number | null };

/** How many made starts are followed at once; a start past it is not made and is answered `busy`. The core refuses a start while another is unanswered or its agent is still coming up, so a second is answered at once; this only guards. */
export const SENT_CAP = 8;

/**
 * The other computer's page: what its own window's operator asked of this
 * screen, done here as this computer's own work (D-59). An ask waits for the
 * page's first snapshot, and a start whose folder is no project here adds it
 * first and goes once the add is listed (D-73); both waits together end
 * `TAKE_WITHIN_MS` after the ask arrived, read before anything is made, so
 * nothing starts late even when the snapshot that would allow it arrives
 * late. Only the latest ask not made yet waits; one it replaces is answered
 * `late`. A start once made is followed to its own end, whatever is asked
 * after it: the core's answer within `START_ANSWER_TIMEOUT_MS`, then its
 * agent's (`agentAnswer`, told `starting` meanwhile), each reported once to
 * the window that handed it; once the page loses its core, every made start
 * is `unconfirmed`, never sent again. Returns the stop.
 */
export function takeComputerWork(bridge: Pick<RemoteBridge, "onWork" | "workResult">, deps: ComputerWorkDeps): () => void {
  let pending: Pending | null = null;
  const sent = new Map<string, Sent>();
  let unsubscribe: (() => void) | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let on = true;
  const report = (work: Start, result: RemoteWorkResult) => {
    if (result.outcome !== "started" && result.outcome !== "starting") deps.note(`start from another computer ${result.outcome}: ${placeFolder(work.place)}`);
    bridge.workResult?.(result);
  };
  /** Each made start's own end, read from this snapshot; with the core lost, none can be read any more. */
  const follow = (rest: SnapshotRest | null) => {
    if (!deps.connected()) {
      for (const [request, start] of sent) report(start.work, { request, outcome: "unconfirmed" });
      sent.clear();
      return;
    }
    if (!rest) return;
    for (const [request, start] of sent) {
      if (start.taskId === null) {
        const answer = startAnswer(rest, request, kindOf(start.work));
        if (answer.phase === "pending") {
          if (Date.now() >= start.until) {
            sent.delete(request);
            report(start.work, { request, outcome: "no_answer" });
          }
          continue;
        }
        if (answer.phase !== "ready") {
          sent.delete(request);
          report(start.work, answer.phase === "refused" ? { request, outcome: "refused", kind: answer.kind } : { request, outcome: "failed", code: answer.message });
          continue;
        }
        start.taskId = answer.taskId;
        if (answer.agentPhase === "starting") report(start.work, { request, outcome: "starting" });
      }
      const agent = agentAnswer(rest.task_operation, start.taskId);
      if (agent.phase === "starting") continue;
      sent.delete(request);
      report(start.work, agent.phase === "failed" ? { request, outcome: "agent_failed", reason: agent.reason, line: agent.line } : { request, outcome: "started" });
    }
  };
  /** The ask not made yet: nothing is made once its bound passed, whatever the snapshot now allows. */
  const take = (rest: SnapshotRest | null) => {
    const current = pending;
    if (!current) return;
    const work = current.work;
    if (Date.now() >= current.until) {
      pending = null;
      if (work.op === "start") report(work, { request: work.request, outcome: "late" });
      else {
        deps.note(`show from another computer: ${work.pane} came too late; nothing opened`);
        if (work.request !== undefined) bridge.workResult?.({ request: work.request, outcome: "late" });
      }
      return;
    }
    if (!rest || !deps.connected() || !deps.open()) return;
    if (work.op === "show") {
      pending = null;
      if (work.request === undefined) return deps.actions.openListedAgent(work.pane);
      // The asking computer's own work that runs here (D-55): its pane opens in the screen that reads and types into it, and the asking window hears whether it did.
      const listed = rest.navigator?.guest_panes?.some((guest) => guest.panes.some((pane) => pane.id === work.pane)) ?? false;
      if (listed) deps.actions.openGuestPane(work.pane);
      else deps.note(`show from another computer: ${work.pane} is no pane of its work here; nothing opened`);
      bridge.workResult?.({ request: work.request, outcome: listed ? "shown" : "not_listed" });
      return;
    }
    if (placeListed(rest.navigator?.workspaces ?? [], work.place)) {
      pending = null;
      if (sent.size >= SENT_CAP) {
        return report(work, { request: work.request, outcome: "busy" });
      }
      makeStart(deps.actions, rest, work);
      sent.set(work.request, { work, until: Date.now() + START_ANSWER_TIMEOUT_MS, taskId: null });
      return;
    }
    if (current.add !== null) {
      const receipt = rest.status?.project_adds?.findLast((row) => row.request_id === current.add);
      if (receipt?.state === "refused") {
        pending = null;
        report(work, { request: work.request, outcome: "not_added", kind: receipt.kind });
      }
      return;
    }
    // The folder is no project here: the start adds it (D-73), and goes once the core lists it.
    current.add = `${work.request}-add`;
    const folder = placeFolder(work.place);
    deps.actions.createWorkspace(folder, folderName(folder), localDeviceId(rest), false, current.add);
  };
  /** Listens and keeps one timer while anything is open: the nearest of the pending ask's bound and each unanswered start's. */
  const rearm = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
    const open = pending !== null || sent.size > 0;
    if (open && !unsubscribe) unsubscribe = deps.subscribe(step);
    if (!open && unsubscribe) {
      unsubscribe();
      unsubscribe = null;
    }
    const bounds = [...(pending ? [pending.until] : []), ...[...sent.values()].filter((start) => start.taskId === null).map((start) => start.until)];
    if (bounds.length > 0) timer = setTimeout(step, Math.max(0, Math.min(...bounds) - Date.now()));
  };
  function step() {
    if (!on) return;
    const rest = deps.rest();
    follow(rest);
    take(rest);
    rearm();
  }
  bridge.onWork?.((work) => {
    if (!on) return;
    // The same start asked again is the one already taken.
    if (work.op === "start" && (sent.has(work.request) || (pending?.work.op === "start" && pending.work.request === work.request))) return;
    if (pending?.work.op === "start") report(pending.work, { request: pending.work.request, outcome: "late" });
    else if (pending?.work.request !== undefined) bridge.workResult?.({ request: pending.work.request, outcome: "late" });
    pending = { work, until: Date.now() + TAKE_WITHIN_MS, add: null };
    step();
  });
  return () => {
    on = false;
    pending = null;
    sent.clear();
    rearm();
  };
}
