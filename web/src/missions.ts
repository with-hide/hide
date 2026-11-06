// What the mission map and the Conductor row read from a mission (PRD
// software-factory-conductor): the core decides every state and number
// (`MissionView`), so this module only finds a mission, words its parts
// and counts what the record holds. Pure, so the rules test on their own;
// `MissionMap.tsx` draws them.

import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { PROVIDER_KINDS, type ProviderKind } from "./agentPicker";
import { agentsByPane } from "./paneAgents";
import { planCells, planChecklist, stepOf } from "./plans";
import { orderByDependencies } from "./projectBoard";
import { resumeProvider } from "./linkPanel";
import type { AgentRow, AgentStartChoice, Mission, MissionAttention, MissionBrief, MissionNodeBrief, MissionAttentionReason, MissionChecker, MissionCriterion, MissionWorker, MissionEvent, MissionEventsSnapshot, MissionHideAction, MissionLandingAction, MissionNode, MissionNodePr, MissionNodeWarning, MissionOperatorAction, MissionVerdict, RowMark, SnapshotRest } from "./snapshot";

export function missionOf(rest: SnapshotRest | null | undefined, id: string): Mission | null {
  return rest?.missions?.items.find((mission) => mission.id === id) ?? null;
}

/** Whether a mission is still the Conductor's to carry: planned, running or waiting on a merge. */
export function missionOpen(mission: Pick<Mission, "status">): boolean {
  return mission.status === "planning" || mission.status === "running" || mission.status === "merge_wait";
}

/** The pane or the session a mission's Conductor is known by: a live row's pane, an archived row's session. */
export type ConductorOwner = { pane_id?: string; device_id?: string; session?: { device_id: string; session_id: string } | null };

/**
 * The mission a row conducts (B14, B46): its open one, else the last it
 * closed, so a done mission's map stays reachable from its Conductor, a
 * live row by its pane and an archived one by its session.
 */
export function conductedBy(rest: SnapshotRest | null | undefined, owner: ConductorOwner): Mission | null {
  const mine = (rest?.missions?.items ?? []).filter((mission) => {
    const session = mission.conductor.session;
    if (owner.session && session) return session.device_id === owner.session.device_id && session.session_id === owner.session.session_id;
    return owner.pane_id !== undefined && mission.conductor.pane_id === owner.pane_id && (!owner.device_id || mission.device_id === owner.device_id);
  });
  return mine.find(missionOpen) ?? mine.filter((mission) => mission.status !== "planning").sort((a, b) => (b.closed_at_unix_ms ?? 0) - (a.closed_at_unix_ms ?? 0))[0] ?? null;
}

/**
 * The mission a pane's session conducts, found through the pane's agent row as
 * its sidebar row finds it (PRD mission-pane-chip B1, B10): none while the
 * record cannot be read, so no count is invented.
 */
export function missionOfPane(rest: SnapshotRest | null | undefined, agents: AgentRow[], paneId: string): Mission | null {
  if (rest?.missions?.failed) return null;
  const agent = agentsByPane(agents).get(paneId);
  return agent ? conductedBy(rest, agent) : null;
}

/** What the record counts (B21): done nodes of those still in the plan, and the ones dropped. */
export function missionCounts(mission: { nodes: readonly Pick<MissionNode, "status">[] }): { done: number; total: number; dropped: number } {
  const dropped = mission.nodes.filter((node) => node.status === "dropped").length;
  return { done: mission.nodes.filter((node) => node.status === "done").length, total: mission.nodes.length - dropped, dropped };
}

/** The pull requests of a stack still waiting on a lower one, as the core counts them. */
export function stackWaiting(mission: Mission): number {
  return mission.stack_waiting;
}

const KIND_WORD: Record<MissionNode["kind"], MessageKey> = {
  change: "mission.kind.change",
  research: "mission.kind.research",
  verify: "mission.kind.verify",
  external: "mission.kind.external",
};

/** A node's own criteria in the Conductor's words, one after another; null when it names none. */
export function ownCriteria(node: MissionNode): string | null {
  const own = node.criteria.flatMap((line) => (line.kind === "own" && line.text ? [line.text] : []));
  return own.length > 0 ? own.join(" · ") : null;
}

export function kindWord(kind: MissionNode["kind"]): MessageKey {
  return KIND_WORD[kind];
}

/**
 * A node's line (B15, B51, B53): why it waits on a dropped node, else what
 * its worker last said, else its kind. A node that waits on a node not done
 * yet says nothing of it: the edge above it does.
 */
export function nodeLine(node: MissionNode, t: TFunction<"translation">): string {
  if (node.blocked === "upstream_dropped") return t("mission.blocked.dropped");
  if (node.line) return node.line;
  return t(kindWord(node.kind));
}

/** The launch Hide recorded for a pane (`overview_graph.sessions`), on its machine or the mission's; the store's own row, so a selector can return it. */
export function launchSession(rest: SnapshotRest | null | undefined, mission: Mission, paneId: string | null | undefined, machine: string | null | undefined): LaunchSession | null {
  if (!paneId) return null;
  const device = machine ?? mission.device_id;
  return rest?.overview_graph?.sessions.find((one) => one.device_id === device && one.pane_id === paneId) ?? null;
}

type LaunchSession = NonNullable<SnapshotRest["overview_graph"]>["sessions"][number];

/** What runs in a pane or what the plan asked, and which of the two it is. */
export type RunWords = { words: string; planned: boolean };

/**
 * A worker or a checker as the panel and a checker strip name it (B29),
 * never by its agent, whose logo stands beside the words. Once Hide proved
 * the pane's launch choice, only that launch speaks: its model, or the CLI's
 * default when it set none, and its effort when it set one; the plan is not
 * mixed in. The core sends a session it proved no choice for with neither
 * value, so neither value is no proof of a launch. Without one the plan's
 * model and effort are shown marked as planned (`자동` for an effort the
 * Conductor will choose), or nothing.
 */
export function runWords(launch: Pick<LaunchSession, "launch_model" | "launch_effort"> | null, plan: MissionWorker | null | undefined, words: { auto: string; defaultModel: string }): RunWords | null {
  if (launch && (launch.launch_model || launch.launch_effort)) return { words: [launch.launch_model ?? words.defaultModel, launch.launch_effort].filter(Boolean).join("·"), planned: false };
  if (!plan) return null;
  const effort = plan.effort ?? (plan.effort_auto ? words.auto : null);
  const planned = [plan.model, effort].filter(Boolean).join("·");
  return planned ? { words: planned, planned: true } : null;
}

