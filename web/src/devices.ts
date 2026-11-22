// The device rail's facts (PRD home-device-rail D-09, D-10, D-27, superseded
// by quick device-rail-badges and quick device-rail-slack): which devices the
// rail lists, whether each is connected, the Needs You and unseen Done counts
// each tile carries, and the Home row's project count. Everything is read from the snapshot the core
// already publishes; nothing is counted that it does not carry, and a device
// that is not connected has no count, since what it last reported is not current.

import type { TFunction } from "i18next";
import { deviceScope } from "./agentScope";
import { machineName, windowDeviceId, windowFirst, wslPlace } from "./screenMachine";
import { isRemoteView } from "./remoteView";
import { localDeviceId, type AgentRow, type Device, type SnapshotRest, type Workspace } from "./snapshot";

export { thisMachineKey } from "./screenMachine";

/**
 * The rail is shown unless the operator hid it; the choice is the core's
 * `ui_state.device_rail_visible`. Another machine's view draws none: the
 * desktop that shows it has its own rail beside it (PRD device-connect-iroh
 * B20).
 */
export function railShown(rest: SnapshotRest | null): boolean {
  if (isRemoteView()) return false;
  return rest?.ui_state?.device_rail_visible ?? true;
}

export { localDeviceId } from "./snapshot";

/**
 * The rail's devices top to bottom: the window's own machine, then each
 * other device in the core's order; before the snapshot names a device, the
 * window's machine alone, by the id it has then.
 */
export function railDeviceIds(devices: readonly Device[] | undefined): string[] {
  const rows = windowFirst(devices);
  return rows.length > 0 ? rows.map((device) => device.id) : [windowDeviceId(devices)];
}

/** The device in front: the one the core focuses, this machine by default. */
export function frontDeviceId(rest: SnapshotRest | null): string {
  return rest?.navigator?.focused_device_id ?? localDeviceId(rest);
}

/**
 * The tile the rail rings (PRD device-connect-iroh D-55): the device in
 * front, except that this computer's work on a paired machine is this
 * computer's, opened in its own tile with that machine's mark, and so is a
 * pane of another machine's work that runs here. A paired machine's own tile
 * is that machine's screen, never this computer's work on it.
 */
export function railFrontId(rest: SnapshotRest | null, guestPaneInFront: boolean): string {
  const front = frontDeviceId(rest);
  if (guestPaneInFront) return localDeviceId(rest) || front;
  const paired = rest?.navigator?.devices?.find((device) => device.id === front)?.peer_key != null;
  return paired ? localDeviceId(rest) || front : front;
}

/**
 * Whether the core reads and changes this project's Git facts, issues and
 * disk as its own: one of its own machine's, or one of a node that dials it
 * (`Device.dials_in`, PRD core-host-node-move D-29); never a device it dials.
 */
export function readsHere(workspace: Workspace, devices: readonly Device[] | null | undefined): boolean {
  if (!workspace.remote_target_id) return true;
  return devices?.some((device) => device.id === workspace.device_id && device.dials_in === true) ?? false;
}

/** Whether the core reads the device now: this machine always, a remote while its connection row says connected. */
export function deviceConnected(rest: SnapshotRest | null, deviceId: string): boolean {
  if (deviceId === localDeviceId(rest)) return true;
  return rest?.status?.remote?.find((row) => row.target_id === deviceId)?.state === "connected";
}

/** The agents a device reports now: this machine's, or a connected device's own; none while it is not connected. */
export function deviceAgents(rest: SnapshotRest | null, localAgents: AgentRow[], deviceId: string): AgentRow[] {
  if (deviceId === localDeviceId(rest)) return localAgents;
  const status = rest?.status?.remote?.find((row) => row.target_id === deviceId);
  return status?.state === "connected" ? (status.session?.agents ?? []) : [];
}

/** The states the Agents tab counts above its list, in its order: Needs You, unseen Done, Working. */
export const BADGE_STATES = [{ state: "needs_you" }, { state: "done" }, { state: "working" }] as const;

/** The text color of a state's count in the Agents tab's header: one color per state, the fill the rail's marks wear. */
export const BADGE_TEXT: Record<BadgeState, string> = { needs_you: "text-warning", done: "text-success", working: "text-agent-working" };

export type BadgeState = (typeof BADGE_STATES)[number]["state"];

