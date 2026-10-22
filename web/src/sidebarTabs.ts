import type { BoardRow } from "./projectBoard";
import type { AgentRow, Checkout, Tab } from "./snapshot";
import type { AgentGroup } from "./agentLayout";

/** One tab's segment of a checkout's agent list; `glyph` is set for every real tab, which draws its guide and tab line whatever it holds or is named. */
export type SidebarTabGroup = { tab: Tab | null; rows: BoardRow[]; count: number; glyph: boolean };

/** One collection of a checkout's agent tree: the core's own group, its drawn tab segments and its draft ids. */
export type SidebarCollection = { group: AgentGroup; segments: SidebarTabGroup[]; drafts: string[] };

/** A tab on the strip the operator named through Rename is kept when its last agent leaves, so it stays a segment of its checkout with no rows; a spawn's name does not keep it. */
function namedOnStrip(tab: Tab): boolean {
  return tab.id !== null && tab.sidebar !== undefined && tab.sidebar.named_by_operator && !tab.delegated;
}

/** Keep complete lineage subtrees with their root, ordered by the actual tabs; a named tab with no agent keeps its segment, empty. */
export function sidebarTabGroups(checkout: Checkout, device: string, rows: readonly BoardRow[], agents: readonly AgentRow[]): SidebarTabGroup[] {
  const agentPanes = new Set(agents.filter((agent) => agent.device_id === undefined || agent.device_id === device).map((agent) => agent.pane_id));
  const paneTabs = new Map(checkout.tabs.flatMap((tab) => tab.panes.map((pane) => [pane.id, tab] as const)));
  const byTab = new Map<Tab | null, BoardRow[]>();
  let owner: Tab | null = null;
  for (const row of rows) {
    if (row.depth === 0) owner = paneTabs.get(row.agent.pane_id) ?? null;
    const group = byTab.get(owner) ?? [];
    group.push(row);
    byTab.set(owner, group);
  }
  return [...checkout.tabs, null].flatMap((tab) => {
    const members = byTab.get(tab) ?? [];
    if (!members.length && !(tab !== null && namedOnStrip(tab))) return [];
    const count = tab ? new Set(tab.panes.filter((pane) => agentPanes.has(pane.id)).map((pane) => pane.id)).size : 0;
    const glyph = tab !== null && tab.id !== null && tab.sidebar !== undefined;
    return [{ tab, rows: members, count, glyph }];
  });
}

/** Keep each lineage subtree in its root tab's collection, including hidden delegated tabs. */
export function sidebarNamedGroups(checkout: Checkout, segments: readonly SidebarTabGroup[]): SidebarCollection[] {
  const collections = checkout.agent_groups;
  if (!collections) return [];
  return collections.groups.map((group) => {
    const owned = new Set(group.tabs);
    const selected = group.tabs.flatMap((id) => {
      if (collections.drafts.includes(id)) return [];
      const existing = segments.find((segment) => segment.tab?.id === id);
      if (existing) return [existing];
      const tab = checkout.tabs.find((tab) => tab.id === id && !tab.delegated);
      return tab ? [{ tab, rows: [], count: 0, glyph: true }] : [];
    });
    if (group.id === null) selected.push(...segments.filter((segment) => segment.tab === null || (!collections.groups.some((candidate) => candidate.tabs.includes(segment.tab!.id!)) && !owned.has(segment.tab!.id!))));
    return { group, segments: selected, drafts: group.tabs.filter((id) => collections.drafts.includes(id)) };
  });
}

/** Up/Down/Home/End traverse only the visible main row controls. */
export function moveSidebarKey(event: import("react").KeyboardEvent<HTMLElement>): void {
  if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  if (!["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
  const target = event.target;
  if (!(target instanceof HTMLElement) || !target.matches("[data-agent-open], [data-sidebar-tab], [data-sidebar-group-select]")) return;
  const rows = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("[data-agent-open]:not(:disabled), [data-sidebar-tab]:not(:disabled), [data-sidebar-group-select]:not(:disabled)")];
  const index = rows.indexOf(target as HTMLButtonElement);
  if (index < 0) return;
  const next = event.key === "Home" ? 0 : event.key === "End" ? rows.length - 1 : Math.max(0, Math.min(rows.length - 1, index + (event.key === "ArrowDown" ? 1 : -1)));
  event.preventDefault();
  rows[next]?.focus();
}