/** A change node's steps: work, PR or branch, review, merge or main (D-42 B, D-48). */
export const STAGE_STEPS = 4;

/**
 * The popover row's status mark (B15): its worker's own mark while its row is
 * listed, so a node reads as its agent does everywhere; else the node's
 * state, a node that waits on another dashed.
 */
export function nodeMark(node: Pick<MissionNode, "status" | "blocked" | "agent_runtime">, worker: RowMark | null): RowMark {
  // A working node whose worker Hide put to sleep is not at work, row or none.
  if (workerAsleep(node)) return "asleep";
  if (worker && node.status === "working") return worker;
  if (node.blocked) return "waiting";
  switch (node.status) {
    case "working":
      return "going";
    case "done":
      return "done";
    case "planned":
    case "dropped":
      return "quiet";
  }
}

/** The Conductor row's mark (B14): `미션 n/m` while it runs, its plan or its wait otherwise. */
export function chipWords(mission: Mission): { key: MessageKey; counts: string | null } {
  const { done, total } = missionCounts(mission);
  switch (mission.status) {
    case "planning":
      return { key: "mission.chip.plan", counts: null };
    case "merge_wait":
      return { key: "mission.chip.mergeWait", counts: null };
    case "done":
      return { key: "mission.chip.done", counts: null };
    case "cancelled":
      return { key: "mission.chip.cancelled", counts: null };
    case "running":
      return { key: "mission.chip", counts: `${done}/${total}` };
  }
}

/** The Conductor pane's chip (PRD mission-pane-chip B2): the state it reads as, its word and its count. */
export type PaneChipState = "planning" | "running" | "decision" | "merge_wait" | "done" | "cancelled";

/** Whether an open mission waits on the operator's decision: its chips keep their progress and add `!` in the decision colour (needs-you-cleanup D-02). */
export function missionDeciding(mission: Mission): boolean {
  return Boolean(mission.decision) && missionOpen(mission);
}

/** The pane chip's words, its progress kept while a decision waits; the decision only colours it (`decision`) and marks it `!` (needs-you-cleanup B6). */
export function paneChipWords(mission: Mission): { state: PaneChipState; key: MessageKey; counts: string | null } {
  const words = paneStateWords(mission);
  return missionDeciding(mission) ? { ...words, state: "decision" } : words;
}

function paneStateWords(mission: Mission): { state: PaneChipState; key: MessageKey; counts: string | null } {
  const { done, total } = missionCounts(mission);
  const counts = `${done}/${total}`;
  switch (mission.status) {
    case "planning":
      // A plan put up and not started yet counts from none done (approved board 4).
      return mission.nodes.length === 0 ? { state: "planning", key: "mission.pane.planning", counts: null } : { state: "running", key: "mission.chip", counts };
    case "running":
      return { state: "running", key: "mission.chip", counts };
    case "merge_wait":
      return { state: "merge_wait", key: "mission.pane.mergeWait", counts: null };
    case "done":
      return { state: "done", key: "mission.chip", counts };
    case "cancelled":
      return { state: "cancelled", key: "mission.pane.cancelled", counts: null };
  }
}

/**
 * A node's state as the head's and the chip's cells colour it (PRD
 * mission-node-simple D-01, D-02, B5): waiting, in progress, done, and a
 * node someone has to look at in the decision colour (`hold`). A node that
 * cannot start yet waits; once the mission is cancelled, a node it never
 * finished is cancelled, and a dropped node left the plan, so it has no cell.
 */
export type NodeTone = "done" | "working" | "hold" | "waiting" | "cancelled" | "dropped";

export function nodeTone(mission: Pick<Mission, "status">, node: Pick<MissionNode, "status" | "blocked" | "attention">): NodeTone {
  if (node.status === "dropped") return "dropped";
  if (mission.status === "cancelled" && node.status !== "done") return "cancelled";
  if (openAttention(mission, node)) return "hold";
  if (node.blocked) return "waiting";
  switch (node.status) {
    case "done":
      return "done";
    case "working":
      return "working";
    case "planned":
      return "waiting";
  }
}

/**
 * A cell's state on a bar (B8, B24): a node's tone, and `review` for a
 * mission whose merges wait on the operator, as its chip wears it.
 */
export type CellTone = Exclude<NodeTone, "dropped"> | "review";

/** The nodes top down by what they wait on, a dropped node where it stood (B17). */
export function orderedNodes<N extends MissionNodeBrief>(mission: { nodes: readonly N[] }): N[] {
  return orderByDependencies(mission.nodes, (node) => node.key, (node) => node.depends_on).map(({ item }) => item);
}

/** The chip's and the head's bar (B8, B24): one cell per node in the plan, in order. */
export function missionCells(mission: MissionBrief): CellTone[] {
  return orderedNodes(mission).flatMap((node) => {
    const tone = nodeTone(mission, node);
    return tone === "dropped" ? [] : [tone];
  });
}

/**
 * A head cell per node in the plan order, drawn as its card is (B26): a node
 * someone has to look at in the decision colour (PRD mission-node-simple B5);
 * dropped nodes have none.
 */
export type HeadCell = Exclude<NodeLook, "dropped"> | "attention";
export function missionHeadCells(mission: Mission): HeadCell[] {
  return orderedNodes(mission).flatMap((node): HeadCell[] => {
    const look = nodeLook(mission, node);
    if (look === "dropped") return [];
    return [openAttention(mission, node) ? "attention" : look];
  });
}

/** The colour a mission's own chip wears on the sidebar row and the Overview (B35): its decision, else its state. */
export function missionTone(mission: Mission): CellTone {
  return missionToneOf(mission.status, missionDeciding(mission));
}

/** A mission chip's colour from its status and whether a decision waits: the one rule every mission chip, the sidebar's and Desk's, follows. */
export function missionToneOf(status: Mission["status"], deciding: boolean): CellTone {
  if (deciding) return "hold";
  switch (status) {
    case "planning":
      return "waiting";
    case "running":
      return "working";
    case "merge_wait":
      return "review";
    case "done":
      return "done";
    case "cancelled":
      return "cancelled";
  }
}

/** The nodes a worker is at now, in order (B9, B10); one whose worker sleeps is not. */
export function liveNodes<N extends MissionNodeBrief>(mission: { nodes: readonly N[] }): N[] {
  return orderedNodes(mission).filter((node) => node.status === "working" && !workerAsleep(node));
}

