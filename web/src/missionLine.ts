// A mission node card's bottom line (PRD mission-destination D-51-D-55,
// B54-B60): the one computation the desktop's map card, its hover and the
// phone's card read, from fields both carry, so the two screens draw one line.
//
// A node that changes code (`change`) ends in its branch line,
// `[icon] #n <branch> → <destination>`: the icon is its pull request's state
// as the PR list draws it (`PR_LOOK`), a grey branch before one, faint while
// the node is only planned; the operator's merge is a person before the
// destination. A general node, which changes no code, ends in its result
// line: its preset's icon and name until it is done, then its result line or
// its verdict (PRD node-presets D-08). A waiting node (`wait`) draws what it
// waits on instead (`waitCard`, PRD core-wait D-15-D-17, option A of the
// operator's decision on with-hide/hide pull request 2848): no branch line, so
// it never reads as delivering what it waits on. Every value comes from the
// core; a branch or destination it did not send is left out, never named (B60).

import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { PR_LOOK } from "./prMark";
import type { ChangeRequestRef, MissionDelivery, MissionExternal, MissionMergeStep, MissionNode, MissionNodeBrief, MissionNodePreset, MissionOutside, MissionVerdict, MissionWaitOutcome, MissionWaitTarget, PrState } from "./snapshot";
import type { NodeLook } from "./missions";
import { pullHost, pullLabel } from "./codeHost";

/** What the bottom line reads of a node: the shared rules' fields, its pull requests' base and count, an outside node's facts and, on the desktop, its branch (the phone's frame carries none, D-52). */
export type LineNode = Pick<MissionNodeBrief, "kind" | "preset" | "status" | "pr" | "pr_reference" | "pr_state" | "landing" | "merge"> & {
  delivery: MissionDelivery | null;
  base_pr: number | null;
  /** The pull request it is stacked on as its host's unit names it; absent from a core before it, which reads as `#n`. */
  base_reference?: ChangeRequestRef | null;
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
  /** A waiting node's target, its deadline, the progress of a mission it waits on and how it failed (PRD core-wait D-15-D-17); absent from a core before waits. Its pull request is named by `outside.reference`. */
  wait_target?: MissionWaitTarget | null;
  wait_until_unix_ms?: number | null;
  wait_progress?: { done: number; total: number } | null;
  wait_outcome?: MissionWaitOutcome | null;
  /** The checks a failed CI wait named; absent where the frame carries none. */
  wait_checks?: readonly string[] | null;
  /** A waiting node Hide could not read this time (`확인 못 함`); absent where the frame carries none. */
  unread?: boolean;
  /** Whom a raised reason waits on and the pull request it is about, with its reference; absent where the frame carries none. */
  attention?: { to: "operator" | "conductor"; pr?: number | null; reference?: ChangeRequestRef | null } | null;
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
  return { ...node, pr_base: current?.base ?? null, pr_more: Math.max(node.prs.length - 1, 0), base_state: base?.state ?? null };
}

/** Whether a node's result goes into a branch, so it ends in a branch line (D-51): a change node; a waiting node draws what it waits on instead. */
export function changesCode(node: Pick<MissionNodeBrief, "kind">): boolean {
  return node.kind === "change";
}

/**
 * The branch line's icon (D-52): a pull request's state, drawn by `PR_LOOK`;
 * `pull` a pull request whose state GitHub has not answered yet; `branch`
 * the grey branch before any; `plan_pr` and `plan_direct` the faint marks
 * of a node only planned.
 */
export type LineMark = PrState | "pull" | "branch" | "plan_pr" | "plan_direct";

/** Where the result lands: a branch (bold when it is not the mission's destination) or the pull request it is stacked on, drawn as that pull request's mark by its state (grey when unknown) and a grey number (D-57). */
export type LineTo = { kind: "branch"; name: string; elsewhere: boolean } | { kind: "stack"; pr: number; reference: ChangeRequestRef | null; state: PrState | null };

export type BranchLine = {
  mark: LineMark;
  /** `#n` as its host writes it (`reference`, the one the node carries), and `+N` for the others the node holds. */
  pr: { number: number; reference?: ChangeRequestRef | null; more: number } | null;
  /** The branch; null when not known. */
  branch: { name: string } | null;
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
  return changesCode(node) ? ownLine(node, destination) : null;
}

