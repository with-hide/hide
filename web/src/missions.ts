// What the mission map and the Conductor row read from a mission (PRD
// software-factory-conductor): the core decides every state and number
// (`MissionView`), so this module only finds a mission, words its parts
// and counts what the record holds. Pure, so the rules test on their own;
// `MissionMap.tsx` draws them.

import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { PROVIDER_KINDS, type ProviderKind } from "./agentPicker";
import { agentsByPane } from "./paneAgents";
import { orderByDependencies } from "./projectBoard";
import type { AgentRow, AgentStartChoice, Mission, MissionChecker, MissionCriterion, MissionWorker, MissionEvent, MissionEventsSnapshot, MissionNode, MissionVerdict, RowMark, SnapshotRest } from "./snapshot";

export function missionOf(rest: SnapshotRest | null | undefined, id: string): Mission | null {
  return rest?.missions?.items.find((mission) => mission.id === id) ?? null;
}

/** Whether a mission is still the Conductor's to carry: planned, running or waiting on a merge. */
export function missionOpen(mission: Mission): boolean {
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
export function missionCounts(mission: Mission): { done: number; total: number; dropped: number } {
  const dropped = mission.nodes.filter((node) => node.status === "dropped").length;
  return { done: mission.nodes.filter((node) => node.status === "done").length, total: mission.nodes.length - dropped, dropped };
}

/** The pull requests of a stack still waiting on a lower one: open, with a base the mission holds. */
export function stackWaiting(mission: Mission): number {
  return mission.nodes.filter((node) => node.base_pr != null && node.pr_state !== "merged" && node.pr_state !== "closed" && node.status !== "dropped").length;
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
 * The card's last line (B15, B51, B53): why it waits on a failed or dropped
 * node, else what its worker last said, else its kind. A node that waits
 * on a node not done yet says nothing of it: the edge above it does.
 */
export function nodeLine(node: MissionNode, t: TFunction<"translation">): string {
  if (node.blocked === "upstream_failed") return t("mission.blocked.failed");
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
export function runWords(launch: LaunchSession | null, plan: MissionWorker | null | undefined, words: { auto: string; defaultModel: string }): RunWords | null {
  if (launch && (launch.launch_model || launch.launch_effort)) return { words: [launch.launch_model ?? words.defaultModel, launch.launch_effort].filter(Boolean).join("·"), planned: false };
  if (!plan) return null;
  const effort = plan.effort ?? (plan.effort_auto ? words.auto : null);
  const planned = [plan.model, effort].filter(Boolean).join("·");
  return planned ? { words: planned, planned: true } : null;
}

/** A change node's steps: work, PR or branch, review, merge or main (D-42 B, D-48). */
export const STAGE_STEPS = 4;

/**
 * The card's status mark (B15): its worker's own mark while its row is
 * listed, so a node reads as its agent does everywhere; else the node's
 * state in the same four shapes, a node that waits on another dashed.
 */
export function nodeMark(node: MissionNode, worker: RowMark | null): RowMark {
  if (worker && (node.status === "working" || node.status === "review")) return worker;
  if (node.blocked) return "waiting";
  switch (node.status) {
    case "working":
    case "review":
      return "going";
    case "waiting":
    case "held":
      return "waiting";
    case "failed":
      return "turn";
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

/** A mission holds at most one decision (`decision`), so the chip's count is always one. */
export function paneChipWords(mission: Mission): { state: PaneChipState; key: MessageKey; counts: string | null } {
  const { done, total } = missionCounts(mission);
  const counts = `${done}/${total}`;
  if (mission.decision && missionOpen(mission)) return { state: "decision", key: "mission.pane.decision", counts: null };
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
 * mission-plan-visibility D-11, B8). A node that cannot start yet waits; once
 * the mission is cancelled, a node it never finished is cancelled, and a
 * dropped node left the plan, so it has no cell.
 */
export type NodeTone = "done" | "working" | "review" | "hold" | "failed" | "waiting" | "cancelled" | "dropped";

export function nodeTone(mission: Mission, node: MissionNode): NodeTone {
  if (node.status === "dropped") return "dropped";
  if (mission.status === "cancelled" && node.status !== "done" && node.status !== "failed") return "cancelled";
  if (node.blocked) return "waiting";
  switch (node.status) {
    case "done":
      return "done";
    case "working":
      return "working";
    case "review":
      return "review";
    case "held":
      return "hold";
    case "failed":
      return "failed";
    case "planned":
    case "waiting":
      return "waiting";
  }
}

/** A cell's state on the head's and the chip's bar (B8, B24): what finished, what moves, what waits on the operator, what is still ahead. */
export type CellTone = Exclude<NodeTone, "dropped">;

/** The nodes top down by what they wait on, a dropped node where it stood (B17). */
export function orderedNodes(mission: Mission): MissionNode[] {
  return orderByDependencies(mission.nodes, (node) => node.key, (node) => node.depends_on).map(({ item }) => item);
}

/** The chip's and the head's bar (B8, B24): one cell per node in the plan, in order. */
export function missionCells(mission: Mission): CellTone[] {
  return orderedNodes(mission).flatMap((node) => {
    const tone = nodeTone(mission, node);
    return tone === "dropped" ? [] : [tone];
  });
}

/** A head cell per node in the plan order, drawn as its card is (B26): dropped nodes have none. */
export type HeadCell = Exclude<NodeLook, "dropped">;
export function missionHeadCells(mission: Mission): HeadCell[] {
  return orderedNodes(mission).flatMap((node) => {
    const look = nodeLook(mission, node);
    return look === "dropped" ? [] : [look];
  });
}

/** The colour a mission's own chip wears on the sidebar row and the Overview (B35): its decision, else its state. */
export function missionTone(mission: Mission): CellTone {
  if (mission.decision && missionOpen(mission)) return "hold";
  switch (mission.status) {
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

/** The nodes a worker is at now, working or in review, in order (B9, B10). */
export function liveNodes(mission: Mission): MissionNode[] {
  return orderedNodes(mission).filter((node) => node.status === "working" || node.status === "review");
}

/** The running popover's last line (B10): the nodes done, and those still ahead of any worker. */
export function missionTally(mission: Mission): { done: number; next: number } {
  const ahead = (node: MissionNode) => node.status === "planned" || node.status === "waiting" || node.status === "held";
  return { done: mission.nodes.filter((node) => node.status === "done").length, next: mission.nodes.filter(ahead).length };
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
export function stageWord(node: MissionNode): MessageKey {
  if (node.kind === "research") return "mission.stage.research";
  if (node.kind === "verify") return "mission.stage.verify";
  const stages = node.delivery === "direct" ? DIRECT_STAGES : PR_STAGES;
  return stages[Math.min(Math.max(node.stage ?? 1, 1), STAGE_STEPS) - 1]!;
}

/**
 * What a node left behind, by its kind (B22, D-14): its pull request, a
 * direct change's commit on main or else its branch, a research node's
 * conclusion, a verify node's recorded verdict or else its line. A research
 * or verify node's line is a result only once the node ended (done, or a
 * failed check); while it runs the line is its progress. Null when the
 * record holds none, so nothing is drawn for it.
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
  return node.status === "done" || node.status === "failed" ? { kind: "verify", line: node.line } : null;
}

/**
 * The commit a change has in main (B17): the one Hide read there for the
 * node's branch, else one the Conductor recorded for a direct change by hand.
 */
export function mainCommit(node: MissionNode): string | null {
  return node.in_main ?? node.commit;
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
 * How a card is drawn (PRD mission-map-git-criteria D-02, B26): its icon's
 * shape says the state, and colour is kept for what wants a look (working,
 * review, a decision, a failure). Finished work is grey, and once it reached
 * main it is `landed`, a faint check with no fill; an outside node watches
 * until its pull request merges or its branch reaches main. Being outside is
 * a node's kind, not a state: its card keeps the dotted border and `↗` in
 * every look (B19).
 */
export type NodeLook = "waiting" | "working" | "review" | "hold" | "failed" | "done" | "landed" | "external" | "cancelled" | "dropped";

export function nodeLook(mission: Mission, node: MissionNode): NodeLook {
  if (node.status === "dropped") return "dropped";
  if (node.kind === "external") {
    if (node.status === "done") return "landed";
    if (node.status === "failed") return "failed";
    return mission.status === "cancelled" ? "cancelled" : "external";
  }
  const tone = nodeTone(mission, node);
  if (tone === "done") return nodeLanded(node) ? "landed" : "done";
  return tone;
}

/** A finished change is in main: its pull request merged or its commit found or recorded there; an outside node is done only once it reached main (B17, B19). */
export function nodeLanded(node: MissionNode): boolean {
  if (node.status !== "done") return false;
  return node.kind === "external" || node.pr_state === "merged" || mainCommit(node) != null;
}

/**
 * What a finished change still waits on before main (B27), only from what
 * the core read: a pull request it read open, or a done direct change's
 * branch, which it reads while the mission runs or waits on its merges, not
 * found in main.
 * None for a pull request whose state was never read (a mission recorded
 * before the core kept one), a branch it cannot read, work that reached main,
 * research, a verify node or a cancelled mission.
 */
export function landingLeft(mission: Mission, node: MissionNode): "merge" | "join" | null {
  if (node.kind !== "change" || node.status !== "done" || nodeLanded(node) || mission.status === "cancelled") return null;
  if (node.pr != null) return node.pr_state != null && node.pr_state !== "closed" ? "merge" : null;
  const read = mission.status === "running" || mission.status === "merge_wait";
  return read && node.delivery === "direct" && node.branch != null && !node.unread ? "join" : null;
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
 * A node's times from its events (B28): how long it took, from the first
 * time it went to work to when it ended, or to `end` while it has not; and
 * how long decisions held it, every held stretch added. Null with no event
 * that moved it, as in a mission recorded before its events, so no number is
 * drawn for it.
 */
export function nodeTimes(node: MissionNode, events: readonly MissionEvent[], end: number): { took: number | null; held: number; running: boolean } | null {
  const moves = events.filter((event) => event.kind === "node" && event.node === node.key && event.status);
  if (moves.length === 0) return null;
  const start = moves.find((move) => move.status === "working" || move.status === "review");
  const last = moves[moves.length - 1]!;
  const running = last.status !== "done" && last.status !== "failed" && last.status !== "dropped";
  const stop = running ? end : last.at_unix_ms;
  let held = 0;
  moves.forEach((move, index) => {
    if (move.status === "held") held += Math.max(0, (moves[index + 1]?.at_unix_ms ?? stop) - move.at_unix_ms);
  });
  return { took: start ? Math.max(0, stop - start.at_unix_ms) : null, held, running };
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
 * The card's one line under its title (B28): where a working node is (its
 * worker's plan, the Conductor's line, else what is left), what a held,
 * failed or blocked node waits on, what a planned node waits for, and what
 * an outside node watches; nothing for finished work, whose result speaks.
 */
export function cardLine(mission: Mission, node: MissionNode, planLine: string | null, t: TFunction<"translation">): { text: string; attention: boolean } | null {
  const look = nodeLook(mission, node);
  if (look === "cancelled" || look === "dropped") return null;
  // An outside node says where its pull request or branch is in every state, finished too (B19).
  if (node.kind === "external") {
    if (look === "failed") return { text: t("mission.outside.closed"), attention: true };
    if (look === "landed") return { text: t("mission.look.landed"), attention: false };
    return { text: t(node.unread ? "mission.outside.unread" : "mission.outside.waiting"), attention: false };
  }
  if (look === "done" || look === "landed") return null;
  if (look === "working" || look === "review") {
    const left = linesLeft(node);
    const text = planLine ?? node.line ?? (left.length > 0 ? t("mission.line.left", { lines: left.map((line) => criterionShort(line, t)).join(", ") }) : null);
    return text ? { text, attention: false } : null;
  }
  if (look === "hold") return { text: t("mission.line.held"), attention: false };
  if (look === "failed") return { text: node.line ?? t("mission.status.failed"), attention: true };
  if (node.blocked) return { text: nodeLine(node, t), attention: true };
  const before = node.depends_on.flatMap((key) => {
    const dep = mission.nodes.find((other) => other.key === key);
    return dep && dep.status !== "done" && dep.status !== "dropped" ? [dep.pr != null ? `#${dep.pr}` : dep.title] : [];
  });
  if (before.length > 0) return { text: t("mission.line.waitsFor", { names: before.join(" ") }), attention: false };
  return node.line ? { text: node.line, attention: false } : null;
}

/**
 * The changes left to merge or join, in the order the Conductor gave (B18,
 * B25): the plan's merge order first, then any other finished change not in
 * main yet, top down.
 */
export function landingOrder(mission: Mission): MissionNode[] {
  const left = orderedNodes(mission).filter((node) => landingLeft(mission, node) !== null);
  const at = (node: MissionNode) => {
    const index = mission.merge_order.indexOf(node.key);
    return index === -1 ? mission.merge_order.length : index;
  };
  return left.map((node, index) => ({ node, index })).sort((a, b) => at(a.node) - at(b.node) || a.index - b.index).map(({ node }) => node);
}

/** Why a worker's row was raised to the operator (`AgentRow.escalation`, docs/status-model.md, Delegated escalation). */
export type EscalationCause = NonNullable<AgentRow["escalation"]>["cause"];

/**
 * What only the operator can do now (B25): answer the decision, take over a
 * mission no Conductor holds, clear a worker the core raised past its parent
 * (one of the six escalation causes), see an outside pull request closed
 * without its merge, and merge or join what waits for main. A worker's own
 * block its parent can handle stays on its card, and a finished or cancelled
 * mission has nothing here.
 */
export type TurnItem =
  | { kind: "decision" }
  | { kind: "takeover" }
  | { kind: "worker_blocked"; node: MissionNode; cause: EscalationCause; line: string | null }
  | { kind: "outside_closed"; node: MissionNode }
  | { kind: "landing"; nodes: MissionNode[]; git: boolean };

export function turnItems(mission: Mission, workerRow: (node: MissionNode) => AgentRow | null): TurnItem[] {
  if (!missionOpen(mission)) return [];
  const items: TurnItem[] = [];
  if (mission.decision) items.push({ kind: "decision" });
  else if (!mission.conductor.present) items.push({ kind: "takeover" });
  for (const node of orderedNodes(mission)) {
    const escalation = node.status === "working" || node.status === "review" ? workerRow(node)?.escalation : null;
    if (escalation) {
      const request = workerRow(node)?.request;
      items.push({ kind: "worker_blocked", node, cause: escalation.cause, line: request?.end === "blocked" && request.line ? request.line : null });
    }
  }
  for (const node of orderedNodes(mission)) if (node.kind === "external" && node.status === "failed") items.push({ kind: "outside_closed", node });
  if (mission.status === "merge_wait") {
    const nodes = landingOrder(mission);
    if (nodes.length > 0) items.push({ kind: "landing", nodes, git: nodes.every((node) => node.pr == null) });
  }
  return items;
}

/** The events the core sends for `missionId`, which it does only for the mission open in the 보기 slot (D-22). */
export function missionEventsOf(rest: SnapshotRest | null | undefined, missionId: string): MissionEventsSnapshot | null {
  const events = rest?.mission_events;
  return events && events.mission === missionId ? events : null;
}

/** An event in words (B20, B31): what happened, and the one line it carried. */
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
  }
}

/**
 * The question a held node's `!` carries (B16, B25): the mission's decision
 * on the node it names, or on every held node when it names none, as a
 * decision from before `--node` did.
 */
export function heldQuestion(mission: Mission, node: MissionNode): string | null {
  if (node.status !== "held" || nodeTone(mission, node) === "cancelled" || !mission.decision) return null;
  const named = mission.question?.node;
  return named == null || named === node.key ? mission.decision : null;
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
 * The first prompt of a takeover: the guide and the command that make the new
 * pane the mission's Conductor. It names the mission by id only: the title can
 * come from an issue someone else wrote, and the new agent reads it with
 * `hide mission show` as the record's data rather than as its instructions.
 */
export function takeOverPrompt(mission: Mission): string {
  return `Read \`hide mission help\` and follow it as this mission's Conductor: take over mission ${mission.id}. Run \`hide mission take ${mission.id}\`, then \`hide mission show\` and \`hide inbox\`, read each worker's state, and go on from there.`;
}