/**
 * Why a step waits on someone, in the order a step is read: a node raised
 * to the operator or the Conductor (`! 나`, `! Conductor`), a working step
 * whose worker the core raised past its parent (one of the seven escalation
 * causes, the same worker `내 차례` lists), or the working step the open
 * decision stopped. Null for a step nobody has to look at.
 */
export type HoldReason = { kind: "attention"; attention: MissionAttention } | { kind: "worker"; escalation: WorkerEscalation } | { kind: "decision" };

/** What `holdReason` reads of a mission: its state, and the decision with the node it stopped. */
export type HoldMission = Pick<Mission, "status" | "decision"> & { question?: Pick<NonNullable<Mission["question"]>, "node"> | null };

export function holdReason<N extends MissionNodeBrief>(mission: HoldMission, node: N, escalationOf: (node: N) => WorkerEscalation | null): HoldReason | null {
  const attention = openAttention(mission, node);
  if (attention) return { kind: "attention", attention };
  if (node.status !== "working") return null;
  const escalation = escalationOf(node);
  if (escalation) return { kind: "worker", escalation };
  return mission.decision && mission.question?.node === node.key ? { kind: "decision" } : null;
}

/**
 * The summary's groups in the map's order (PRD mission-node-simple B5; PRD
 * mobile-companion-ui B33, B36): the steps that wait on someone
 * (`holdReason`), raised nodes first and then the working steps a raised
 * worker or the decision holds, those in progress, and those done; a step
 * that waits is in the first group only, so none reads as working on the
 * desktop's summary or the phone's 지금, and a node not started or dropped
 * is in none. `escalationOf` answers for a node's worker: the desktop reads
 * its row, the phone the escalation its core sent with the node.
 */
export function summaryGroups<N extends MissionNodeBrief>(mission: HoldMission & { nodes: readonly N[] }, escalationOf: (node: N) => WorkerEscalation | null): { attention: N[]; working: N[]; done: N[] } {
  const nodes = orderedNodes(mission).filter((node) => node.status !== "dropped");
  const raised = (node: N) => openAttention(mission, node) !== null;
  const held = (node: N) => holdReason(mission, node, escalationOf) !== null;
  return {
    attention: [...nodes.filter(raised), ...nodes.filter((node) => !raised(node) && held(node))],
    working: nodes.filter((node) => !held(node) && node.status === "working"),
    done: nodes.filter((node) => !held(node) && node.status === "done"),
  };
}

/** The running popover's last line (B10): the nodes done, and those still ahead of any worker. */
export function missionTally(mission: Mission): { done: number; next: number } {
  return { done: mission.nodes.filter((node) => node.status === "done").length, next: mission.nodes.filter((node) => node.status === "planned").length };
}

/**
 * The Conductor pane's second line (B9, D-21): the decision waiting on the
 * operator first, else `<first> 외 k`; nothing with no worker at it. The count
 * is left to the children button's marks beside it.
 */
export function missionWorkLine(mission: Mission, t: TFunction<"translation">): { decision: boolean; text: string } | null {
  if (mission.decision && missionOpen(mission)) return { decision: true, text: `! ${mission.decision}` };
  if (!missionOpen(mission)) return null;
  const live = liveNodes(mission);
  const [first] = live;
  if (!first) return null;
  const text = live.length > 1 ? t("mission.line.others", { title: first.title, count: live.length - 1 }) : first.title;
  return { decision: false, text };
}

const PR_STAGES: readonly MessageKey[] = ["mission.stage.code", "mission.stage.pr", "mission.stage.review", "mission.stage.merge"];
const DIRECT_STAGES: readonly MessageKey[] = ["mission.stage.code", "mission.stage.check", "mission.stage.review", "mission.stage.main"];

/** Where a node's work is, named (B10): a change node's step of four, else its kind's one word. */
export function stageWord(node: Pick<MissionNode, "kind" | "delivery" | "stage">): MessageKey {
  if (node.kind === "research") return "mission.stage.research";
  if (node.kind === "verify") return "mission.stage.verify";
  const stages = node.delivery === "direct" ? DIRECT_STAGES : PR_STAGES;
  return stages[Math.min(Math.max(node.stage ?? 1, 1), STAGE_STEPS) - 1]!;
}

/**
 * What a node left behind, by its kind (B22, D-14): its pull request, a
 * direct change's commit on main or else its branch, a research node's
 * conclusion, a verify node's recorded verdict or else its line. A research
 * or verify node's line is a result only once the node is done; while it
 * runs the line is its progress. Null when the record holds none, so
 * nothing is drawn for it.
 */
export type NodeResult =
  | { kind: "pr"; number: number; state: MissionNode["pr_state"] }
  | { kind: "commit"; commit: string }
  | { kind: "branch"; branch: string }
  | { kind: "conclusion"; line: string }
  | { kind: "verdict"; verdict: MissionVerdict; checks: { passed: number; total: number } | null }
  | { kind: "verify"; line: string };

export function nodeResult(node: MissionNode): NodeResult | null {
  if (node.pr != null) return { kind: "pr", number: node.pr, state: node.pr_state };
  if (node.kind === "change") {
    const commit = mainCommit(node);
    return commit ? { kind: "commit", commit } : node.branch ? { kind: "branch", branch: node.branch } : null;
  }
  if (node.kind === "verify" && node.verdict) return { kind: "verdict", verdict: node.verdict, checks: node.checks };
  if (!node.line) return null;
  if (node.kind === "research") return node.status === "done" ? { kind: "conclusion", line: node.line } : null;
  return node.status === "done" ? { kind: "verify", line: node.line } : null;
}

/**
 * The commit a change has in main (B17): the one Hide read there for the
 * node's branch, else one the Conductor recorded for a direct change by hand.
 */
export function mainCommit(node: MissionNode): string | null {
  const proof = node.landing?.state === "landed" ? node.landing.proof : null;
  return proof?.by === "in_main" ? proof.commit : node.commit;
}

/** A commit as the results show it: its first seven digits. */
export function shortCommit(commit: string): string {
  return commit.slice(0, 7);
}

/** A finished or cancelled mission's results (B11, B23, B24): each done node that left something, and a verify node with its verdict, in order. */
export function missionResults(mission: Mission): { node: MissionNode; result: NodeResult }[] {
  return orderedNodes(mission).flatMap((node) => {
    const result = node.status === "done" || (node.kind === "verify" && node.verdict) ? nodeResult(node) : null;
    return result ? [{ node, result }] : [];
  });
}