function ownLine(node: LineNode, destination: string | null): BranchLine {
  const byPr = node.delivery !== "direct" || node.pr != null;
  const person = node.merge?.by === "operator" ? node.merge.state : null;
  const to = node.base_pr != null ? { kind: "stack" as const, pr: node.base_pr, reference: node.base_reference ?? null, state: node.base_state ?? null } : branchTo(byPr ? (node.pr_base ?? destination) : destination, destination);
  // A node only planned says how it will go; one replanned after it opened a pull request keeps the pull request it has.
  if (node.status === "planned" && node.pr == null) {
    return { mark: byPr ? "plan_pr" : "plan_direct", pr: null, branch: null, plan: byPr ? "pr" : "direct", direct: false, to, person };
  }
  const branch = node.branch ? { name: node.branch } : null;
  if (!byPr) return { mark: node.landing?.state === "landed" ? "merged" : "branch", pr: null, branch, plan: null, direct: true, to, person: null };
  const pr = node.pr != null ? { number: node.pr, reference: node.pr_reference ?? null, more: node.pr_more ?? 0 } : null;
  return { mark: pr ? (node.pr_state ?? "pull") : "branch", pr, branch, plan: null, direct: false, to, person };
}

/**
 * What a waiting node waits on (PRD core-wait D-02, D-15, D-16), as its
 * condition names it: a pull request as its code host writes it (`#12`,
 * `!12`) with the state its code host last answered, a branch and the branch
 * it should reach, a mission, or a node of one.
 */
export type WaitAim =
  | { kind: "pr" | "ci"; pr: string; mark: LineMark }
  | { kind: "branch"; branch: string; to: string | null }
  | { kind: "mission"; mission: string }
  | { kind: "node"; mission: string; node: string };

/** Where a waiting node's wait stands, its head's icon and its border: waiting (dotted), read again after a read that failed (`확인 못 함`), met (solid) or failed. */
export type WaitState = "waiting" | "unread" | "met" | "failed";

/**
 * A waiting node's card (option A of the operator's decision on with-hide/hide
 * pull request 2848; PRD core-wait D-15-D-17, B4-B8): its state, the condition
 * it waits for, and its foot: a waited mission's progress and under a day the
 * hours left while it waits, `끝남` once met, how it failed and whose turn
 * that is once it failed, `확인 못 함` while a read could not say.
 */
export type WaitCard = {
  state: WaitState;
  aim: WaitAim;
  progress: { done: number; total: number } | null;
  left: number | null;
  failure: { reason: MissionWaitOutcome; checks: readonly string[] } | null;
  turn: "operator" | "conductor" | null;
};

/** The day a waiting node's deadline shows from (D-15): under it the foot says the hours left. */
const WAIT_LEFT_FROM_MS = 24 * 60 * 60 * 1000;
const HOUR_MS = 60 * 60 * 1000;

/**
 * A waiting node's card at `now` by its look (`nodeLook`), or null for any
 * other node, a waiting one whose target the frame does not carry, and one no
 * one judges any more, dropped or in a cancelled mission before it was met:
 * that card is its look's, struck or cancelled, with no live state. Past that
 * line its raise is open (`openAttention`'s rule), so its turn is the raise's.
 */
export function waitCard(node: LineNode, look: NodeLook, now: number): WaitCard | null {
  if (node.kind !== "wait" || look === "dropped" || look === "cancelled") return null;
  const aim = waitAim(node);
  if (!aim) return null;
  const outcome = node.wait_outcome ?? null;
  const state: WaitState = outcome ? "failed" : node.status === "done" ? "met" : node.unread ? "unread" : "waiting";
  const open = state === "waiting" || state === "unread";
  const until = node.wait_until_unix_ms;
  const left = open && until != null && until > now && until - now < WAIT_LEFT_FROM_MS ? Math.max(1, Math.ceil((until - now) / HOUR_MS)) : null;
  return {
    state,
    aim,
    progress: open ? (node.wait_progress ?? null) : null,
    left,
    failure: outcome ? { reason: outcome, checks: node.wait_checks ?? [] } : null,
    turn: outcome ? (node.attention?.to ?? null) : null,
  };
}

/** What it waits on; a waiting node from a core before waits names only its pull request or branch (`external`). */
function waitAim(node: LineNode): WaitAim | null {
  const target = node.wait_target ?? node.external ?? null;
  if (!target) return null;
  const pr = (number: number) => pullLabel({ number, reference: node.outside?.reference ?? null });
  const mark: LineMark = node.status === "done" ? "merged" : (node.outside?.state ?? "pull");
  if ("pr" in target) return { kind: "pr", pr: pr(target.pr), mark };
  if ("ci" in target) return { kind: "ci", pr: pr(target.ci), mark };
  if ("branch" in target) return { kind: "branch", branch: target.branch, to: node.outside?.base ?? null };
  if ("mission" in target) return { kind: "mission", mission: target.mission };
  return { kind: "node", mission: target.node.mission, node: target.node.node };
}

