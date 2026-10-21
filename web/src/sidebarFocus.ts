import type { AgentScope } from "./agentScope";
import type { AgentRow, Workspace } from "./snapshot";

/**
 * Every pane the sidebar draws a row for when no checkout is folded: each
 * checkout's roots and the children and grandchildren of the roots opened,
 * and the raised Needs You and Done rows. The Home has no row of its own
 * (PRD sidebar-titlebar-refresh D-06), so its agents are drawn only raised.
 */
export function sidebarPanes(workspaces: readonly Workspace[], scope: AgentScope | null): Set<string> {
  const panes = new Set<string>();
  for (const workspace of workspaces) {
    if (workspace.is_home) continue;
    for (const checkout of workspace.checkouts) for (const row of checkout.agent_scope.sidebar_tree.visible_rows) panes.add(row.pane_id);
  }
  for (const raised of scope?.raised ?? []) for (const ref of [...raised.shown, ...raised.more]) panes.add(ref.pane_id);
  return panes;
}

/**
 * The pane whose row stands for the focused pane: the focused pane when the
 * sidebar draws it, otherwise its nearest ancestor that it draws, since a
 * delegated child is reached through its parent's badge (docs/UI_BEHAVIOR.md,
 * Sidebar hierarchy). Null when no pane on that line is drawn.
 */
export function sidebarFocusPane(focused: string | null, agents: readonly AgentRow[], drawn: ReadonlySet<string>): string | null {
  const parents = new Map(agents.map((agent) => [agent.pane_id, agent.lineage_parent_pane_id ?? null]));
  const seen = new Set<string>();
  for (let pane = focused; pane !== null && !seen.has(pane); pane = parents.get(pane) ?? null) {
    if (drawn.has(pane)) return pane;
    seen.add(pane);
  }
  return null;
}

/**
 * Scrolls the project list by as little as brings the focus's row into view,
 * and not at all while one of its rows is already in view, in the project
 * list or in the raised list held above it. The row is the deepest one marked
 * current: an agent row, else the focused checkout's row, else the project or
 * fold row that hides it; an agent drawn both raised and in its tree is
 * brought in at its tree row, and one drawn only raised within the raised list.
 */
export function revealSidebarFocus(lists: HTMLElement): void {
  const marked = [...lists.querySelectorAll<HTMLElement>('[aria-current="true"]')];
  if (marked.length === 0) return;
  const deepest = ["[data-agent-open]", "[data-checkout]"].map((kind) => marked.filter((row) => row.matches(kind))).find((rows) => rows.length > 0) ?? marked;
  const raised = deepest.filter((row) => row.closest("[data-raised-list]") !== null);
  const tree = deepest.filter((row) => !raised.includes(row));
  const held = lists.querySelector<HTMLElement>("[data-raised-list]")?.getBoundingClientRect();
  if (held && raised.some((row) => inside(row.getBoundingClientRect(), held.top, held.bottom))) return;
  if (tree.length > 0) revealRow(lists.querySelector<HTMLElement>("[data-project-list]")!, tree, tree[0]!);
  else raised[0]!.scrollIntoView({ block: "nearest" });
}

/**
 * Scrolls the list by as little as brings `target` into view, and not at all
 * while one of `rows`, the places the same selection is drawn, is already in
 * view; Sessions brings its selected row in by this rule too. A section
 * header stuck to the list's top covers what passes under it, so a row there
 * is not in view, and the list's scroll padding brings a row in below the
 * header (PRD sidebar-titlebar-refresh B10).
 */
export function revealRow(list: HTMLElement, rows: readonly HTMLElement[], target: HTMLElement): void {
  const view = list.getBoundingClientRect();
  // The paint marker updates on the next frame; focus can arrive before it.
  const top = [...list.querySelectorAll<HTMLElement>("[data-section-sticky]")].reduce((top, header) => {
    const rect = header.getBoundingClientRect();
    return rect.top <= view.top + 0.5 && rect.bottom > view.top ? Math.max(top, rect.bottom) : top;
  }, view.top);
  if (rows.some((row) => inside(row.getBoundingClientRect(), top, view.bottom))) return;
  target.scrollIntoView({ block: "nearest" });
}

function inside(row: DOMRect, top: number, bottom: number): boolean {
  return row.top >= top && row.bottom <= bottom;
}