/**
 * How a card is drawn (PRD mission-node-simple D-01, B1): one of three
 * shapes, waiting, in progress and done, each readable without its colour;
 * a working node whose worker sleeps is still in progress, its sleep said by
 * the mark's hover and the panel, and what someone has to look at is the card's `!` tag (`node.attention`), not a
 * shape. Finished work has one look whether or not it reached main. An
 * outside node watches until its pull request merges or its branch reaches
 * main. Being outside is a node's kind, not a state: its card keeps the
 * dotted border and `↗` in every look (B19).
 */
export type NodeLook = "waiting" | "working" | "done" | "external" | "cancelled" | "dropped";

/**
 * A node at work whose worker Hide put to sleep, read from the core's one
 * answer of an agent's life (`agent_runtime`, as `hide agent show` gives
 * it): nothing works it until a letter wakes it.
 */
export function workerAsleep(node: Pick<MissionNode, "status" | "agent_runtime">): boolean {
  return node.status === "working" && node.agent_runtime === "asleep";
}

export function nodeLook(mission: Mission, node: MissionNode): NodeLook {
  if (node.status === "dropped") return "dropped";
  if (node.status === "done") return "done";
  if (mission.status === "cancelled") return "cancelled";
  if (node.kind === "external") return "external";
  if (node.blocked || node.status === "planned") return "waiting";
  // A worker Hide put to sleep leaves its node in progress (PRD mission-node-simple D-01, B1): its sleep is the panel's and the mark's hover words.
  return "working";
}

/** Whether the core found a node's work in main (B17, B19). */
export function nodeLanded(node: MissionNode): boolean {
  return node.landing?.state === "landed";
}

/**
 * What a finished change still waits on before main (B27), as the core
 * decides it: `merge` for a pull request it read open, `join` for a done
 * direct change's branch it reads and has not found in main. Nothing for
 * work it could not read, which carries no tag.
 */
export function landingLeft(node: MissionNode): MissionLandingAction | null {
  return node.landing?.state === "left" ? node.landing.action : null;
}

/** A card's `기준 n/m` and `⤳k` (B28): none for a mission from before criteria (B23) or a node with no lines, as an outside one. */
export function criteriaTally(mission: Mission, node: MissionNode): { passed: number; total: number; skipped: number } | null {
  if (mission.exempt || node.criteria.length === 0) return null;
  // The core's `criteria_met` holds the skipped lines too; `n` is the ✓ lines alone, the skips standing beside it as `⤳k`.
  return { passed: node.criteria_met - node.criteria_skipped, total: node.criteria.length, skipped: node.criteria_skipped };
}

/** The lines a node still has to meet, open or failed, in its order (B22, B28). */
export function linesLeft(node: MissionNode): MissionCriterion[] {
  return node.criteria.filter((line) => line.state === "open" || line.state === "fail");
}

/** Who checks a review or verify line, by agent, as its words name them (`codex 리뷰`, B32). */
function checkerNames(line: MissionCriterion): string {
  return [...new Set((line.checkers ?? []).map((checker) => checker.worker.agent))].join(" · ");
}

/**
 * A line in words (B32): the Conductor's own words for the node's criteria
 * and a defect, and for the lines Hide adds the check command, the CI and the
 * review or verification with its checkers' names; no origin and no column
 * of who checks.
 */
export function criterionName(line: MissionCriterion, t: TFunction<"translation">): string {
  switch (line.kind) {
    case "own":
    case "defect":
      return line.text ?? line.key;
    case "check":
      return t("mission.line.check");
    case "ci":
      return t("mission.line.ci");
    case "review":
    case "verify": {
      const who = checkerNames(line);
      return who ? t(`mission.line.${line.kind}.by`, { who }) : t(`mission.line.${line.kind}`);
    }
  }
}

/** A line in one word for `남음: 리뷰, CI` (B28): a criterion by its number, the rest by kind. */
export function criterionShort(line: MissionCriterion, t: TFunction<"translation">): string {
  if (line.kind === "own") return t("mission.line.short.own", { number: line.key.replace(/^\D+/, "") || line.key });
  return t(`mission.line.short.${line.kind}`);
}

/** Of several checkers, how many passed (`2/3 통과`, B6); none for one checker or a line without them. */
export function checkerTally(line: MissionCriterion): { passed: number; total: number } | null {
  const checkers = line.checkers ?? [];
  if (checkers.length < 2) return null;
  return { passed: checkers.filter((checker) => checker.state === "pass").length, total: checkers.length };
}

/** One checker on its strip (B29): its answer, else at work once the Conductor bound an agent to it, else still to start. */
export function checkerState(checker: MissionChecker): "pass" | "fail" | "running" | "waiting" {
  if (checker.state === "pass" || checker.state === "fail") return checker.state;
  return checker.agent_id ? "running" : "waiting";
}

/**
 * What a line adds after its words (B12, B21, B32): why it was skipped, the
 * operator's decision for a check or CI line, a rerun that passed, a rerun
 * running, the checkers' count while several decide, or the node that found
 * a defect.
 */
export function criterionNote(line: MissionCriterion, mission: Mission, t: TFunction<"translation">): string | null {
  if (line.state === "skipped" && line.skip) return line.skip.decision != null ? t("mission.line.operator", { reason: line.skip.reason }) : line.skip.reason;
  if (line.rerunning) return t("mission.line.rerunning");
  if (line.state === "pass" && line.reruns) return t("mission.line.reruns", { count: line.reruns });
  const tally = checkerTally(line);
  if (tally && line.state === "open") return t("mission.line.passedOf", { passed: String(tally.passed), total: String(tally.total) });
  if (line.kind === "defect" && line.found_by) {
    const finder = mission.nodes.find((node) => node.key === line.found_by);
    return t("mission.line.foundBy", { node: finder?.title ?? line.found_by });
  }
  return null;
}

/**
 * How long a node took from its events (B28): from the first time it went
 * to work to when it ended, or to `end` while it has not. Null with no event
 * that moved it, as in a mission recorded before its events, so no number is
 * drawn for it.
 */
export function nodeTimes(node: MissionNode, events: readonly MissionEvent[], end: number): { took: number | null; running: boolean } | null {
  const moves = events.filter((event) => event.kind === "node" && event.node === node.key && event.status);
  if (moves.length === 0) return null;
  const start = moves.find((move) => move.status === "working");
  const last = moves[moves.length - 1]!;
  const running = last.status !== "done" && last.status !== "dropped";
  const stop = running ? end : last.at_unix_ms;
  return { took: start ? Math.max(0, stop - start.at_unix_ms) : null, running };
}

