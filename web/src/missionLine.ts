// A mission node card's bottom line (PRD mission-destination D-51-D-55,
// B54-B60): the one computation the desktop's map card, its hover and the
// phone's card read, from fields both carry, so the two screens draw one line.
//
// A node that changes code (`change`, `external`) ends in its branch line,
// `[icon] #n <branch> → <destination>`: the icon is its pull request's state
// as the PR list draws it (`PR_LOOK`), a grey branch before one, faint while
// the node is only planned; the operator's merge is a person before the
// destination. A general node, which changes no code, ends in its result
// line: its preset's icon and name until it is done, then its result line or
// its verdict (PRD node-presets D-08). Every value comes from the core; a branch or destination it did
// not send is left out, never named (B60).

import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { PR_LOOK } from "./prMark";
import type { MissionDelivery, MissionExternal, MissionMergeStep, MissionNode, MissionNodeBrief, MissionNodePreset, MissionOutside, MissionVerdict, MissionWaitOutcome, MissionWaitTarget, PrState } from "./snapshot";

/** What the bottom line reads of a node: the shared rules' fields, its pull requests' base and count, an outside node's facts and, on the desktop, its branch (the phone's frame carries none, D-52). */
export type LineNode = Pick<MissionNodeBrief, "kind" | "preset" | "status" | "pr" | "pr_state" | "landing" | "merge"> & {
  delivery: MissionDelivery | null;
  base_pr: number | null;
  /** How the pull request it is stacked on stands, as the mission's own nodes know it; absent or null when no node holds it. */
  base_state?: PrState | null;
  /** The worker's branch; absent where the frame carries none. */
  branch?: string | null;
  external?: MissionExternal | null;
  /** The branch the current pull request merges into, as GitHub last answered; absent until it answered. */
  pr_base?: string | null;
  /** How many pull requests the node holds besides its current one (`+N`). */
  pr_more?: number;
  /** Where an outside node's work goes and how its pull request stands; absent from a core before it. */
  outside?: MissionOutside | null;
  /** A finished general node's result line and its verdict; absent where the frame carries none. */
  line?: string | null;
  verdict?: MissionVerdict | null;
  checks?: { passed: number; total: number } | null;
  /** A waiting node's target, its deadline, the progress of a mission it waits on, the code host's label of its pull request and how it failed (PRD core-wait D-15-D-17); absent from a core before waits. */
  wait_target?: MissionWaitTarget | null;
  wait_until_unix_ms?: number | null;
  wait_progress?: { done: number; total: number } | null;
  wait_label?: string | null;
  wait_outcome?: MissionWaitOutcome | null;
};

const KIND_WORD: Record<MissionNode["kind"], MessageKey> = {
  change: "mission.kind.change",
  general: "mission.kind.general",
  wait: "mission.kind.external",
};

/** A general node's name by its preset (PRD node-presets D-02); the core's preset table names the same words, which `missionKindWords.test.ts` checks. */
const PRESET_WORD: Record<MissionNodePreset, MessageKey> = {
  research: "mission.kind.research",
  verify: "mission.kind.verify",
  deploy: "mission.kind.deploy",
};

export function kindWord(kind: MissionNode["kind"]): MessageKey {
  return KIND_WORD[kind];
}

/** A node's kind as it is named: a general node by its preset, and by `일반` with none or with a preset this bundle does not know, which a later core may send (D-11). */
export function nodeKindWord(node: Pick<MissionNode, "kind" | "preset">): MessageKey {
  return node.kind === "general" && node.preset ? (PRESET_WORD[node.preset] ?? KIND_WORD.general) : KIND_WORD[node.kind];
}

/** The desktop's node as the line reads it: the base and count from its pull request list, the current one last, and the state of the pull request it is stacked on from whichever of the mission's nodes holds it. */
export function lineNode(node: MissionNode, nodes: readonly Pick<MissionNode, "prs">[] = []): LineNode {
  const current = node.prs.at(-1);
  const base = node.base_pr == null ? null : nodes.flatMap((one) => one.prs).find((pr) => pr.number === node.base_pr);
  return { ...node, pr_base: current?.base ?? null, pr_more: Math.max(node.prs.length - 1, 0), base_state: base?.state ?? null, wait_label: node.wait_reference?.label ?? null };
}

/** Whether a node's result goes into a branch, so it ends in a branch line (D-51): a change or an outside node. */
export function changesCode(node: Pick<MissionNodeBrief, "kind">): boolean {
  return node.kind === "change" || node.kind === "wait";
}

/**
 * The branch line's icon (D-52): a pull request's state, drawn by `PR_LOOK`;
 * `pull` a pull request whose state GitHub has not answered yet; `branch`
 * the grey branch before any; `plan_pr` and `plan_direct` the faint marks
 * of a node only planned.
 */
export type LineMark = PrState | "pull" | "branch" | "plan_pr" | "plan_direct";

/** Where the result lands: a branch (bold when it is not the mission's destination) or the pull request it is stacked on, drawn as that pull request's mark by its state (grey when unknown) and a grey number (D-57). */
export type LineTo = { kind: "branch"; name: string; elsewhere: boolean } | { kind: "stack"; pr: number; state: PrState | null };

