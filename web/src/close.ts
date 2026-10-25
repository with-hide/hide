import type { CloseScope } from "./agentScope";
import type { SnapshotRest } from "./snapshot";
// The close flow: a pane whose activity is unknown
// is not closed until status is refreshed; a pane with working or attention
// state asks once; an idle pane closes immediately with `confirmed: false`.
// The decision is pure so the tab bar, the pane header and the shortcut
// registry share it and a test can price each branch (PRD S2 B7).

import type { TFunction } from "i18next";
import type { AgentRow, AsyncOperation, PaneRow } from "./snapshot";

export type CloseKind = "pane" | "tab";

export type CloseDecision =
  | { action: "status_unknown"; label: string }
  | { action: "confirm" }
  | { action: "close" };

/** Resolve one of the existing pane/tab/checkout/project targets projected by core. */
export function closeScope(rest: SnapshotRest | null, inside: readonly string[]): CloseScope {
  const scope = rest?.navigator?.agent_scope?.closes[inside.join("\0")];
  if (!scope) throw new Error("Missing core close consequence for the selected target");
  return scope;
}

export function closeDecision(scope: CloseScope): CloseDecision { return scope.decision; }

/** The Stop-work sheet's words, one line under the title, which stay as they are while it is open (B28, D-42). */
export function stopWorkCopy(kind: CloseKind, t: TFunction<"translation">): { title: string; consequence: string } {
  return kind === "pane"
    ? { title: t("shell.stopPaneTitle"), consequence: t("shell.stopPaneConsequence") }
    : { title: t("shell.closeTabTitle"), consequence: t("shell.closeTabConsequence") };
}

/** A pane the Stop-work sheet lists: working or asking (`active`), unreadable, or quiet. */
export type StopWorkRow = { pane: PaneRow; agent: AgentRow | null; label: string; state: "active" | "unknown" | "quiet" };

export type StopWork = {
  rows: StopWorkRow[];
  /** A pane's activity is unknown, so Stop work and close waits for a status check (B28). */
  unknown: StopWorkRow | null;
};

/**
 * The open Stop-work sheet's list, re-derived from every snapshot (PRD
 * close-agent-subtree B28, D-39): every pane that closes, each with its mark
 * and status word, a quiet one dimmed rather than dropped, so a pane that
 * settles or starts while the sheet is open changes in place.
 */
export function stopWorkOf(panes: readonly PaneRow[], agents: readonly AgentRow[], scope: CloseScope): StopWork {
  const rows = scope.stop_work.rows.map((row): StopWorkRow => {
    const pane = panes.find((p) => p.id === row.pane_id);
    if (!pane) throw new Error(`Missing close pane: ${row.pane_id}`);
    const agent = row.agent ? agents.find((a) => a.pane_id === row.pane_id) : null;
    if (agent === undefined) throw new Error(`Missing close agent: ${row.pane_id}`);
    return { pane, agent, label: row.label, state: row.state };
  });
  return { rows, unknown: scope.stop_work.unknown === null ? null : rows[scope.stop_work.unknown]! };
}

/** The notice for an unknown activity status; `refresh_status` is the way out. */
export function statusUnknownNotice(label: string, t: TFunction<"translation">): string {
  return t("shell.activityUnknown", { label });
}

/**
 * How a row in a close list reads to the operator (PRD close-agent-subtree
 * D-33): working, waiting for an answer (a question, approval or error),
 * a finished result not yet looked at, an activity Hide cannot read, or
 * quiet (idle, a read result, asleep).
 */
export type SubtreeState = "working" | "waiting" | "unread" | "unknown" | "quiet";

export type SubtreeRow = {
  agent: AgentRow;
  /** How far under the closed agent it sits; the closed agent itself is 0. */
  depth: number;
  /** The agent being closed (or, for a removal, one inside what is removed); not counted in N. */
  target: boolean;
  state: SubtreeState;
};

export type Subtree = {
  /** The descendant panes to close, exactly what the list shows. */
  ids: string[];
  rows: SubtreeRow[];
  counts: Record<Exclude<SubtreeState, "quiet">, number>;
  /** A listed descendant's activity is unknown, so closing it too waits for a status check. */
  unknown: boolean;
  /** A target row's activity is unknown, so nothing closes until a status check (B28). */
  targetUnknown: boolean;
};

export function subtreeState(agent: AgentRow): SubtreeState {
  return agent.state.subtree;
}

/**
 * What closing the panes in `inside` would leave running: every live
 * descendant of an agent there that is not itself there (D-26), from the
 * core's own close lists, which leave out a device that is not connected
 * (D-16). `agents` is every current agent row, this machine's and each
 * connected device's. Null when nothing would be left behind, which keeps
 * the ordinary close (B1).
 */