/**
 * Where a defect a later node found points (B21): from the node that found it
 * to the node it opened again, while that line is still open or failing.
 */
export function defectEdges(mission: Mission): { from: string; to: string }[] {
  const keys = new Set(mission.nodes.map((node) => node.key));
  const edges = new Map<string, { from: string; to: string }>();
  for (const node of mission.nodes) {
    for (const line of node.criteria) {
      if (line.kind !== "defect" || !line.found_by || !keys.has(line.found_by) || (line.state !== "open" && line.state !== "fail")) continue;
      edges.set(`${line.found_by}>${node.key}`, { from: line.found_by, to: node.key });
    }
  }
  return [...edges.values()];
}

/** The card a selection keeps lit (B30): itself and every card one edge away, either way. */
export function neighbours(edges: readonly { from: string; to: string }[], key: string): Set<string> {
  const near = new Set([key]);
  for (const edge of edges) {
    if (edge.from === key) near.add(edge.to);
    if (edge.to === key) near.add(edge.from);
  }
  return near;
}

/**
 * A node's one line where it is now (B4): where a working node is (its
 * worker's plan, the Conductor's line, else what is left), what a blocked
 * node waits on, what a planned node waits for, and what an outside node
 * watches; nothing for finished work, whose result speaks.
 */
export function cardLine(mission: Mission, node: MissionNode, planLine: string | null, t: TFunction<"translation">): { text: string; attention: boolean } | null {
  const look = nodeLook(mission, node);
  if (look === "cancelled" || look === "dropped") return null;
  // An outside node says where its pull request or branch is in every state, finished too (B19).
  if (node.kind === "external") {
    if (node.attention?.reason === "pr_closed") return { text: t("mission.outside.closed"), attention: true };
    if (look === "done") return { text: t("mission.look.landed"), attention: false };
    return { text: t(node.unread ? "mission.outside.unread" : "mission.outside.waiting"), attention: false };
  }
  if (look === "done") return null;
  if (look === "working") {
    const left = linesLeft(node);
    const text = planLine ?? node.line ?? (left.length > 0 ? t("mission.line.left", { lines: left.map((line) => criterionShort(line, t)).join(", ") }) : null);
    return text ? { text, attention: false } : null;
  }
  if (node.blocked) return { text: nodeLine(node, t), attention: true };
  const before = waitingFor(mission, node).map((dep) => (dep.pr != null ? `#${dep.pr}` : dep.title));
  if (before.length > 0) return { text: t("mission.line.waitsFor", { names: before.join(" ") }), attention: false };
  return node.line ? { text: node.line, attention: false } : null;
}

/** The changes left to merge or join, in the order the core gives them (B18, B25). */
export function landingOrder<N extends MissionNodeBrief>(mission: { nodes: readonly N[]; landing_order: readonly string[] }): N[] {
  const byKey = new Map(mission.nodes.map((node) => [node.key, node]));
  return mission.landing_order.flatMap((key) => {
    const node = byKey.get(key);
    return node ? [node] : [];
  });
}

/** Why a worker's row was raised to the operator (`AgentRow.escalation`, with-hide/hide-private docs/task-identity.md, Delegated escalation). */
export type EscalationCause = NonNullable<AgentRow["escalation"]>["cause"];

/**
 * What only the operator can do now (B25; PRD mission-node-simple B14): answer
 * the decision, take over a mission no Conductor holds, settle a node the
 * core raised to the operator (`! 나`), clear a worker the core raised past
 * its parent (one of the seven escalation causes), and merge or join what
 * waits for main. What waits on the Conductor stays on its card, and a
 * finished or cancelled mission has nothing here.
 */
export type TurnItem<N extends MissionNodeBrief = MissionNode> =
  | { kind: "decision" }
  | { kind: "takeover" }
  | { kind: "attention"; node: N; attention: MissionAttention }
  | { kind: "worker_blocked"; node: N; cause: EscalationCause; line: string | null }
  | { kind: "landing"; nodes: N[]; git: boolean };

/** A worker row the core raised past its parent: the cause, and the line its blocked turn left. */
export type WorkerEscalation = { cause: EscalationCause; line: string | null };

/** The escalation of a node's worker row, as the `내 차례` rule reads it; none for a row that was not raised. */
export function workerEscalation(row: Pick<AgentRow, "escalation" | "request"> | null | undefined): WorkerEscalation | null {
  if (!row?.escalation) return null;
  const request = row.request;
  return { cause: row.escalation.cause, line: request?.end === "blocked" && request.line ? request.line : null };
}

/** `escalationOf` answers for a node's worker: the desktop reads its row, the phone the escalation its core sent with the node. */
export function turnItems<N extends MissionNodeBrief>(mission: MissionBrief<N>, escalationOf: (node: N) => WorkerEscalation | null): TurnItem<N>[] {
  if (!missionOpen(mission)) return [];
  const items: TurnItem<N>[] = [];
  if (mission.decision) items.push({ kind: "decision" });
  else if (!mission.conductor.present) items.push({ kind: "takeover" });
  for (const node of orderedNodes(mission)) {
    const attention = openAttention(mission, node);
    if (attention?.to === "operator") items.push({ kind: "attention", node, attention });
  }
  for (const node of orderedNodes(mission)) {
    const escalation = node.status === "working" ? escalationOf(node) : null;
    if (escalation) items.push({ kind: "worker_blocked", node, cause: escalation.cause, line: escalation.line });
  }
  if (mission.status === "merge_wait") {
    const nodes = landingOrder(mission);
    if (nodes.length > 0) items.push({ kind: "landing", nodes, git: nodes.every((node) => node.pr == null) });
  }
  return items;
}

/** A step's state on the map: finished, being worked, or only planned (the worker has not reached it). */
export type StepTone = "done" | "working" | "plan";

/** A step of a node as the card's bar draws it. */
export type NodeStep = { title: string; tone: StepTone };

/**
 * How a node's work goes, from the one source that is true now: the worker's
 * own plan once it exists (`worker`), else the steps the Conductor planned
 * (`plan`), never both as two lists. `now` is the worker's current step.
 * Null when the node has neither. Which worker step differs from the
 * Conductor's is the checklist's to mark (`PlanChecklist`'s `planned`).
 */