export type BranchLine = {
  mark: LineMark;
  /** `#n`, `↗#n` for an outside one, and `+N` for the others the node holds. */
  /** `label` is the code host's own name for an outside pull request (`#12`, `!12`), the core's `wait_reference`. */
  pr: { number: number; outside: boolean; more: number; label?: string } | null;
  /** The branch, `↗` before an outside one; null when not known. */
  branch: { name: string; outside: boolean } | null;
  /** A planned node says how it will go in its branch's place: `PR` or `PR 없이`. */
  plan: "pr" | "direct" | null;
  /** A direct change ends the line in `PR 없이`. */
  direct: boolean;
  /** Null when the destination is not known: the line then stops before `→` (B60). */
  to: LineTo | null;
  /** The operator's merge, a person before the destination, by where it stands (D-53). */
  person: MissionMergeStep | null;
};

/**
 * A code node's branch line, or null for a node that changes no code.
 * `destination` is the mission's destination branch as the core sends it,
 * null when it could not be read.
 */
export function branchLine(node: LineNode, destination: string | null): BranchLine | null {
  if (!changesCode(node) || waitLine(node)) return null;
  return node.kind === "wait" ? outsideLine(node, destination) : ownLine(node, destination);
}

function ownLine(node: LineNode, destination: string | null): BranchLine {
  const byPr = node.delivery !== "direct" || node.pr != null;
  const person = node.merge?.by === "operator" ? node.merge.state : null;
  const to = node.base_pr != null ? { kind: "stack" as const, pr: node.base_pr, state: node.base_state ?? null } : branchTo(byPr ? (node.pr_base ?? destination) : destination, destination);
  // A node only planned says how it will go; one replanned after it opened a pull request keeps the pull request it has.
  if (node.status === "planned" && node.pr == null) {
    return { mark: byPr ? "plan_pr" : "plan_direct", pr: null, branch: null, plan: byPr ? "pr" : "direct", direct: false, to, person };
  }
  const branch = node.branch ? { name: node.branch, outside: false } : null;
  if (!byPr) return { mark: node.landing?.state === "landed" ? "merged" : "branch", pr: null, branch, plan: null, direct: true, to, person: null };
  const pr = node.pr != null ? { number: node.pr, outside: false, more: node.pr_more ?? 0 } : null;
  return { mark: pr ? (node.pr_state ?? "pull") : "branch", pr, branch, plan: null, direct: false, to, person };
}

/** An outside node (D-07, D-52): `↗#n → <its base>` lit by the state GitHub last answered, or `↗ <branch> → <the default branch>`; done is the result in, whatever else was read. */
function outsideLine(node: LineNode, destination: string | null): BranchLine {
  const done = node.status === "done";
  const to = branchTo(node.outside?.base ?? null, destination);
  const external = node.external;
  if (external && "pr" in external) {
    return { mark: done ? "merged" : (node.outside?.state ?? "pull"), pr: { number: external.pr, outside: true, more: 0, ...(node.wait_label ? { label: node.wait_label } : {}) }, branch: null, plan: null, direct: false, to, person: null };
  }
  return { mark: done ? "merged" : "branch", pr: null, branch: external ? { name: external.branch, outside: true } : null, plan: null, direct: false, to, person: null };
}

/**
 * A waiting node's line for a target that is not a pull request's merge or a
 * branch (PRD core-wait D-15-D-17): `↗#12 · CI 끝나면`, `mission-91 · 끝 2/7`
 * or `mission-91 · gitlab-write`, then how it failed; null for any other node,
 * whose branch line speaks. The pull request is named as its code host writes
 * it (`#12`, `!12`).
 */
export type WaitLine = { failed: Exclude<MissionWaitOutcome, "expired"> | null } & (
  | { kind: "ci"; pr: string; mark: LineMark }
  | { kind: "mission"; mission: string; progress: { done: number; total: number } | null }
  | { kind: "node"; mission: string; node: string }
);

export function waitLine(node: LineNode): WaitLine | null {
  const target = node.kind === "wait" ? node.wait_target : null;
  if (!target) return null;
  const failed = node.wait_outcome && node.wait_outcome !== "expired" ? node.wait_outcome : null;
  if ("ci" in target) {
    const mark: LineMark = node.status === "done" ? "merged" : (node.outside?.state ?? "pull");
    return { kind: "ci", pr: node.wait_label ?? `#${target.ci}`, mark, failed };
  }
  if ("mission" in target) return { kind: "mission", mission: target.mission, progress: node.wait_progress ?? null, failed };
  if ("node" in target) return { kind: "node", mission: target.node.mission, node: target.node.node, failed };
  return null;
}

/** The wait line in words, for the card's name and its hover. */
export function waitLineWords(line: WaitLine, t: TFunction<"translation">): string {
  const head = line.kind === "ci" ? [`↗${line.pr}`, t("mission.wait.ci")] : line.kind === "mission" ? [line.mission, line.progress ? t("mission.wait.progress", { done: String(line.progress.done), total: String(line.progress.total) }) : null] : [line.mission, line.node];
  return [...head, line.failed ? t(`mission.wait.failed.${line.failed}`) : null].filter(Boolean).join(" · ");
}

