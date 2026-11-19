// An external tool's cell beside the devices (PRD hide-external-control
// D-45..D-53): the core publishes one per tool connected at Act with the
// work it keeps (`navigator.tools`). Which cell is in front is a scope, as a
// device is, read from what the center shows (`frontTool`), never a choice
// kept beside it; this reads that and what a cell holds.

import type { PrBoard, ToolCell } from "./agentScope";
import { frontDeviceId, localDeviceId } from "./devices";
import type { BoardProject } from "./projectBoard";
import { missionAsks } from "./missions";
import { missionOwnAsks } from "./ownAsks";
import { catalogWorkspaces, type AgentRow, type Checkout, type CheckoutAgentSummary, type Mission, type SidebarProject, type SnapshotRest, type TaskView } from "./snapshot";
import { taskList, type TaskList } from "./taskList";
import { boardProjects } from "./navigation";
import { focusedPaneOf, useShellStore } from "./store";
import { useUiStore, type Screen } from "./ui";
import { useMemo } from "react";

const NO_TOOLS: ToolCell[] = [];

/** Every tool's cell, in the access list's order. */
export function toolCells(rest: SnapshotRest | null): ToolCell[] {
  return rest?.navigator?.tools ?? NO_TOOLS;
}

/**
 * The tool whose cell is in front, or null while the device the core
 * focuses is, as a device's is read from what the center shows: a tool's
 * Overview or mission page while that page names it (its tile, or the
 * sidebar's 개요 and 미션 rows in its cell), else on a Workspace the tool
 * that keeps the pane in front, so landing on a tool's pane by any path
 * brings its cell forward and landing anywhere else takes it away, as
 * landing on a device's pane brings that device. A tool that is no longer
 * at Act has no cell, so its pages and panes read as the device's (B44).
 */
export function frontTool(rest: SnapshotRest | null, screen: Screen | null): ToolCell | null {
  const cells = toolCells(rest);
  if (!rest || cells.length === 0) return null;
  if (screen?.kind === "main" || screen?.kind === "missions") return screen.tool === undefined ? null : (cells.find((cell) => cell.connection === screen.tool) ?? null);
  if (screen?.kind !== "workspace") return null;
  const pane = focusedPaneOf(rest);
  return pane === null ? null : keeperOf(cells, frontDeviceId(rest), pane);
}

/** The tool that keeps `paneId` on `deviceId` now, by its cell's members. */
function keeperOf(cells: readonly ToolCell[], deviceId: string, paneId: string): ToolCell | null {
  return cells.find((cell) => cell.devices.some((device) => device.device_id === deviceId && device.members.some((member) => member.pane_id === paneId))) ?? null;
}

/** Whether the tool of `connection`, or with none any tool, keeps `paneId` on `deviceId`: a press leaving a tool's cell does not land on it, and one inside it stays. */
export function toolKeeps(rest: SnapshotRest | null, deviceId: string, paneId: string, connection?: string): boolean {
  const keeper = keeperOf(toolCells(rest), deviceId, paneId);
  return keeper !== null && (connection === undefined || keeper.connection === connection);
}

// A page that names a tool forgets it once the cell goes (disconnect, leave,
// View only): the window shows the device under it (B44), and Act again on
// the same tool does not take the page back into its cell unasked. A device
// the Overview named while the cell stood gives way too, so the Overview,
// the rail and the sidebar show the same machine.
useShellStore.subscribe((state) => {
  const ui = useUiStore.getState();
  const screen = ui.screen;
  if ((screen?.kind !== "main" && screen?.kind !== "missions") || screen.tool === undefined || state.rest === null) return;
  if (toolCells(state.rest).some((cell) => cell.connection === screen.tool)) return;
  if (ui.overview.deviceId !== null) ui.setOverview({ deviceId: null });
  useUiStore.setState({ screen: { kind: screen.kind } });
});