export function nodeSteps(node: MissionNode): { source: "worker" | "plan"; steps: NodeStep[]; now: string | null } | null {
  const plan = node.plan;
  if (plan) {
    // `planCells` lists the plan's steps in this order, a dropped step left out.
    const rows = planChecklist(plan).filter(({ step }) => step.status !== "dropped");
    const cells = planCells(plan);
    const steps = rows.map(({ step }, index): NodeStep => ({
      title: step.title,
      tone: cells[index] === "done" ? "done" : cells[index] === "working" || cells[index] === "hold" ? "working" : "plan",
    }));
    if (steps.length > 0) return { source: "worker", steps, now: plan.progress.finished ? null : (stepOf(plan, plan.progress.current)?.title ?? null) };
  }
  if (node.steps.length === 0) return null;
  return { source: "plan", steps: node.steps.map((title) => ({ title, tone: "plan" })), now: null };
}

/** The events the core sends for `missionId`, which it does only for the mission open in the 보기 slot (D-22). */
export function missionEventsOf(rest: SnapshotRest | null | undefined, missionId: string): MissionEventsSnapshot | null {
  const events = rest?.mission_events;
  return events && events.mission === missionId ? events : null;
}

/** An event in words (B20, B31): what happened, and the one line it carried. */
/** What Hide did, in words, with the pull request it was about. */
function hideAction(action: MissionHideAction, pr: number | null, t: TFunction<"translation">): string {
  return t(`mission.hide.${action}`, { pr: prWords(pr, t) });
}

export function eventWords(event: MissionEvent, t: TFunction<"translation">): { what: string; line: string | null } {
  const line = event.line ?? null;
  switch (event.kind) {
    case "planned":
      return { what: t("mission.event.planned", { count: event.count ?? 0 }), line: null };
    case "replanned":
      return { what: t("mission.event.replanned", { count: event.count ?? 0 }), line };
    case "node":
      return { what: event.status ? t(`mission.status.${event.status}`) : t("mission.event.node"), line };
    case "retried":
      return { what: t("mission.event.retried", { count: event.count ?? 0 }), line: null };
    case "pr_opened":
      return { what: t("mission.event.prOpened", { number: String(event.pr ?? "") }), line: null };
    case "pr_merged":
      return { what: t("mission.event.prMerged", { number: String(event.pr ?? "") }), line: null };
    case "landed":
      return { what: t("mission.event.landed"), line: line ? shortCommit(line) : null };
    case "asked":
      return { what: t("mission.event.asked"), line: [line, ...(event.options ?? [])].filter(Boolean).join(" / ") || null };
    case "answered":
      return { what: t(event.source === "banner" ? "mission.event.answered.banner" : "mission.event.answered.conversation"), line };
    case "answer_undelivered":
      return { what: t("mission.event.undelivered"), line };
    case "answer_unconfirmed":
      return { what: t("mission.event.unconfirmed"), line };
    case "reported":
      return { what: t("mission.event.reported"), line: null };
    case "cancelled":
      return { what: t("mission.event.cancelled"), line: null };
    case "skipped":
      return { what: t("mission.event.skipped"), line };
    case "reopened":
      return { what: t("mission.event.reopened"), line };
    case "strength":
      return { what: t("mission.event.strength", { strength: event.strength ? t(`mission.strength.${event.strength}`) : "" }), line };
    case "hide":
      return { what: event.hide ? t("mission.hide.mark", { what: hideAction(event.hide, event.pr ?? null, t) }) : t("mission.hide.title"), line: event.why ? `${prWords(event.pr ?? null, t)} ${t(`mission.fix.${event.why}`)}` : line };
    case "hide_closed":
      return { what: t("mission.hide.mark", { what: t("mission.hide.closed") }), line };
    case "raised":
    case "raised_to_operator":
      return { what: `! ${t(event.kind === "raised" ? "mission.attention.to.conductor" : "mission.attention.to.operator")}`, line: event.reason ? attentionReasonText(event.reason, event.pr ?? null, t) : line };
    case "resolved":
      return { what: event.action ? `${t("mission.operator")} · ${t(`mission.operator.${event.action}`)}` : t("mission.operator"), line };
  }
}

/**
 * The question the decision asks of a node (B16, B25): the mission's open
 * decision on the node it names; a decision naming no node holds none.
 */
export function nodeQuestion(mission: Mission, node: MissionNode): string | null {
  if (!missionDeciding(mission) || mission.question?.node !== node.key) return null;
  return mission.decision;
}

/** A pull request in words: `PR #n`, or `PR` when the record names none. */
function prWords(pr: number | null, t: TFunction<"translation">): string {
  return pr != null ? t("mission.attention.pr", { number: String(pr) }) : t("mission.attention.prNone");
}

/** Why a node waits on someone to look, in one line (PRD mission-node-simple B2): its hover, its panel and its 내 차례 item say the same. */
export function attentionReasonText(reason: MissionAttentionReason, pr: number | null, t: TFunction<"translation">): string {
  return t(`mission.attention.reason.${reason}`, { pr: prWords(pr, t) });
}

/**
 * What a node waits on someone to look at, while that can still be acted on:
 * none for a dropped node or in a cancelled mission, whose work stays as it
 * was left.
 */
export function openAttention(mission: Pick<Mission, "status">, node: Pick<MissionNode, "status" | "attention">): MissionAttention | null {
  return mission.status === "cancelled" || node.status === "dropped" ? null : node.attention;
}

/**
 * A mission waiting on the operator that no Needs You row carries (PRD
 * mission-node-simple D-19, B14): no Conductor holds it and no worker of the
 * node stands, so the mission itself is the item, its oldest such node first
 * and `more` for the rest.
 */
export type MissionAsk = { mission: Mission; node: MissionNode; attention: MissionAttention; more: number };

/**
 * The missions a device's scope names as its own Needs You items
 * (`agent_scope.mission_asks`, oldest first), each with its oldest node that
 * waits on the operator and no row carries (`on_row`). The scope counts the
 * same missions in its Needs You number, so a list drawn from this agrees
 * with the Overview and the device rail.
 */
export function missionAsks(missions: readonly Mission[], ids: readonly string[]): MissionAsk[] {
  const byId = new Map(missions.map((mission) => [mission.id, mission]));
  return ids.flatMap((id) => {
    const mission = byId.get(id);
    if (!mission) return [];
    // The core decides which raises the mission itself carries (`item`), the
    // same rule its count reads.
    const waiting = mission.nodes
      .flatMap((node) => (node.attention?.item ? [{ node, attention: node.attention }] : []))
      .sort((a, b) => a.attention.since_unix_ms - b.attention.since_unix_ms);
    const first = waiting[0];
    return first ? [{ mission, node: first.node, attention: first.attention, more: waiting.length - 1 }] : [];
  });
}