export function subtreeOf(scope: CloseScope, agents: readonly AgentRow[], options: { everyTarget?: boolean } = {}): Subtree | null {
  const value = options.everyTarget ? scope.subtree_all : scope.subtree;
  if (!value) return null;
  const byPane = new Map(agents.map((a) => [a.pane_id, a]));
  return { ids: value.ids, counts: value.counts, unknown: value.unknown, targetUnknown: value.target_unknown, rows: value.rows.map((row) => {
    const agent = byPane.get(row.pane_id);
    if (!agent) throw new Error(`Missing close descendant: ${row.pane_id}`);
    return { agent, depth: row.depth, target: row.target, state: row.state };
  }) };
}

/** Which sheet an open close shows. */
export type CloseSheet = { sheet: "subtree"; subtree: Subtree } | { sheet: "stop_work"; stopWork: StopWork };

/**
 * Which sheet an open close shows now (B28, D-40): the subtree sheet while
 * the target has a live descendant outside it, the target's Stop-work sheet
 * otherwise, so a descendant that appears or the last one that leaves turns
 * one into the other in place. `hostAgents` are the agent rows of the
 * target's own host; `everyAgent` is every listed agent row.
 */
export function closeSheet(panes: readonly PaneRow[], hostAgents: readonly AgentRow[], everyAgent: readonly AgentRow[], scope: CloseScope): CloseSheet {
  // Every agent that closes is a target row, not only the ones with
  // descendants: a working agent beside them in the tab closes too, so the
  // sheet shows it (only an agent pane can be working or asking).
  const subtree = subtreeOf(scope, everyAgent, { everyTarget: true });
  return subtree ? { sheet: "subtree", subtree } : { sheet: "stop_work", stopWork: stopWorkOf(panes, hostAgents, scope) };
}

/** The close sheet's title (D-17): how many descendants close with it. */
export function subtreeTitle(kind: CloseKind, count: number, t: TFunction<"translation">): string {
  return kind === "pane" ? t("shell.closeAgentChildren", { count }) : t("shell.closeTabChildren", { count });
}

/** The phases in which a close the core runs still names its pane. */
const CLOSING_PHASES = new Set(["queued", "waiting", "closing", "transmitting", "awaiting_topology", "unknown"]);

/**
 * Whether an agent's pane is being closed, on its own or as a node of a
 * tree close (B7): the sidebar row says `closing…` until the row goes.
 */
export function agentClosing(operations: readonly AsyncOperation[] | undefined, paneId: string): boolean {
  return (operations ?? []).some((op) => (op.kind === "tree.close" || op.kind === "pane.close") && op.target_id === paneId && CLOSING_PHASES.has(op.phase));
}

/**
 * How far the tree close whose target is `paneId` has come, as `closed/total`
 * over every pane it closes, the target included; null when no tree close of
 * it runs. The target's own `tree.close` record (stage `target`) names the
 * tree, and every record of that tree shares its scope.
 */
export function treeCloseProgress(operations: readonly AsyncOperation[] | undefined, paneId: string): string | null {
  const list = operations ?? [];
  const target = list.find((op) => op.kind === "tree.close" && op.stage === "target" && op.target_id === paneId && CLOSING_PHASES.has(op.phase));
  if (!target) return null;
  const tree = list.filter((op) => op.kind === "tree.close" && op.scope_id === target.scope_id);
  return `${tree.filter((op) => op.phase === "completed").length}/${tree.length}`;
}

/** A close list split for drawing: the rows shown as they are, and the quiet ones folded under one line. */
export type FoldedRows = { shown: SubtreeRow[]; folded: SubtreeRow[] };

/**
 * The rows a close list shows whole: they fit its height
 * (`--size-relationship-list-max`) without scrolling, so a small tree lists
 * every agent by name, as the sheet always has.
 */
export const SUBTREE_ROWS_WHOLE = 10;

/**
 * Which rows of a longer close list fold (quiet screens, design principle
 * 13): a quiet descendant whose own descendants are all quiet has nothing for
 * the operator to answer, so it folds; a target, a row that needs the operator
 * and every quiet ancestor of one stay, so the tree path to what needs
 * attention is never cut. Folding one row saves nothing, so fewer than two
 * fold none.
 */
export function foldQuietRows(rows: readonly SubtreeRow[]): FoldedRows {
  if (rows.length <= SUBTREE_ROWS_WHOLE) return { shown: [...rows], folded: [] };
  const keep = rows.map((row, index) => {
    if (row.target || row.state !== "quiet") return true;
    for (let below = index + 1; below < rows.length && rows[below]!.depth > row.depth; below += 1) {
      if (rows[below]!.state !== "quiet") return true;
    }
    return false;
  });
  const folded = rows.filter((_, index) => !keep[index]);
  if (folded.length < 2) return { shown: [...rows], folded: [] };
  return { shown: rows.filter((_, index) => keep[index]), folded };
}