/** The tool cell this window shows, or null while it shows a device (`frontTool`). */
export function useFrontTool(): ToolCell | null {
  const screen = useUiStore((s) => s.screen);
  return useShellStore((s) => frontTool(s.rest, screen));
}

/**
 * The pane a checkout's row opens in a tool's cell in place of the
 * checkout's own focus, or null when the tab the checkout shows is already
 * one the tool's sessions stand in (`checkout` is the cell's, cut to those
 * tabs by `toolSidebarProjects`), so the row opens the checkout as the
 * operator's does. Otherwise the checkout would land on the operator's tab
 * there and leave the cell, so the row opens the tool's session the keyboard
 * was in last (`recent`, most recent first), else its first.
 */
export function toolCheckoutPane(checkout: Checkout, recent: readonly string[]): string | null {
  if (checkout.tabs.some((tab) => tab.id === checkout.active_tab_id)) return null;
  const rows = checkout.agent_scope.sidebar_tree.rows;
  const held = new Set(rows.map((row) => row.pane_id));
  return recent.find((id) => held.has(id)) ?? rows[0]?.pane_id ?? null;
}

/**
 * The tool's 태스크 list in `view`, by state or by project, as the operator's
 * (B40, B45; operator decision 2026-11-15): this machine's cards as the
 * list's own and every other device's named on its rows by `name`, with the
 * tool's missions that are their own ask among its first group's rows, or
 * under their project. With `project`, only that project's tasks and
 * missions, as the project chip narrows it. The sidebar draws it and ⌥n
 * numbers it, so both read this one list.
 */
export function toolTaskList(rest: SnapshotRest | null, cell: ToolCell, rows: ReadonlyMap<string, AgentRow>, name: (deviceId: string) => string, view: TaskView, project: SidebarProject | null = null): TaskList {
  const local = localDeviceId(rest);
  const devices = project
    ? cell.devices.filter((device) => device.device_id === project.device_id).map((device) => ({ ...device, tasks: toolTasksIn(device, project.workspace_id) }))
    : cell.devices;
  const own = devices.find((device) => device.device_id === local)?.tasks ?? null;
  const away = devices.filter((device) => device.device_id !== local).flatMap((device) => device.tasks.cards.map((card) => ({ card, device: name(device.device_id), deviceId: device.device_id })));
  const missions = rest?.missions?.failed ? [] : (rest?.missions?.items ?? []);
  const asks = missionAsks(missions, cell.mission_asks ?? []);
  return taskList(own, local, rows, catalogWorkspaces(rest), view, [], missionOwnAsks(project ? asks.filter((ask) => ask.mission.workspace_id === project.workspace_id) : asks), away);
}

/**
 * The project the tool's cell narrows its 태스크 and PR lists to: the stored
 * one (`ui_state.sidebar_project`, the operator's chip and the tool's share
 * it) while the tool works in it, else none, so a project chosen on the
 * operator's side never empties the tool's cell.
 */
export function toolProjectChoice(rest: SnapshotRest | null, cell: ToolCell): SidebarProject | null {
  const stored = rest?.ui_state?.sidebar_project ?? null;
  if (!stored) return null;
  return cell.devices.some((device) => device.device_id === stored.device_id && device.members.some((member) => member.project_id === stored.workspace_id)) ? stored : null;
}

/** The tool's tasks of one device in one project: those whose root works there, as a member of the tool's says. */
function toolTasksIn(device: ToolCell["devices"][number], projectId: string): ToolCell["devices"][number]["tasks"] {
  const roots = new Set(device.members.filter((member) => member.project_id === projectId).map((member) => member.pane_id));
  const cards = device.tasks.cards.filter((card) => roots.has(card.root));
  return { cards, needs_you: cards.filter((card) => card.state === "needs_you").length, recency: device.tasks.recency };
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
    return [{ ...project, workspace: { ...workspace, agent_scope: { ...workspace.agent_scope, members, tasks: toolTasksIn(device, workspace.id) } } }];
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