/** The asks of the missions of project `projectId` (a workspace id), or every ask when no project is chosen. */
export function projectAsks(asks: readonly MissionAsk[], projectId: string | null): MissionAsk[] {
  return projectId === null ? [...asks] : asks.filter((ask) => ask.mission.workspace_id === projectId);
}

/** A node's `!` with who looks and why: `! 나 · <reason>`, as the card's hover and accessible name say it. */
export function attentionWords(node: MissionNode, t: TFunction<"translation">): { who: string; why: string } | null {
  const attention = node.attention;
  if (!attention) return null;
  return { who: t(`mission.attention.to.${attention.to}`), why: attentionReasonText(attention.reason, attention.pr, t) };
}

/**
 * A node's warnings as its card says them (hide-private issue 552): the
 * short word the card shows, the first warning's with `+n` for the rest,
 * and each warning in full for its hover and accessible name, said as told
 * to the Conductor once its letter was queued and as the Conductor's turn to
 * be told before. Null with no
 * warning, and for a dropped node or a cancelled mission, which warn of
 * nothing.
 */
export function warningWords(mission: Mission, node: MissionNode, t: TFunction<"translation">): { short: string; full: string[] } | null {
  const warnings = mission.status === "cancelled" || node.status === "dropped" ? [] : (node.warnings ?? []);
  const first = warnings[0];
  if (!first) return null;
  const words = (warning: MissionNodeWarning, form: "short" | "full") =>
    warning.kind === "review_fails"
      ? t(`mission.warning.review_fails.${form}`, { count: node.review_fails ?? 0 })
      : t(`mission.warning.no_push.${form}`, { minutes: String(Math.floor((warning.since_unix_ms - warning.from_unix_ms) / 60_000)) });
  const more = warnings.length - 1;
  return {
    short: more > 0 ? `${words(first, "short")} +${more}` : words(first, "short"),
    full: warnings.map((warning) => t(warning.told_unix_ms != null ? "mission.warning" : "mission.warning.untold", { what: words(warning, "full") })),
  };
}

/**
 * What Hide last did to a node and why (B3): `끝 처리함 · 머지 + 기준 3/3`,
 * `깨움 · PR #12 CI 실패`, `PR #13 붙임`. The card's mark
 * says it on hover and the panel reads it; null when Hide did nothing.
 */
export function hideWords(mission: Mission, node: MissionNode, t: TFunction<"translation">): string | null {
  const mark = node.hide;
  if (!mark) return null;
  const what = hideAction(mark.action, mark.pr ?? null, t);
  if (mark.action === "woke") return mark.why ? `${what} · ${prWords(mark.pr ?? null, t)} ${t(`mission.fix.${mark.why}`)}` : what;
  if (mark.action !== "done") return what;
  const tally = criteriaTally(mission, node);
  const prs = node.prs.length > 0 ? t(node.prs.every((pr) => pr.state === "merged") ? "mission.hide.because.merged" : "mission.hide.because.ready") : null;
  const lines = tally ? t("mission.criteria.count", { passed: String(tally.passed + tally.skipped), total: String(tally.total) }) : null;
  const why = [prs, lines].filter(Boolean).join(" + ");
  return why ? `${what} · ${why}` : what;
}

/**
 * Whether the Conductor's line is behind the node's facts (B4): Hide acted
 * on the node or raised it after the line was last written, as the node's
 * events date them, or the line was written before the events kept. The
 * panel dims such a line rather than repeat it as the node's state.
 */
export function lineOutrun(node: MissionNode, events: readonly MissionEvent[] | null): boolean {
  if (!node.line) return false;
  const facts = Math.max(node.hide?.at_unix_ms ?? 0, node.attention?.since_unix_ms ?? 0, node.status === "done" && node.finished_at_unix_ms != null ? node.finished_at_unix_ms : 0);
  if (facts === 0) return false;
  const written = events?.filter((event) => event.node === node.key && event.line === node.line).at(-1)?.at_unix_ms ?? null;
  return written === null || facts > written;
}

/**
 * The operator's choices for a node that waits on someone to look, by why
 * (PRD mission-node-simple B15, B26): each is one `mission_attention`
 * event. A mission no
 * Conductor holds, or whose Conductor left a raise unanswered for ten
 * minutes (`conductorUnanswered`), offers 이어받기 on its Conductor row just
 * above.
 */
export type AttentionChoice = MissionOperatorAction;

/** The reasons only the operator settles, which reach them at once rather than after the Conductor's ten minutes (D-05, B26). */
const OPERATOR_FIRST: readonly MissionAttentionReason[] = ["conductor_failed"];

/**
 * Whether a Conductor that holds the mission let a raise reach the operator
 * (PRD mission-node-simple D-05, B15): a node's `!` the Conductor was asked
 * first now says `! 나`, so it stood ten minutes unanswered and the operator
 * may take the mission over.
 */
export function conductorUnanswered(mission: Mission): boolean {
  if (!missionOpen(mission) || !mission.conductor.present) return false;
  return mission.nodes.some((node) => {
    const attention = openAttention(mission, node);
    return attention?.to === "operator" && !OPERATOR_FIRST.includes(attention.reason);
  });
}

export function attentionChoices(reason: MissionAttentionReason): AttentionChoice[] {
  switch (reason) {
    case "merged_unproven":
      return ["done"];
    case "worker_ended":
    case "pr_closed":
    case "wake_refused":
    case "close_refused":
      return ["drop"];
    case "unplanned_pr":
      return ["attach", "leave"];
    case "conductor_failed":
      return ["resume", "drop"];
  }
}

/** Whether the operator can set a done or dropped node to work again (B16): not in a cancelled mission, which does not reopen. */
export function nodeResumable(mission: Mission, node: MissionNode): boolean {
  return mission.status !== "cancelled" && (node.status === "done" || node.status === "dropped") && node.kind !== "external";
}

/**
 * The missions whose first plan landed between two snapshots this page
 * received (PRD mission-pane-chip D-06, B12-B14): one that went from no
 * nodes to some, or first appeared with nodes. With no snapshot before, as
 * after a load or a reconnect, nothing landed.
 */
