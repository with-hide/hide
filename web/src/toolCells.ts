// An external tool's cell beside the devices (PRD hide-external-control
// D-45..D-53): the core publishes one per tool connected at Act with the
// work it keeps (`navigator.tools`); the window picks which cell it shows.
// This only reads which tool is in front and what its cell holds.

import type { PrBoard, ToolCell } from "./agentScope";
import { localDeviceId } from "./devices";
import type { BoardProject } from "./projectBoard";
import { missionAsks } from "./missions";
import { catalogWorkspaces, type AgentRow, type Checkout, type CheckoutAgentSummary, type Mission, type SnapshotRest, type TaskView } from "./snapshot";
import { taskList, type TaskList } from "./taskList";
import { boardProjects } from "./navigation";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { useMemo } from "react";

const NO_TOOLS: ToolCell[] = [];

/** Every tool's cell, in the access list's order. */
export function toolCells(rest: SnapshotRest | null): ToolCell[] {
  return rest?.navigator?.tools ?? NO_TOOLS;
}

/** The cell `picked` names while its tool is still at Act, else none: the window then shows the device in front (B44). */
export function frontTool(rest: SnapshotRest | null, picked: string | null): ToolCell | null {
  return picked === null ? null : (toolCells(rest).find((cell) => cell.connection === picked) ?? null);
}

// A cell that goes (disconnect, leave, View only) is forgotten at once: the
// window shows the device under it (B44), and Act again on the same tool does
// not take the window back into its cell unasked. A device the Overview named
// while the cell stood gives way too, so the Overview, the rail and the
// sidebar show the same machine.
useShellStore.subscribe((state) => {
  const ui = useUiStore.getState();
  if (ui.toolCell === null || state.rest === null || frontTool(state.rest, ui.toolCell) !== null) return;
  ui.setToolCell(null);
  if (ui.overview.deviceId !== null) ui.setOverview({ deviceId: null });
  if (ui.screen?.kind === "main" && ui.screen.deviceId !== undefined) useUiStore.setState({ screen: { kind: "main" } });
});

/** The tool cell this window shows, or null while it shows a device. */
export function useFrontTool(): ToolCell | null {
  const picked = useUiStore((s) => s.toolCell);
  return useShellStore((s) => frontTool(s.rest, picked));
}

/**
 * The tool's 태스크 list in `view`, by state or by project, as the operator's
 * (B40, B45; operator decision 2026-11-15): this machine's cards as the
 * list's own and every other device's named on its rows by `name`, with the
 * tool's missions that are their own ask among its first group's rows, or
 * under their project. The sidebar draws it and ⌥n numbers it, so both read
 * this one list.
 */
export function toolTaskList(rest: SnapshotRest | null, cell: ToolCell, rows: ReadonlyMap<string, AgentRow>, name: (deviceId: string) => string, view: TaskView): TaskList {
  const local = localDeviceId(rest);
  const own = cell.devices.find((device) => device.device_id === local)?.tasks ?? null;
  const away = cell.devices.filter((device) => device.device_id !== local).flatMap((device) => device.tasks.cards.map((card) => ({ card, device: name(device.device_id), deviceId: device.device_id })));
  const missions = rest?.missions?.failed ? [] : (rest?.missions?.items ?? []);
  return taskList(own, local, rows, catalogWorkspaces(rest), view, [], missionAsks(missions, cell.mission_asks ?? []), away);
}

/** Whether `mission` is the work of the tool of `connection` now: its Conductor is the tool's and the tool keeps it (D-36). */
export function keptBy(mission: Pick<Mission, "tool">, connection: string): boolean {
  return mission.tool?.owns === true && mission.tool.connection === connection;
}

/**
 * The Overview's projects in a tool's cell (D-48): those the tool works in,
 * each holding the tool's members and the tool's tasks only, so the graph
 * and the Projects tab draw the tool's work and none of the operator's, even
 * in a project both work in. A task belongs to the project its root works in,
 * as a member of the tool's says.
 */
