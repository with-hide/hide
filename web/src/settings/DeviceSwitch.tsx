// What a device that was switched to the connection of the Hide on its
// machine says (PRD device-connect-iroh B36, D-48; operator decision
// 2026-11-24): once, what starts fresh there, and how many terminals the
// switch could not carry on. The operator asks for a switch; nothing here
// starts one. The core owns both facts (`Device.not_carried`, and a
// device's `peer_key` once it was switched).

import { useEffect, useRef } from "react";
import { Note, Status } from "../components/settings-rows";
import { frontDeviceId } from "../devices";
import { useInterfaceTranslation } from "../i18n/client";
import { ownWindow } from "../pairing";
import type { Device } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";

/**
 * How many of a machine's terminals its move could not take up again and
 * closed (B36), under that machine's row: the strip over its screen says it
 * once to the window that watched the move, and the row keeps it for a
 * first open that came later. Nothing when every terminal carried on.
 */
export function DeviceNotCarriedLine({ deviceId }: { deviceId: string }) {
  const { t } = useInterfaceTranslation();
  const left = useShellStore((s) => s.rest?.navigator?.devices?.find((device) => device.id === deviceId)?.not_carried ?? 0);
  if (left <= 0) return null;
  return <Note tone="warn" data-device-not-carried={`${deviceId}:${left}`}>{t("devices.switch.notCarried", { count: left })}</Note>;
}

/**
 * The strip over the screen of a machine that just moved (D-48): what
 * starts fresh there, said once. Only this computer's own window draws it.
 * `start` is where the strip's words begin when it takes the window's top
 * row (the remote screen's notice, after macOS's traffic lights); left out,
 * its own inset.
 */
export function DeviceMovedStrip({ start }: { start?: string }) {
  const { t } = useInterfaceTranslation();
  const ownScreen = useShellStore((s) => ownWindow(s.pairing));
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const front = useShownDevice();
  const device = devices?.find((row) => row.id === front);
  // Said once: the line goes when the operator leaves the machine's screen
  // (`useMovedSeen`), whichever screen draws this strip meanwhile.
  const landed = useUiStore((s) => front != null && s.switchedDevices.includes(front));
  if (!ownScreen || !device || !landed || device.peer_key == null) return null;
  const left = device.not_carried ?? 0;
  return (
    <div className="flex flex-col gap-xxs border-b border-border bg-card px-md py-xs text-caption text-subtle-foreground" style={{ paddingInlineStart: start }} data-device-switched={device.id}>
      {left > 0 ? (
        <Status tone="warn" data-device-not-carried={String(left)}>
          {t("devices.switch.notCarried", { count: left })}
        </Status>
      ) : null}
      <span className="break-keep">{t("devices.switch.fresh")}</span>
    </div>
  );
}

/**
 * Watches the devices for the window, which always mounts it: it notes
 * each device that moves while the window is open, and forgets it once the
 * operator has left that machine's screen. It draws nothing.
 */
export function DeviceMoveWatch() {
  useMoved();
  useMovedSeen();
  return null;
}

/** Whether the core reaches the device over SSH or WSL: one that has not moved. */
function reachedByLogin(device: Device): boolean {
  return device.peer_key == null && (device.ssh_alias != null || device.wsl_distro != null);
}

/**
 * Notes each device that moves while this window watches it: one the core
 * reached over SSH or WSL that is its machine's now, under the id it had.
 * Its screen then says once what starts fresh there (D-48).
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

/**
 * The machine whose screen the window shows: a paired machine's own screen
 * opened from its tile, else the device in front. A paired machine's screen
 * moves no focus in the core, so the device in front alone would name the
 * machine the operator left.
 */
function useShownDevice(): string {
  const remote = useUiStore((s) => s.remoteScreen);
  const front = useShellStore((s) => frontDeviceId(s.rest));
  return remote ?? front;
}

/**
 * A moved machine's line is said once (D-48): it goes when the operator
 * leaves that machine's screen for another. Kept here, where the window
 * always draws, since the strip that says it moves from the main screen to
 * the remote screen as the move lands, and unmounting there is no leaving.
 */
function useMovedSeen() {
  const front = useShownDevice();
  const shown = useRef(front);
  useEffect(() => {
    const left = shown.current;
    shown.current = front;
    if (left != null && left !== front) useUiStore.getState().clearSwitched(left);
  }, [front]);
}