export function firstPlans(before: SnapshotRest | null | undefined, after: SnapshotRest | null | undefined): Mission[] {
  const previous = before?.missions;
  // A frame that left the missions alone keeps their section as it was.
  if (!previous || previous.failed || previous === after?.missions) return [];
  const known = new Map(previous.items.map((mission) => [mission.id, mission]));
  return (after?.missions?.items ?? []).filter((mission) => mission.nodes.length > 0 && (known.get(mission.id)?.nodes.length ?? 0) === 0);
}

/**
 * The agent a takeover starts (D-40, B44): the gone Conductor's own kind when
 * Hide can start it, else the start panel's remembered one, else the first
 * kind Hide starts.
 */
export function takeOverProvider(mission: Mission, remembered: AgentStartChoice | undefined): ProviderKind {
  const kinds = PROVIDER_KINDS as readonly string[];
  if (kinds.includes(mission.conductor.agent_kind)) return mission.conductor.agent_kind as ProviderKind;
  if (remembered?.kind && kinds.includes(remembered.kind)) return remembered.kind as ProviderKind;
  const first = PROVIDER_KINDS[0];
  if (!first) throw new Error("No agent kind Hide can start");
  return first;
}

/**
 * What 이어받기 resumes first (PRD mission-tool D-11, B14): the gone
 * Conductor's own conversation, when its agent can resume one; null when it
 * has none to resume, and 이어받기 starts a new agent instead (B15). A
 * Conductor still there that left a raise unanswered keeps its conversation
 * in its own pane, so a takeover of it starts a new agent (PRD
 * mission-node-simple B15).
 */
export function conductorResume(mission: Mission): { provider: ProviderKind; sessionId: string } | null {
  if (mission.conductor.present) return null;
  const session = mission.conductor.session;
  const provider = session ? resumeProvider(session.agent) : null;
  return session && provider ? { provider, sessionId: session.session_id } : null;
}

/**
 * The first prompt of a takeover: the guide and the command that make the new
 * pane the mission's Conductor. It names the mission by id only: the title can
 * come from an issue someone else wrote, and the new agent reads it with
 * `hide mission show` as the record's data rather than as its instructions.
 */
export function takeOverPrompt(mission: Mission, handover?: string): string {
  const take = handover ? `hide mission take ${mission.id} --handover ${handover}` : `hide mission take ${mission.id}`;
  return `Read \`hide mission help\` and follow it as this mission's Conductor: take over mission ${mission.id}. Run \`${take}\`, then \`hide mission show\` and \`hide inbox\`, read each worker's state, and go on from there.`;
}

/**
 * The Conductor row's state (PRD mission-tool B12, B13): `gone` for an open
 * mission no Conductor holds, the decision it waits on, else what its pane's
 * row shows (asleep, at work, resting). `unknown` while the core holds the
 * Conductor but its row is not listed, so no state word is made up; null for
 * a closed mission with no Conductor row to bring forward.
 */
export type ConductorState = "working" | "deciding" | "resting" | "asleep" | "gone" | "unknown";

export function conductorState(mission: Mission, row: AgentRow | null): ConductorState | null {
  const open = missionOpen(mission);
  if (open && !mission.conductor.present) return "gone";
  if (!row) return open ? (missionDeciding(mission) ? "deciding" : "unknown") : null;
  if (missionDeciding(mission)) return "deciding";
  if (row.mark === "asleep") return "asleep";
  if (row.mark === "going") return "working";
  return "resting";
}

/** The mission's own pull requests with the node that holds each, in order, a node's several in the order it got them (B18, B20): an outside node's pull request is not the mission's. */
export function missionPulls(mission: Mission): { node: MissionNode; pr: MissionNodePr }[] {
  return orderedNodes(mission).flatMap((node) => (node.kind !== "external" && node.status !== "dropped" ? node.prs.map((pr) => ({ node, pr })) : []));
}

/**
 * A finished mission's result tiles (B20), each counted from the record and
 * null when the record holds nothing for it: merged pull requests of those
 * it made (none while no state was read), finish lines met of all of them
 * with the skipped ones, the agents bound to its work and its checks, and
 * its changes in main of those it planned.
 */
export type MissionResultCounts = {
  merged: { merged: number; total: number } | null;
  criteria: { passed: number; total: number; skipped: number } | null;
  agents: { workers: number; checkers: number } | null;
  landed: { landed: number; total: number } | null;
};

export function missionResultCounts(mission: Mission): MissionResultCounts {
  const kept = mission.nodes.filter((node) => node.status !== "dropped");
  const pulls = missionPulls(mission);
  const read = pulls.filter(({ pr }) => pr.state != null);
  const lines = mission.exempt ? [] : kept;
  const total = lines.reduce((sum, node) => sum + node.criteria.length, 0);
  const skipped = lines.reduce((sum, node) => sum + node.criteria_skipped, 0);
  const passed = lines.reduce((sum, node) => sum + node.criteria_met, 0) - skipped;
  const workers = kept.filter((node) => node.kind !== "external" && node.agent_id != null).length;
  const checkers = kept.reduce((sum, node) => sum + missionCheckers(node).filter((checker) => checker.agent_id != null).length, 0);
  const changes = kept.filter((node) => node.kind === "change");
  return {
    merged: read.length > 0 ? { merged: read.filter(({ pr }) => pr.state === "merged").length, total: pulls.length } : null,
    criteria: total > 0 ? { passed, total, skipped } : null,
    agents: workers + checkers > 0 ? { workers, checkers } : null,
    landed: changes.length > 0 ? { landed: changes.filter(nodeLanded).length, total: changes.length } : null,
  };
}

/** A node's other agents checking it, review and verify lines in order (B17). */
export function missionCheckers(node: MissionNode): MissionChecker[] {
  return node.criteria.flatMap((line) => (line.kind === "review" || line.kind === "verify" ? (line.checkers ?? []) : []));
}

/**
 * The report's folded line (B21): its first line of words without Markdown
 * marks, the next one when the first only repeats the mission's title; null
 * for a report with no words.
 */
export function reportLine(plain: string, title: string): string | null {
  const lines = plain.split("\n").map((one) => one.trim()).filter(Boolean);
  const [first, second] = lines;
  if (first === undefined) return null;
  return first === title.trim() && second !== undefined ? second : first;
}

/** The nodes the core says a planned node still waits for (`waiting_on`, B28), which its line names by pull request or title. */
export function waitingFor(mission: Mission, node: MissionNode): MissionNode[] {
  if (node.waiting_on?.kind !== "nodes") return [];
  return node.waiting_on.keys.flatMap((key) => mission.nodes.find((other) => other.key === key) ?? []);
}
