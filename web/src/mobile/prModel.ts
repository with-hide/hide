// The phone's PR tab rules (PRD mobile-companion-ui B37-B45, D-12, D-13): the
// four groups the open pull requests stand in, what the tab reads while GitHub
// does not answer, which buttons a page offers and why Merge is off. The core
// decided each row's reason (`hide-core/src/agent_state/work/board.rs`) and
// hided only trimmed the desktop's list and cards; the groups here only sort
// that reason into the phone's four, and the merge rules are the desktop
// card's own (`prCardModel.ts`), called and never copied. Pure, so every rule
// is tested without a socket or a browser.

import type { PrReasonKind } from "../agentScope";
import { confirmHolds, defaultMethod, mergeWhy, primaryAction, type Confirmed, type MergeWhy } from "../prCardModel";
import { readingOfStatuses, type PrReading } from "../prWork";
import type { PrMergeMethod } from "../snapshot";
import type { PhonePr, PhonePrProject } from "./protocol";

/** The phone's groups, in the order the tab draws them (F9). */
export type PrGroup = "ready" | "running" | "failed" | "review";

export const PR_GROUPS: readonly PrGroup[] = ["ready", "running", "failed", "review"];

/**
 * Where a reason stands on the phone. Merge is what can be merged now; CI, an agent's work and the merge queue are
 * things still running; a failed check, a change request, a conflict and a branch behind its base are the ones that
 * need a fix before they merge; the rest wait on a person: a reviewer, an author, an answer, GitHub.
 */
export function prGroupOf(kind: PrReasonKind): PrGroup {
  switch (kind) {
    case "merge":
      return "ready";
    case "ci":
    case "working":
    case "queued":
      return "running";
    case "failed":
    case "changes":
    case "conflict":
    case "update":
      return "failed";
    case "waiting_review":
    case "assign_reviewer":
    case "assign_reviewer_optional":
    case "review":
    case "team_review":
    case "reply":
    case "check":
    case "unjudged":
    case "checking":
    case "draft":
    case "merged":
      return "review";
    default:
      throw new Error(`Core PR reason the phone does not group: ${String(kind)}`);
  }
}

/** A pull request GitHub has settled: it is no longer in the open list and takes no merge. */
export function settled(pr: Pick<PhonePr, "state">): boolean {
  return pr.state === "merged" || pr.state === "closed";
}

export type PrEntry = { pr: PhonePr; project: PhonePrProject; group: PrGroup };

export type PrGroupView = { group: PrGroup; entries: PrEntry[] };

/** The open pull requests in the phone's groups, each in the order the core listed them; an empty group is left out. */
export function groupPrs(items: readonly PhonePr[], projects: readonly PhonePrProject[]): PrGroupView[] {
  const byGroup = new Map<PrGroup, PrEntry[]>();
  for (const pr of items) {
    if (settled(pr)) continue;
    const project = projects.find((candidate) => candidate.id === pr.project);
    if (!project) throw new Error(`Core PR item without its project: ${pr.key}`);
    const group = prGroupOf(pr.reason.kind);
    byGroup.set(group, [...(byGroup.get(group) ?? []), { pr, project, group }]);
  }
  return PR_GROUPS.flatMap((group) => {
    const entries = byGroup.get(group);
    return entries ? [{ group, entries }] : [];
  });
}

/** How many open pull requests the tab lists. */
export function openCount(groups: readonly PrGroupView[]): number {
  return groups.reduce((sum, view) => sum + view.entries.length, 0);
}

/** How GitHub reads stand over the projects listed (B38): the desktop list's own reading rule. */
export function prsReading(projects: readonly PhonePrProject[], now: number): PrReading {
  return readingOfStatuses(projects.flatMap((project) => project.github), now);
}

/** The pull request's page, from the current list; null once it left the list. */
export function prOf(items: readonly PhonePr[] | null, key: string | null): PhonePr | null {
  return items?.find((pr) => pr.key === key) ?? null;
}

/** Whether this pull request's projects name their device: a project away from the paired Mac does. */
export function projectTag(project: PhonePrProject): string {
  return project.device_label ? `${project.device_label} · ${project.label}` : project.label;
}

/**
 * What a page's buttons do for this pull request (B39-B44): whether Merge is shown (the desktop card shows it on the
 * operator's own pull requests that are not a review), why it is off, the method it will use, whether the repository has a
 * merge queue, and whether a merge is already in. Hand over is for any open pull request.
 */
export type PrActions = {
  open: boolean;
  /** The operator wrote it and it is not waiting on their review: the desktop card's own condition for a Merge button. */
  mergeShown: boolean;
  why: MergeWhy | null;
  head: string | null;
  method: PrMergeMethod | null;
  queue: boolean;
  /** In the merge queue already, or its write is going out. */
  working: boolean;
  /** GitHub refused a write that would only be refused again until the card is read once more (desktop B37). */
  repeats: boolean;
};

export function prActions(pr: PhonePr): PrActions {
  const card = pr.card;
  const open = !settled(pr);
  const mine = pr.work?.mine === true;
  const kind = pr.reason.kind;
  // An agent working on the pull request blocks Merge as it does on the desktop (`PrCard`'s `working`).
  const agentWorking = pr.group === "working";
  const write = card?.write ?? null;
  return {
    open,
    mergeShown: open && mine && primaryAction(kind) !== "review",
    why: mergeWhy(pr, card, agentWorking),
    head: card?.head ?? null,
    method: defaultMethod(card),
    queue: card?.merge?.queue === true,
    working: write?.phase === "working" || pr.work?.queue.state === "at",
    repeats: write?.phase === "failed" && write.reason !== "unreachable" && write.reason !== "failed",
  };
}

/** What Merge asks the operator to confirm: the desktop card's `Confirmed`, for the pull request named by `key`. */
export type MergeConfirm = Confirmed & { key: string };

/** Merge is asked only where the page offers it and nothing blocks, writes or repeats it. */
function mergeBlocked(view: PrActions): boolean {
  return !view.mergeShown || view.why !== null || view.working || view.repeats;
}

/** Opens a confirmation for what the page shows now, or null while Merge cannot be asked. */
export function confirmOf(pr: PhonePr): MergeConfirm | null {
  const view = prActions(pr);
  if (mergeBlocked(view) || view.head === null) return null;
  return { key: pr.key, action: "merge", head: view.head, method: view.method, queue: view.queue };
}

/**
 * Whether an open confirmation still names what the page shows (the desktop's own `confirmHolds`): the same head, queue and
 * method, and Merge still allowed. Else it closes and nothing is sent, so a press on the confirm never writes what the operator
 * was not shown.
 */
export function holds(confirm: MergeConfirm, pr: PhonePr | null): boolean {
  if (!pr || pr.key !== confirm.key) return false;
  const view = prActions(pr);
  return confirmHolds(confirm, { head: view.head, queue: view.queue, method: view.method, mergeBlocked: mergeBlocked(view), behind: pr.work?.merge_state === "behind" });
}