/** The day a waiting node's deadline shows from (D-15): under it the head says the hours left. */
const WAIT_LEFT_FROM_MS = 24 * 60 * 60 * 1000;

/** What a waiting node's head says after `기다림` (D-15, B8): `기한 지남` once its deadline passed, the hours left under a day, else nothing. */
export function waitLeft(node: Pick<LineNode, "kind" | "status" | "wait_until_unix_ms" | "wait_outcome">, now: number): { expired: true } | { hours: number } | null {
  if (node.kind !== "wait" || node.status === "done" || node.status === "dropped") return null;
  if (node.wait_outcome === "expired") return { expired: true };
  const until = node.wait_until_unix_ms;
  if (until == null || until <= now || until - now >= WAIT_LEFT_FROM_MS) return null;
  return { hours: Math.max(1, Math.ceil((until - now) / (60 * 60 * 1000))) };
}

/** A branch the result lands in, bold when it is not the destination; bold needs a destination to differ from, so with none read it is drawn plainly. */
function branchTo(branch: string | null, destination: string | null): LineTo | null {
  return branch ? { kind: "branch", name: branch, elsewhere: destination != null && branch !== destination } : null;
}

/**
 * A general node's line (PRD node-presets D-08, B1, B2, B5): its preset's
 * icon, and its preset's name until it is done; done, the result line itself,
 * or the verdict with its checks when one was recorded, whatever the preset,
 * which keeps the line beside it for the panel. The icon's tone follows the
 * card (v4): grey while planned, blue while worked, green done, red on a
 * failed verdict. The card, its hover, the node panel, the Conductor pane's
 * results and the phone all read this one value.
 */
export type ResultTone = "planned" | "working" | "done" | "failed";
export type ResultLine = { preset: MissionNodePreset | null; tone: ResultTone } & (
  | { shows: "kind" }
  | { shows: "line"; line: string }
  | { shows: "verdict"; verdict: MissionVerdict; checks: { passed: number; total: number } | null; line: string | null }
);

export function resultLine(node: LineNode): ResultLine | null {
  if (node.kind !== "general") return null;
  const preset = node.preset ?? null;
  const done = node.status === "done";
  if (done && node.verdict) return { preset, tone: node.verdict === "fail" ? "failed" : "done", shows: "verdict", verdict: node.verdict, checks: node.checks ?? null, line: node.line ?? null };
  const tone: ResultTone = done ? "done" : node.status === "working" ? "working" : "planned";
  if (done && node.line) return { preset, tone, shows: "line", line: node.line };
  return { preset, tone, shows: "kind" };
}

/** The branch line's destination as drawn: the branch, or a stack's number beside its pull request's mark (D-57). */
export function lineToLabel(to: LineTo): string {
  return to.kind === "stack" ? String(to.pr) : to.name;
}

/** The destination in words, for the card's name and the hover: the branch, or `PR n` for a stack (D-57). */
export function lineToWords(to: LineTo, t: TFunction<"translation">): string {
  return to.kind === "stack" ? t("mission.foot.stackPr", { number: String(to.pr) }) : to.name;
}

/** The pull request's words as the line draws them: `#n`, `↗#n`, `+N`. */
export function linePrLabel(pr: NonNullable<BranchLine["pr"]>): string {
  return `${pr.outside ? "↗" : ""}${pr.label ?? `#${pr.number}`}${pr.more > 0 ? ` +${pr.more}` : ""}`;
}

/**
 * The branch line in words, for the card's accessible name and its hover:
 * what it draws, then the pull request's state and the operator's merge, so
 * no state is only a colour.
 */
export function branchLineWords(line: BranchLine, t: TFunction<"translation">): string {
  const head = line.plan ? [t(line.plan === "pr" ? "mission.foot.planPr" : "mission.foot.planDirect")] : [line.pr ? linePrLabel(line.pr) : null, line.branch ? `${line.branch.outside ? "↗ " : ""}${line.branch.name}` : null];
  const path = [...head, line.to ? `→ ${lineToWords(line.to, t)}` : null].filter(Boolean).join(" ");
  const state = line.mark in PR_LOOK ? t(PR_LOOK[line.mark as PrState].word) : null;
  const person = line.person ? [t("mission.merge.mine"), line.person === "due" ? t("mission.merge.turn") : line.person === "merged" ? t("mission.merge.merged") : null] : [];
  return [path, line.direct ? t("mission.foot.planDirect") : null, state, ...person].filter(Boolean).join(" · ");
}

/** The result line in words, as the card draws it. */
export function resultLineWords(line: ResultLine, t: TFunction<"translation">): string {
  switch (line.shows) {
    case "kind":
      return t(nodeKindWord({ kind: "general", preset: line.preset }));
    case "line":
      return line.line;
    case "verdict":
      return `${t(line.verdict === "pass" ? "mission.foot.pass" : "mission.foot.fail")}${line.checks ? ` ${line.checks.passed}/${line.checks.total}` : ""}`;
  }
}
