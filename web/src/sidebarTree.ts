// The sidebar's drawn agent tree (PRD agent-hierarchy-screens D-12, D-28,
// D-38; B10 to B12): the core flattens the opened roots, children and
// grandchildren in order; this keeps the core's number of siblings per parent until the
// operator asks for the rest, and says where each row's rails run.
import type { BoardRow } from "./projectBoard";
import { treePlaces, type TreePlace } from "./components/agent-tree";
import { subagentsOf } from "./components/subagent-row";
import type { AgentRow, SubagentRow } from "./snapshot";

/** How many siblings a tree shows before "N more" (B12): the core's number, carried on the snapshot's navigator. */
export function siblingsShown(rest: { navigator?: { siblings_shown?: number } } | null | undefined): number {
  const shown = rest?.navigator?.siblings_shown;
  if (shown === undefined) throw new Error("The snapshot names no sibling cap");
  return shown;
}

type LineBody =
  | { kind: "agent"; row: BoardRow }
  | { kind: "more"; parent: string; depth: number; count: number }
  | { kind: "subagent"; parent: string; depth: number; subagent: SubagentRow }
  | { kind: "subagents-more"; parent: string; depth: number; count: number; listed: number };

export type SidebarTreeLine = LineBody & { place: TreePlace };

/**
 * The lines an opened row's own subagents add after its children (docs/UI_BEHAVIOR.md, Subagents): the ones the
 * core lists, as many as the sibling cap, then one line for the rest. They have no pane, so no line of theirs opens
 * anything, and none counts against the row's children.
 */
function subagentLines(agent: AgentRow, depth: number, siblings: number): Extract<LineBody, { kind: "subagent" | "subagents-more" }>[] {
  const { rows } = subagentsOf(agent);
  const shown = rows.slice(0, siblings);
  const rest = (agent.subagents ?? 0) - shown.length;
  return [
    ...shown.map((subagent) => ({ kind: "subagent" as const, parent: agent.pane_id, depth, subagent })),
    ...(rest > 0 ? [{ kind: "subagents-more" as const, parent: agent.pane_id, depth, count: rest, listed: shown.length }] : []),
  ];
}

/**
 * Past `siblings` children of one parent the rest, with their subtrees, wait
 * behind one "N more" line at their depth, until `shownAll` names the parent.
 * Roots are never limited: the checkout row above folds them. `place` draws
 * the rails; Sessions passes one that stops indenting (`cappedPlaces`). A row
 * `opened` says is open lists its own subagents after its children.
 */
export function sidebarTreeLines(rows: readonly BoardRow[], shownAll: ReadonlySet<string>, siblings: number, place: (depths: readonly number[]) => TreePlace[] = treePlaces, opened: (row: BoardRow) => boolean = () => false): SidebarTreeLine[] {
  type Pending = LineBody;
  const out: Pending[] = [];
  // Per depth: the parent above, how many of its children were seen, and the open "more" line.
  const parents: { pane: string; seen: number; more: Extract<Pending, { kind: "more" }> | null }[] = [];
  // Opened rows whose subagents follow their subtree, deepest last.
  const open: BoardRow[] = [];
  const close = (depth: number) => {
    while (open.length > 0 && open.at(-1)!.depth >= depth) {
      const row = open.pop()!;
      out.push(...subagentLines(row.agent, row.depth + 1, siblings));
    }
  };
  let skipBelow: number | null = null;
  for (const row of rows) {
    if (skipBelow !== null && row.depth > skipBelow) continue;
    skipBelow = null;
    close(row.depth);
    parents.length = row.depth;
    const parent = row.depth > 0 ? parents[row.depth - 1] : undefined;
    if (parent) {
      parent.seen += 1;
      if (parent.seen > siblings && !shownAll.has(parent.pane)) {
        if (parent.more) parent.more.count += 1;
        else {
          parent.more = { kind: "more", parent: parent.pane, depth: row.depth, count: 1 };
          out.push(parent.more);
        }
        skipBelow = row.depth;
        parents[row.depth] = { pane: row.agent.pane_id, seen: 0, more: null };
        continue;
      }
    }
    out.push({ kind: "agent", row });
    parents[row.depth] = { pane: row.agent.pane_id, seen: 0, more: null };
    if ((row.agent.subagents ?? 0) > 0 && opened(row)) open.push(row);
  }
  close(0);
  const places = place(out.map((line) => (line.kind === "agent" ? line.row.depth : line.depth)));
  return out.map((line, index) => ({ ...line, place: places[index]! }) as SidebarTreeLine);
}