/** The condition in one sentence: `PR 머지되면`, `PR CI 통과하면`, `main에 release/1.3 들어오면`, `mission-91 끝나면`, `mission-91 · gitlab-write 끝나면`. */
export function waitConditionWords(aim: WaitAim, t: TFunction<"translation">): string {
  switch (aim.kind) {
    case "pr":
      return t("mission.wait.if.pr", { pr: aim.pr });
    case "ci":
      return t("mission.wait.if.ci", { pr: aim.pr });
    case "branch":
      return aim.to ? t("mission.wait.if.branch", { branch: aim.branch, to: aim.to }) : t("mission.wait.if.branchDefault", { branch: aim.branch });
    case "mission":
      return t("mission.wait.if.mission", { mission: aim.mission });
    case "node":
      return t("mission.wait.if.node", { mission: aim.mission, node: aim.node });
  }
}

/** How a wait failed in words: `머지 없이 닫힘`, `CI 실패 · build 외 1`, `기다리던 미션이 취소됨`, `노드가 계획에서 빠짐`, `기한 지남`. */
export function waitFailureWords(failure: NonNullable<WaitCard["failure"]>, t: TFunction<"translation">): string {
  const [first, ...rest] = failure.checks;
  if (failure.reason === "ci_failed" && first) {
    return rest.length > 0 ? t("mission.wait.failed.ciChecks", { check: first, more: String(rest.length) }) : t("mission.wait.failed.ciCheck", { check: first });
  }
  return t(`mission.wait.failed.${failure.reason}`);
}

/** Whose turn a failed wait is, in words: `Conductor 차례`, or 내 차례 when the core raised it to the operator. */
export function waitTurnWords(turn: NonNullable<WaitCard["turn"]>, t: TFunction<"translation">): string {
  return turn === "operator" ? t("agents.mark.turn") : t("mission.attention.to.conductor");
}

/** The card's foot in words, in its order, for the card's name, its hover and a list: none when the foot has nothing. */
export function waitFootWords(card: WaitCard, t: TFunction<"translation">): string[] {
  switch (card.state) {
    case "met":
      return [t("mission.wait.met")];
    case "failed":
      return [waitFailureWords(card.failure!, t), card.turn ? waitTurnWords(card.turn, t) : null].filter((word): word is string => word != null);
    case "unread":
    case "waiting":
      return [
        card.progress ? t("mission.wait.progress", { done: String(card.progress.done), total: String(card.progress.total) }) : null,
        card.state === "unread" ? t("mission.wait.unread") : null,
        card.left != null ? t("mission.wait.left", { hours: String(card.left) }) : null,
      ].filter((word): word is string => word != null);
  }
}

/** The whole card in words: the condition, then its foot. */
export function waitCardWords(card: WaitCard, t: TFunction<"translation">): string {
  return [waitConditionWords(card.aim, t), ...waitFootWords(card, t)].join(" · ");
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

/** The destination in words, for the card's name and the hover: the branch, or `PR n` (`MR n` on gitlab.com) for a stack (D-57). */
export function lineToWords(to: LineTo, t: TFunction<"translation">): string {
  return to.kind === "stack" ? t("mission.foot.stackPr", { item: pullHost({ number: to.pr, reference: to.reference }).item, number: String(to.pr) }) : to.name;
}

/** The pull request's words as the line draws them: `#n`, `+N`. */
export function linePrLabel(pr: NonNullable<BranchLine["pr"]>): string {
  return `${pullLabel({ number: pr.number, reference: pr.reference })}${pr.more > 0 ? ` +${pr.more}` : ""}`;
}

/**
 * The branch line in words, for the card's accessible name and its hover:
 * what it draws, then the pull request's state and the operator's merge, so
 * no state is only a colour.
 */
export function branchLineWords(line: BranchLine, t: TFunction<"translation">): string {
  const head = line.plan ? [t(line.plan === "pr" ? "mission.foot.planPr" : "mission.foot.planDirect")] : [line.pr ? linePrLabel(line.pr) : null, line.branch ? line.branch.name : null];
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
