// What a device says about moving to the connection of the Hide on its
// machine (PRD device-connect-iroh B30, B36, D-48; operator decision
// 2026-11-24). Hide moves a device by itself, so nothing here asks for a
// move or draws its steps: a device that stays says in one faint line what
// its machine needs before it can move, and a device that just moved says
// once what starts fresh there. The core owns both facts (`Device.needs`,
// `Device.not_carried`, and a device's `peer_key` once it moved).

import { useEffect, useRef } from "react";
import type { Actions } from "../actions";
import { Note, Status } from "../components/settings-rows";
import { frontDeviceId } from "../devices";
import { useInterfaceTranslation } from "../i18n/client";
import { ownWindow } from "../pairing";
import type { Device } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { FixCommand } from "./FixCommand";

/** The site's section on a Mac's automatic login, which a Mac that stays on SSH links to. */
export const AUTO_LOGIN_DOCS = "https://withhide.dev/docs/reference/troubleshooting/#a-mac-over-ssh-works-only-while-this-computer-is-on";

/**
 * What a machine needs from the operator to be back after a restart with no
 * one there (B30, B43), under its row; nothing for a machine that needs
 * nothing. A device still reached over SSH or WSL (its own row, which
 * leaves `moved` out) says it faintly, as what
 * would let it work without this computer: automatic login on a Mac, with
 * where to turn it on, or on Linux the one command an administrator runs,
 * with Copy. Once the setting is on, the next connection moves the device
 * and the line goes. A machine that already moved (its pairing's row, which
 * sets `moved`) says what the Hide started there still needs, as a warning:
 * it is not back after a restart until then.
 */
export function DeviceNeedsLine({ deviceId, actions, moved = false }: { deviceId: string; actions: Actions; moved?: boolean }) {
  const { t } = useInterfaceTranslation();
  const device = useShellStore((s) => s.rest?.navigator?.devices?.find((row) => row.id === deviceId) ?? null);
  const needs = device?.needs ?? null;
  if (needs === null) return null;
  if (needs.kind === "open_distribution") return <Note tone="warn" data-device-needs={`${deviceId}:open_distribution`}>{t("devices.needs.openDistribution")}</Note>;
  if (needs.kind === "auto_login") {
    if (moved) return <Note tone="warn" data-device-needs={`${deviceId}:auto_login`}>{t("devices.needs.autoLogin")}</Note>;
    return (
      <Note data-device-needs={`${deviceId}:auto_login`}>
        {t("devices.stays.autoLogin")}
        {" · "}
        <a href={AUTO_LOGIN_DOCS} target="_blank" rel="noopener noreferrer" className="text-primary underline underline-offset-2 outline-none focus-visible:ring-1 focus-visible:ring-ring" data-device-needs-docs={deviceId}>
          {t("devices.stays.howTo")}
        </a>
      </Note>
    );
  }
  return (
    <div className="flex flex-wrap items-center gap-x-sm gap-y-xs" data-device-needs={`${deviceId}:linger`}>
      {moved ? <Note tone="warn">{t("devices.needs.linger")}</Note> : <Note>{t("devices.stays.linger")}</Note>}
      {needs.command ? <FixCommand command={needs.command} what="linger command" actions={actions} /> : null}
    </div>
  );
}

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
