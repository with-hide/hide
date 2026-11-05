// The sidebar's drawn agent tree (PRD agent-hierarchy-screens D-12, D-28,
// D-38; B10 to B12): the core flattens the opened roots, children and
// grandchildren in order; this keeps the core's number of siblings per parent until the
// operator asks for the rest, and says where each row's rails run.
import type { BoardRow } from "./projectBoard";
import { treePlaces, type TreePlace } from "./components/agent-tree";

/** How many siblings a tree shows before "N more" (B12): the core's number, carried on the snapshot's navigator. */
export function siblingsShown(rest: { navigator?: { siblings_shown?: number } } | null | undefined): number {
  const shown = rest?.navigator?.siblings_shown;
  if (shown === undefined) throw new Error("The snapshot names no sibling cap");
  return shown;
}

export type SidebarTreeLine =
  | { kind: "agent"; row: BoardRow; place: TreePlace }
  | { kind: "more"; parent: string; depth: number; count: number; place: TreePlace };

/**
 * Past `siblings` children of one parent the rest, with their subtrees, wait
 * behind one "N more" line at their depth, until `shownAll` names the parent.
 * Roots are never limited: the checkout row above folds them. `place` draws
 * the rails; Sessions passes one that stops indenting (`cappedPlaces`).
 */
export function sidebarTreeLines(rows: readonly BoardRow[], shownAll: ReadonlySet<string>, siblings: number, place: (depths: readonly number[]) => TreePlace[] = treePlaces): SidebarTreeLine[] {
  type Pending = Omit<SidebarTreeLine, "place"> & ({ kind: "agent"; row: BoardRow } | { kind: "more"; parent: string; depth: number; count: number });
  const out: Pending[] = [];
  // Per depth: the parent above, how many of its children were seen, and the open "more" line.
  const parents: { pane: string; seen: number; more: Extract<Pending, { kind: "more" }> | null }[] = [];
  let skipBelow: number | null = null;
  for (const row of rows) {
    if (skipBelow !== null && row.depth > skipBelow) continue;
    skipBelow = null;
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
  }
  const places = place(out.map((line) => (line.kind === "agent" ? line.row.depth : line.depth)));
  return out.map((line, index) => ({ ...line, place: places[index]! }) as SidebarTreeLine);
}