export function toolProjects(projects: readonly BoardProject[], cell: ToolCell): BoardProject[] {
  return projects.flatMap((project) => {
    const { workspace } = project;
    const device = cell.devices.find((row) => row.device_id === workspace.device_id);
    const members = device?.members.filter((member) => member.project_id === workspace.id) ?? [];
    if (!device || members.length === 0) return [];
    const roots = new Set(members.map((member) => member.pane_id));
    const cards = device.tasks.cards.filter((card) => roots.has(card.root));
    const tasks = { cards, needs_you: cards.filter((card) => card.state === "needs_you").length, recency: device.tasks.recency };
    return [{ ...project, workspace: { ...workspace, agent_scope: { ...workspace.agent_scope, members, tasks } } }];
  });
}

/**
 * The sidebar's projects in a tool's cell, which its 프로젝트 and PR tabs draw
 * as the operator's are drawn (operator decision 2026-11-15): the tool's
 * projects (`toolProjects`), each cut to the checkouts that hold the tool's
 * sessions. Such a checkout carries the tool's tree and badge marks in place
 * of the operator's, keeps only the tabs the tool's sessions stand in, and
 * none of the operator's tab groups, counts, folds or sleeping-project
 * presence, so nothing of the operator's is drawn or counted there; the
 * project's PRs are the ones whose checkouts hold the tool's sessions. A
 * project holding none of them is left out.
 */
export function toolSidebarProjects(projects: readonly BoardProject[], cell: ToolCell): BoardProject[] {
  return toolProjects(projects, cell).flatMap((project) => {
    const { workspace } = project;
    const held = new Map((cell.devices.find((device) => device.device_id === workspace.device_id)?.checkouts ?? []).filter((checkout) => checkout.project_id === workspace.id).map((checkout) => [checkout.checkout_id, checkout]));
    const checkouts = workspace.checkouts.flatMap((checkout): Checkout[] => {
      const tool = held.get(checkout.id);
      if (!tool) return [];
      const roots = new Set(tool.sidebar_tree.rows.map((row) => row.pane_id));
      return [{
        ...checkout,
        tabs: checkout.tabs.filter((tab) => tab.panes.some((pane) => roots.has(pane.id))),
        agent_groups: undefined,
        agent_summary: { ...NO_COUNTS, marks: tool.marks },
        agent_scope: { ...checkout.agent_scope, sidebar_tree: tool.sidebar_tree, badge_total: 0 },
      }];
    });
    if (checkouts.length === 0) return [];
    const ids = new Set(checkouts.map((checkout) => checkout.id));
    const prs = heldPullRequests(workspace.agent_scope.prs, ids);
    return [{ ...project, workspace: { ...workspace, checkouts, session_folds: undefined, agent_presence: undefined, agent_scope: { ...workspace.agent_scope, prs } } }];
  });
}

/**
 * `toolSidebarProjects` of `tool` from the snapshot, or null while a device's
 * cell is in front, which then reads nothing more of the snapshot here.
 */
export function useToolSidebarProjects(tool: ToolCell | null): BoardProject[] | null {
  const rest = useShellStore((s) => (tool ? s.rest : null));
  const agents = useShellStore((s) => s.agents);
  return useMemo(() => (tool ? toolSidebarProjects(boardProjects(rest, agents), tool) : null), [tool, rest, agents]);
}

/** A tool's checkout counts no group of the operator's, so its hover card names none; its badge reads `marks`. */
const NO_COUNTS: CheckoutAgentSummary = { representative_pane_id: null, needs_you: 0, done: 0, working: 0, seen: 0, unknown: 0, marks: { turn: 0, going: 0, done: 0, unread: 0 } };

/** The project's PR board cut to the pull requests whose checkouts include one of `checkouts`, each kept where the core placed it. */
function heldPullRequests(board: PrBoard, checkouts: ReadonlySet<string>): PrBoard {
  const rows = board.rows.filter((row) => row.checkout_ids.some((id) => checkouts.has(id)));
  const numbers = new Set(rows.map((row) => row.number));
  return {
    rows,
    groups: board.groups.map((group) => ({ ...group, numbers: group.numbers.filter((number) => numbers.has(number)) })).filter((group) => group.numbers.length > 0),
    needs_you: rows.filter((row) => row.group === "needs_you" && row.fold === null).length,
  };
}
