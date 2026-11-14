// An external tool's cell beside the devices (PRD hide-external-control
// D-45..D-53): the core publishes one per tool connected at Act with the
// work it keeps (`navigator.tools`); the window picks which cell it shows.
// This only reads which tool is in front and what its cell holds.

import type { ToolCell } from "./agentScope";
import { localDeviceId } from "./devices";
import type { BoardProject } from "./projectBoard";
import { missionAsks } from "./missions";
import { catalogWorkspaces, type AgentRow, type Mission, type SnapshotRest } from "./snapshot";
import { taskList, type TaskList } from "./taskList";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

const NO_TOOLS: ToolCell[] = [];

/** Every tool's cell, in the access list's order. */
export function toolCells(rest: SnapshotRest | null): ToolCell[] {
  return rest?.navigator?.tools ?? NO_TOOLS;
}

/** The cell `picked` names while its tool is still at Act, else none: the window then shows the device in front (B44). */
export function frontTool(rest: SnapshotRest | null, picked: string | null): ToolCell | null {
  return picked === null ? null : (toolCells(rest).find((cell) => cell.connection === picked) ?? null);
}

/** The tool cell this window shows, or null while it shows a device. */
export function useFrontTool(): ToolCell | null {
  const picked = useUiStore((s) => s.toolCell);
  return useShellStore((s) => frontTool(s.rest, picked));
}

/**
 * The tool's 태스크 list by state (B40, B45): this machine's cards as the
 * list's own and every other device's named on its rows by `name`, with the
 * tool's missions that are their own ask among its first group's rows. The
 * sidebar draws it and ⌥n numbers it, so both read this one list.
 */
export function toolTaskList(rest: SnapshotRest | null, cell: ToolCell, rows: ReadonlyMap<string, AgentRow>, name: (deviceId: string) => string): TaskList {
  const local = localDeviceId(rest);
  const own = cell.devices.find((device) => device.device_id === local)?.tasks ?? null;
  const away = cell.devices.filter((device) => device.device_id !== local).flatMap((device) => device.tasks.cards.map((card) => ({ card, device: name(device.device_id), deviceId: device.device_id })));
  const missions = rest?.missions?.failed ? [] : (rest?.missions?.items ?? []);
  return taskList(own, local, rows, catalogWorkspaces(rest), "status", [], missionAsks(missions, cell.mission_asks ?? []), away);
}

/** Whether `mission` is the work of the tool of `connection` now: its Conductor is the tool's and the tool keeps it (D-36). */
export function keptBy(mission: Pick<Mission, "tool">, connection: string): boolean {
  return mission.tool?.owns === true && mission.tool.connection === connection;
}

/**
 * The Overview's projects in a tool's cell (D-48): those the tool works in,
 * each holding the tool's members only, so the graph draws the tool's work
 * and none of the operator's.
 */
export function toolProjects(projects: readonly BoardProject[], cell: ToolCell): BoardProject[] {
  return projects.flatMap((project) => {
    const { workspace } = project;
    const members = cell.devices.find((device) => device.device_id === workspace.device_id)?.members.filter((member) => member.project_id === workspace.id) ?? [];
    return members.length === 0 ? [] : [{ ...project, workspace: { ...workspace, agent_scope: { ...workspace.agent_scope, members } } }];
  });
}
