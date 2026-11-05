// The Sessions tree under one root (docs/UI_BEHAVIOR.md, Sessions): every
// opened row shows its children at any depth, the sidebar's number of siblings
// at a time, and the indent stops at SESSION_INDENT_LEVELS so a narrow Tools
// column keeps room for titles; deeper rows hang from their parent's rail.
import { cappedPlaces } from "./components/agent-tree";
import { childrenOf } from "./components/agent-tree-popover";
import type { BoardRow } from "./projectBoard";
import { sidebarTreeLines, type SidebarTreeLine } from "./sidebarTree";
import { catalogWorkspaces, type AgentRow, type SnapshotRest } from "./snapshot";

/** How many levels below a root step right before the rest share one column. */
export const SESSION_INDENT_LEVELS = 3;

/**
 * The root and, through every row the operator opened, its descendants in
 * display order (most urgent sibling first), with the sidebar's sibling limit
 * and the capped rails. A root that is not opened draws only itself.
 */
export function sessionTreeLines(root: AgentRow, byPane: ReadonlyMap<string, AgentRow>, opened: ReadonlySet<string>, shownAll: ReadonlySet<string>, siblings: number): SidebarTreeLine[] {
  const rows: BoardRow[] = [];
  const seen = new Set<string>();
  const walk = (agent: AgentRow, depth: number) => {
    // Lineage is acyclic in the core; a repeated pane would still stop here.
    if (seen.has(agent.pane_id)) return;
    seen.add(agent.pane_id);
    rows.push({ agent, depth });
    if (opened.has(agent.pane_id)) for (const child of childrenOf(agent, byPane)) walk(child, depth + 1);
  };
  walk(root, 0);
  return sidebarTreeLines(rows, shownAll, siblings, (depths) => cappedPlaces(depths, SESSION_INDENT_LEVELS));
}

/** The path of the checkout whose tab holds `pane`, for the branch's tooltip. */
export function checkoutPathOfPane(rest: SnapshotRest | null, pane: string): string | null {
  for (const workspace of catalogWorkspaces(rest)) {
    for (const checkout of workspace.checkouts) {
      if (checkout.tabs.some((tab) => tab.panes.some((row) => row.id === pane))) return checkout.path;
    }
  }
  return null;
}