/**
 * What a device's tile counts now (quick device-rail-slack): its Needs You and
 * unseen Done agents, both zero while it is not connected. Working is not on
 * the rail, since it is no reason to switch device; the Agents tab counts it.
 */
export type TileCounts = { needs_you: number; done: number };

export function tileCounts(rest: SnapshotRest | null, _localAgents: AgentRow[], deviceId: string): TileCounts {
  const devices = rest?.navigator?.devices ?? [];
  const own = (id: string): TileCounts => {
    const counts = deviceScope(rest, id)?.groups;
    return { needs_you: counts?.needs_you ?? 0, done: counts?.done ?? 0 };
  };
  // A paired machine's tile is that machine's own screen, so it counts what
  // that machine told of its own work and nothing else (PRD
  // device-connect-iroh D-50); this computer's work that runs there opens in
  // this computer's tile (D-55) and is counted on it.
  const peerKey = devices.find((device) => device.id === deviceId)?.peer_key;
  if (peerKey != null) {
    const told = rest?.navigator?.peer_work?.find((peer) => peer.key === peerKey);
    return { needs_you: told?.needs_you ?? 0, done: told?.done ?? 0 };
  }
  const counts = own(deviceId);
  if (deviceId !== localDeviceId(rest)) return counts;
  for (const device of devices) {
    if (device.peer_key == null || !deviceConnected(rest, device.id)) continue;
    const away = own(device.id);
    counts.needs_you += away.needs_you;
    counts.done += away.done;
  }
  return counts;
}

/** The number the Needs You pill draws: the count, or `9+` from ten. */
export function badgeText(count: number): string {
  return count >= 10 ? "9+" : String(count);
}

/**
 * The letters a remote device's tile draws in place of a name: the first
 * letter of each of the first two words (`Mac mini` → `Mm`, `build-box` →
 * `Bb`), or the first letter alone for a one-word name (`mini` → `M`), the
 * first one capitalized. The name itself is the tile's hint.
 */
export function tileMonogram(label: string): string {
  const [first = "", second = ""] = label.normalize("NFC").split(/[\s._-]+/).filter(Boolean);
  const initial = (word: string) => Array.from(word)[0] ?? "";
  return initial(first).toLocaleUpperCase() + initial(second);
}

/** What a tile says besides its name: that it is not connected, or each non-zero count it marks, in full where the pill reads `9+`. */
function tileFacts(connected: boolean, counts: TileCounts, t: TFunction<"translation">): string[] {
  if (!connected) return [t("devices.rail.notConnected")];
  return [counts.needs_you > 0 ? t("devices.rail.needsYou", { count: counts.needs_you }) : null, counts.done > 0 ? t("devices.rail.done", { count: counts.done }) : null].filter((fact) => fact !== null);
}

/** What a tile is called for assistive technology: the name, then its connection or its counts (B4). */
export function tileName(label: string, connected: boolean, counts: TileCounts, t: TFunction<"translation">): string {
  return [label, ...tileFacts(connected, counts, t)].join(", ");
}

/**
 * A tile's hint, the only place its name is written: the name, then its
 * connection or its counts (`Mac mini · Needs You 3`).
 */
export function tileHint(label: string, connected: boolean, counts: TileCounts, t: TFunction<"translation">): string {
  return [label, ...tileFacts(connected, counts, t)].join(" · ");
}

/**
 * The name on the sidebar's top line and the smaller word after it: the
 * device in front, named as this window calls it, then Remote for another
 * machine, or where a WSL distribution is (`WSL Ubuntu This PC`, PRD
 * wsl-device B5).
 */
export function frontTitle(devices: readonly Device[] | undefined, frontId: string | null | undefined, t: TFunction<"translation">): { name: string; note: string | null } {
  const own = windowDeviceId(devices);
  const id = (frontId ? devices?.find((row) => row.id === frontId)?.id : undefined) ?? devices?.find((row) => row.kind !== "remote")?.id ?? own;
  return { name: machineName(devices, id, t), note: id === own ? null : (wslPlace(devices, id, t) ?? t("devices.remote")) };
}

/** What the sidebar's list shows: the front device's Projects, or the way to reconnect a device that cannot be read. */
export type SidebarBody = "disconnected" | "projects";

/** Which list fills the sidebar (B8): the device's name and the way to reconnect while it is not connected, else its Projects. */
export function sidebarBody(rest: SnapshotRest | null): SidebarBody {
  return deviceConnected(rest, frontDeviceId(rest)) ? "projects" : "disconnected";
}
