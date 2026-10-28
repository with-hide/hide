import type { TFunction } from "i18next";
import type { MoveMachines, MoveView } from "./coreMove";
import { hostBridge } from "./host";
import { localDeviceId, type Device, type SnapshotRest } from "./snapshot";

/** The longest node id a hash may name. */
const NODE_MAX = 128;

/**
 * The machine named by a page's hash as `node`: `hide connect` adds it when
 * this machine's hided runs in the node role, its core on another machine
 * (docs/ARCHITECTURE.md, A core on another machine). A value that cannot be
 * a node id is not one.
 */
export function screenNodeFromHash(hash: string): string | null {
  const node = new URLSearchParams(hash.replace(/^#/, "")).get("node");
  return node && node.length <= NODE_MAX && /^[A-Za-z0-9._-]+$/.test(node) ? node : null;
}

function hashNode(): string | null {
  const hash = typeof window === "undefined" ? undefined : window.location?.hash;
  return hash ? screenNodeFromHash(hash) : null;
}

/** The core's own row: the machine the core runs on, wherever the window is. */
function coreRowId(devices: readonly Device[] | undefined): string {
  return devices?.find((device) => device.kind !== "remote")?.id ?? "";
}

/**
 * The device this window runs on: what only this machine can do (reveal or
 * open with an app, a terminal path, a page on its loopback) follows its
 * panes and checkouts, and it is the one this window names This Mac. Without
 * a node in the hash it is the core's own.
 */
export function windowDeviceId(devices: readonly Device[] | undefined): string {
  return hashNode() ?? coreRowId(devices);
}

export type MachineKey = "common.thisMac" | "common.thisPc" | "common.thisComputer";

/**
 * The catalog key that names a machine as its system does: This Mac on
 * macOS, This PC on Windows, This computer on Linux and any system it does
 * not recognize. `platform` is a device row's `host.platform` (`macos
 * aarch64`, `windows x86_64`) or the host bridge's `process.platform`.
 */
export function systemMachineKey(platform: string): MachineKey {
  if (/^(mac|darwin)/i.test(platform)) return "common.thisMac";
  if (/^win/i.test(platform)) return "common.thisPc";
  return "common.thisComputer";
}

/**
 * The same for a machine this window can see: with no `platform`, the system
 * the shell runs on, which is this window's machine.
 */
export function thisMachineKey(platform: string | null | undefined): MachineKey {
  return systemMachineKey(platform ?? hostBridge()?.platform ?? (typeof navigator === "undefined" ? "" : navigator.platform));
}

/**
 * What this window calls a machine (PRD core-host-node-move B2, N1), the one
 * name every surface gives it: its own machine is This Mac, This PC or This
 * computer, as its system names itself, wherever the core runs; any other
 * machine goes by the name the operator registered it under, which the core's
 * own row keeps after the core moved to it, else by the name that machine
 * gives itself.
 */
export function machineName(devices: readonly Device[] | undefined, deviceId: string, t: TFunction<"translation">): string {
  const device = devices?.find((row) => row.id === deviceId);
  if (deviceId === windowDeviceId(devices)) return t(thisMachineKey(device?.host?.platform));
  return device ? (device.label ?? device.machine_name ?? device.id) : deviceId;
}

/**
 * A registration's own name, for a line about the registration itself: its
 * alias in its stead, its install, revoke or removal. It never names a
 * machine to the operator, which is `machineName`'s, since a node's
 * registration is its host name and the window on that node calls it This Mac.
 */
export function registeredName(device: Pick<Device, "id" | "label">): string {
  return device.label ?? device.id;
}

/**
 * The device in front when this window does not run on it: its Workspace
 * wears the device band and border, and Settings names it beside the page
 * title. Null while the window's own machine is in front, wherever the core
 * runs; a node is a device to the core, never to its own window.
 */
export function awayDevice(rest: SnapshotRest | null): Device | null {
  const devices = rest?.navigator?.devices;
  const front = rest?.navigator?.focused_device_id ?? localDeviceId(rest);
  if (front === windowDeviceId(devices)) return null;
  return devices?.find((device) => device.id === front) ?? null;
}

/**
 * Where a WSL distribution's device is, said beside its name (PRD wsl-device
 * B5, D-12): on the core's machine, which its own window calls This PC
 * (`WSL Ubuntu · This PC`); never Remote. Null for any other device.
 */
export function wslPlace(devices: readonly Device[] | undefined, deviceId: string, t: TFunction<"translation">): string | null {
  return devices?.find((row) => row.id === deviceId)?.wsl_distro ? coreMachineName(devices, t) : null;
}

/** The core's machine as this window names it. */
export function coreMachineName(devices: readonly Device[] | undefined, t: TFunction<"translation">): string {
  return machineName(devices, coreRowId(devices), t);
}

/**
 * The core's machine as this window names it, when the window runs on
 * another machine: what the core does with its own logins (gh, Hide AI's
 * agents) is fixed there (B15). Null on the core's own machine (B1).
 */
export function coreMachineAway(devices: readonly Device[] | undefined, t: TFunction<"translation">): string | null {
  return windowDeviceId(devices) === coreRowId(devices) ? null : coreMachineName(devices, t);
}

/** Whether `deviceId` is the machine the core runs on. */
export function isCoreMachine(devices: readonly Device[] | undefined, deviceId: string): boolean {
  return deviceId === coreRowId(devices);
}

/** The device rows in this window's order: its own machine first, then the rest as the core lists them (N1). */
export function windowFirst(devices: readonly Device[] | undefined): Device[] {
  const rows = devices ?? [];
  const own = windowDeviceId(rows);
  return [...rows.filter((device) => device.id === own), ...rows.filter((device) => device.id !== own)];
}

/**
 * Whether the core works with another machine (Q11): this window is on a
 * node, or a node dials in to the core. Only then does the rail mark the
 * core's machine; a core alone on its Mac has nothing to tell apart (B1).
 */
export function coreShared(devices: readonly Device[] | undefined): boolean {
  return windowDeviceId(devices) !== coreRowId(devices) || (devices ?? []).some((device) => device.dials_in === true);
}

/**
 * The machines a move names, as this window calls them: toward a device,
 * from this machine to that device; back, from the core's machine to this
 * one.
 * The move's frame names the other machine by the registration id this
 * machine's core gave it, which only a core that dials it lists: while the
 * core runs there, before a move back or after a forward move, it is the
 * core's machine.
 * A window the move holds before it drew the core, a reload during a long
 * wait, has no device rows, so it names the other machine by the alias the
 * move reaches it by.
 */
export function moveMachines(devices: readonly Device[] | undefined, view: Pick<MoveView, "direction" | "device" | "alias">, t: TFunction<"translation">): MoveMachines {
  const here = t(thisMachineKey(null));
  const rows = devices ?? [];
  const listed = rows.find((device) => device.id === view.device);
  const other = rows.length === 0 && view.alias ? view.alias : listed ? machineName(rows, listed.id, t) : coreMachineName(rows, t);
  if (view.direction === "back") return { from: other, to: here };
  if (!view.device) return { from: here, to: here };
  return { from: here, to: other };
}
