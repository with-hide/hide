// What a device that was switched to the connection of the Hide on its
// machine says (PRD device-connect-iroh B36, D-48; operator decision
// 2026-11-24): how many terminals the switch could not carry on, and, to
// the window that watched it switch, once what starts fresh there
// (`DeviceWays`' notice). The core owns both facts (`Device.not_carried`,
// and a device's `peer_key` once it was switched).

import { useEffect, useRef } from "react";
import { Note } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import type { Device } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";

/**
 * How many of a machine's terminals its move could not take up again and
 * closed (B36), under that machine's row. Nothing when every terminal
 * carried on.
 */
export function DeviceNotCarriedLine({ deviceId }: { deviceId: string }) {
  const { t } = useInterfaceTranslation();
  const left = useShellStore((s) => s.rest?.navigator?.devices?.find((device) => device.id === deviceId)?.not_carried ?? 0);
  if (left <= 0) return null;
  return <Note tone="warn" data-device-not-carried={`${deviceId}:${left}`}>{t("devices.switch.notCarried", { count: left })}</Note>;
}

/**
 * Watches the devices for the window, which always mounts it: it notes
 * each device that moves while the window is open, whoever asked for the
 * move, so its row says once what starts fresh there. It draws nothing.
 */
export function DeviceMoveWatch() {
  useMoved();
  return null;
}

/** Whether the core reaches the device over SSH or WSL: one that has not moved. */
function reachedByLogin(device: Device): boolean {
  return device.peer_key == null && (device.ssh_alias != null || device.wsl_distro != null);
}

/**
 * Notes each device that moves while this window watches it: one the core
 * reached over SSH or WSL that is its machine's now, under the id it had.
 * Its opened row then says once what starts fresh there (D-48).
 */
function useMoved() {
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const reached = useRef(new Set<string>());
  useEffect(() => {
    const listed = new Set<string>();
    for (const device of devices ?? []) {
      listed.add(device.id);
      if (reachedByLogin(device)) reached.current.add(device.id);
      else if (reached.current.delete(device.id) && device.peer_key != null) useUiStore.getState().markSwitched(device.id);
    }
    // A device that was taken off is a new device when it is added again.
    for (const id of reached.current) if (!listed.has(id)) reached.current.delete(id);
  }, [devices]);
}
