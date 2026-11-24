// The pure rules behind switching a helper device to the connection of the
// Hide on its machine (PRD device-connect-iroh B35, B36, B41, B69): which
// device offers it, what its steps and its failure read as, and how many
// terminals a later computer's switch would close. hided runs the switch and
// the core draws its step on the device (`Device.switch`); this only words
// them, so each rule is tested without a browser.

import type { MessageKey } from "./i18n/catalogs";
import { catalogWorkspaces, type Device, type DeviceSwitch, type SnapshotRest } from "./snapshot";

/**
 * Whether the switch is offered on a device: the core's one rule
 * (`Device.switchable`), a helper device it dials over SSH that has
 * connected, or a WSL distribution of this PC (B35, D-27). A paired
 * machine's device is switched already, a node that dials in was never a
 * helper, and an SSH device that never connected is switched once it has.
 */
export function switchOffered(device: Device): boolean {
  return device.switchable === true;
}

/** What is put on a machine that runs no Hide of its own. */
export type InstallPart = "core" | "autostart" | "linger";

/**
 * What a switch says it installs: linger is what keeps a Linux account's
 * programs running after its SSH login ends, so a WSL distribution, which
 * no SSH login reaches, is told of none.
 */
export function installParts(system: string | null | undefined, wsl: boolean): InstallPart[] {
  return system === "linux" && !wsl ? ["core", "autostart", "linger"] : ["core", "autostart"];
}

/** The steps a switch shows, in order; the last is where the device's row becomes its machine's. */
export const SWITCH_STEPS = ["checking", "installing", "connecting"] as const;

export type SwitchStep = (typeof SWITCH_STEPS)[number];

/** The step a running switch is on; null once it failed, while it waits for the operator, or while none runs. */
export function switchStep(state: DeviceSwitch | null | undefined): SwitchStep | null {
  return state == null || state.state === "failed" || state.state === "held" ? null : state.state;
}

/**
 * Whether a device's switch waits for the operator because another computer
 * switched the device first (B69, D-75): the core holds the question on the
 * device, whoever asked for the switch, so a device that moved by itself
 * with no window open is asked about when one opens.
 */
export function switchHeld(device: Device): boolean {
  return device.switch?.state === "held";
}

/** Whether a device's row and strip draw its switch: one that runs or stopped, not one that waits in its question. */
export function switchDrawn(device: Device): boolean {
  return device.switch != null && !switchHeld(device);
}

export const SWITCH_STEP_KEY: Record<SwitchStep | "done", MessageKey> = {
  checking: "devices.switch.checking",
  installing: "devices.switch.installing",
  connecting: "devices.switch.connecting",
  done: "devices.switch.done",
};

/** Why a switch stopped, in one line (B41); a code this build has no words for reads as the plain failure. */
export function switchFailureKey(reason: string | null | undefined): MessageKey {
  switch (reason) {
    case "ssh_not_reached":
      return "devices.switch.failed.notReached";
    case "wsl_not_reached":
      return "devices.switch.failed.wslNotReached";
    case "ssh_update_there":
      return "pairing.ssh.updateThere";
    case "ssh_refused_there":
      return "pairing.ssh.refusedThere";
    case "ssh_hide_not_running":
      return "pairing.ssh.notRunning";
    case "ssh_core_not_started":
      return "devices.switch.failed.notStarted";
    case "ssh_mac_signed_out":
      return "pairing.ssh.signInMac";
    case "ssh_mac_from_mac_only":
      return "devices.switch.failed.macOnly";
    case "switch_not_saved":
      return "devices.switch.failed.notSaved";
    case "switch_not_connected":
      return "devices.switch.failed.notConnected";
    case "device_already_connected":
      return "devices.switch.failed.already";
    case "device_limit":
      return "pairing.failed.full";
    case "peer_unavailable":
      return "pairing.refused.unavailable";
    default:
      return "devices.switch.failed.other";
  }
}

/**
 * How many terminals the core runs on a device, as it counts them for each
 * of the device's projects (`Workspace.removal`): what a switch that takes a
 * folder of its own closes (B69).
 */
export function deviceTerminalCount(rest: SnapshotRest | null | undefined, deviceId: string): number {
  return catalogWorkspaces(rest ?? null).filter((workspace) => workspace.device_id === deviceId).reduce((sum, workspace) => sum + (workspace.removal?.pane_count ?? 0), 0);
}
