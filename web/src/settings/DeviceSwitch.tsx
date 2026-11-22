// Switching a helper device to the connection of the Hide on its machine
// (PRD device-connect-iroh B35, B36, B41, B69; D-49, D-75): the offer and its
// button, the steps while hided runs it, why it stopped with Try again, and
// the one question a later computer is asked before its switch closes
// terminals. The device's row in Settings and the strip over the device's
// screen draw the same parts; the core owns where a switch stands
// (`Device.switch`) and hided what a press is answered (`pairingResult`).

import type { TFunction } from "i18next";
import { useEffect, useRef, useState } from "react";
import type { Actions } from "../actions";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "../components/ui/alert-dialog";
import { Button } from "../components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { Note, Status } from "../components/settings-rows";
import { deviceTerminalCount, installParts, SWITCH_STEP_KEY, switchFailureKey, switchOffered, switchStep, type InstallPart } from "../deviceSwitch";
import { frontDeviceId } from "../devices";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { deviceConnectOffered, ownWindow } from "../pairing";
import { machineName, windowDeviceId } from "../screenMachine";
import type { Device } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { FixCommand } from "./FixCommand";

/**
 * One press: hided switches the device, and answers this window if it has to
 * ask first. `install` and `ownFolder` are what the operator then allowed: a
 * Hide started on a machine that runs none, and going on without the
 * terminals another computer took over.
 */
export function askSwitch(actions: Actions, deviceId: string, allowed: { install?: boolean; ownFolder?: boolean } = {}) {
  actions.pairing("switch", { device: deviceId, request_id: `switch-${Date.now()}`, ...(allowed.install ? { install: true } : {}), ...(allowed.ownFolder ? { own_folder: true } : {}) });
}

/** The button of a device that can be switched; gone while its switch runs. */
export function DeviceSwitchButton({ device, actions, retry = false, size }: { device: Device; actions: Actions; retry?: boolean; size?: "sm" }) {
  const { t } = useInterfaceTranslation();
  return (
    <Button variant="secondary" size={size} onClick={() => askSwitch(actions, device.id)} data-device-switch={device.id}>
      {retry ? t("common.retry") : t("devices.switch.go")}
    </Button>
  );
}

/**
 * Where a device's switch stands (B36, B41): its steps with the one it is on,
 * or why it stopped with Try again. Nothing while no switch runs or failed.
 */
export function DeviceSwitchLine({ device, actions, className }: { device: Device; actions: Actions; className?: string }) {
  const { t } = useInterfaceTranslation();
  const state = device.switch ?? null;
  if (state === null) return null;
  if (state.state === "failed") {
    return (
      <div className={cn("flex flex-wrap items-center gap-x-sm gap-y-xs", className)} data-device-switch-state={`${device.id}:failed`} data-device-switch-reason={state.reason ?? ""}>
        <Status tone="warn">{t(switchFailureKey(state.reason))}</Status>
        <DeviceSwitchButton device={device} actions={actions} retry size="sm" />
      </div>
    );
  }
  const on = switchStep(state);
  // The first step reads as the check until an install starts (a machine
  // that already runs Hide has none), then as the mockup's three.
  const steps = [on === "checking" ? "checking" : "installing", "connecting", "done"] as const;
  const at = on === "connecting" ? 1 : 0;
  return (
    <ol role="status" className={cn("flex flex-wrap items-center gap-x-xs gap-y-xxs text-body", className)} data-device-switch-state={`${device.id}:${state.state}`}>
      {steps.map((step, index) => (
        <li key={step} className="flex items-center gap-xs">
          {index > 0 ? <span aria-hidden="true" className="text-muted-foreground">→</span> : null}
          <span className={index < at ? "text-success" : index === at ? "font-semibold text-foreground" : "text-muted-foreground"} aria-current={index === at ? "step" : undefined}>
            {t(SWITCH_STEP_KEY[step])}
          </span>
        </li>
      ))}
    </ol>
  );
}

/**
 * What the Hide on a switched machine still needs to stay reachable with
 * no one there (B30, B43), under that machine's row: automatic login on a
 * Mac, on Linux the command an administrator runs, with Copy, or a WSL
 * distribution with no systemd being opened again after Windows restarts.
 * Nothing for a machine that needs nothing.
 */
export function DeviceNeedsLine({ deviceId, actions }: { deviceId: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const needs = useShellStore((s) => s.rest?.navigator?.devices?.find((device) => device.id === deviceId)?.needs ?? null);
  if (needs === null) return null;
  if (needs.kind === "auto_login") return <Note tone="warn" data-device-needs={`${deviceId}:auto_login`}>{t("devices.needs.autoLogin")}</Note>;
  if (needs.kind === "open_distribution") return <Note tone="warn" data-device-needs={`${deviceId}:open_distribution`}>{t("devices.needs.openDistribution")}</Note>;
  return (
    <div className="flex flex-wrap items-center gap-x-sm gap-y-xs" data-device-needs={`${deviceId}:linger`}>
      <Note tone="warn">{t("devices.needs.linger")}</Note>
      {needs.command ? <FixCommand command={needs.command} what="linger command" actions={actions} /> : null}
    </div>
  );
}

/**
 * How many of a machine's terminals its switch could not take up again and
 * closed (B36), under that machine's row: the strip over its screen says it
 * once to the window that watched the switch, and the row keeps it for a
 * first open that came later. Nothing when every terminal carried on.
 */
export function DeviceNotCarriedLine({ deviceId }: { deviceId: string }) {
  const { t } = useInterfaceTranslation();
  const left = useShellStore((s) => s.rest?.navigator?.devices?.find((device) => device.id === deviceId)?.not_carried ?? 0);
  if (left <= 0) return null;
  return <Note tone="warn" data-device-not-carried={`${deviceId}:${left}`}>{t("devices.switch.notCarried", { count: left })}</Note>;
}

/**
 * The strip over a helper device's screen (B35): the offer with its button,
 * then the switch's steps or its failure. Only this computer's own window
 * draws it, since no other screen may switch a device (B52).
 */
/**
 * `start` is where the strip's words begin when it takes the window's top
 * row (the remote screen's notice, after macOS's traffic lights); left out,
 * its own inset.
 */
export function DeviceSwitchStrip({ actions, start }: { actions: Actions; start?: string }) {
  const { t } = useInterfaceTranslation();
  const ownScreen = useShellStore((s) => ownWindow(s.pairing));
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const front = useShownDevice();
  const deviceConnect = useShellStore((s) => deviceConnectOffered(s.daemon));
  const device = devices?.find((row) => row.id === front);
  // Said once: the line goes when the operator leaves the machine's screen
  // (`useSwitchedSeen`), whichever screen draws this strip meanwhile.
  const landed = useUiStore((s) => front != null && s.switchedDevices.includes(front));
  if (!ownScreen || !device) return null;
  if (landed && device.peer_key != null) {
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
  if (!switchOffered(device, deviceConnect)) return null;
  return (
    <div className="flex flex-wrap items-center gap-x-sm gap-y-xs border-b border-border bg-card px-md py-xs text-caption text-subtle-foreground" style={{ paddingInlineStart: start }} data-device-switch-strip={device.id}>
      {device.switch ? (
        <DeviceSwitchLine device={device} actions={actions} className="text-caption" />
      ) : (
        <>
          <span className="min-w-0 flex-1 break-keep">{t("devices.switch.offer", { name: machineName(devices, device.id, t) })}</span>
          <DeviceSwitchButton device={device} actions={actions} size="sm" />
        </>
      )}
    </div>
  );
}

/**
 * This computer by its own name, `MacBook`, where a sentence names the
 * machine a program comes from: `This Mac` would read as the target (the
 * name it goes by here) only once the core knows no name of its own.
 */
function ownName(devices: readonly Device[] | undefined, t: TFunction<"translation">): string {
  const own = windowDeviceId(devices);
  return devices?.find((device) => device.id === own)?.machine_name ?? machineName(devices, own, t);
}

/**
 * What a press is asked before it goes on. A machine that runs no Hide of
 * its own (B36, D-45): what would be put there, and `허용하고 설치` starts it.
 * A device another computer switched first (B69, D-75): this one gets a
 * folder of its own and the terminals it ran there close. Closing either
 * leaves the device as it was.
 */
export function DeviceSwitchDialogs({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  useSwitchAfterAdd(actions);
  useSwitchLanded();
  useSwitchedSeen();
  const answer = useShellStore((s) => (s.pairingResult?.verb === "switch" && s.pairingResult.ok && (s.pairingResult.switch === "held" || s.pairingResult.switch === "install_needed") ? s.pairingResult : null));
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const count = useShellStore((s) => (answer?.device_id ? deviceTerminalCount(s.rest, answer.device_id) : 0));
  // The answer stays in the store after its question was closed.
  const [closed, setClosed] = useState<string | null>(null);
  const deviceId = answer?.device_id;
  if (!answer || !deviceId || !answer.request_id || answer.request_id === closed) return null;
  const close = () => setClosed(answer.request_id ?? null);
  // Leaving the question leaves the device as it was; one that was added
  // for this switch is taken off again, since nothing was started on its
  // machine and no new device stays on the old connection (B41).
  const cancel = () => {
    close();
    if (devices?.find((row) => row.id === deviceId)?.switch_first) void actions.removeDevice(deviceId);
  };
  const name = machineName(devices, deviceId, t);
  if (answer.switch === "install_needed") {
    const parts = installParts(answer.system, devices?.find((row) => row.id === deviceId)?.wsl_distro != null);
    return (
      <Dialog open onOpenChange={(next) => { if (!next) cancel(); }}>
        <DialogContent data-device-switch-install={deviceId}>
          <DialogHeader>
            <DialogTitle>{t("devices.installTitle", { name })}</DialogTitle>
          </DialogHeader>
          <DialogBody className="space-y-sm">
            <p className="text-body text-subtle-foreground">{t("devices.switch.installWhat")}</p>
            <ul className="list-disc space-y-xs pl-md text-body text-subtle-foreground" data-device-switch-install-parts={parts.join(",")}>
              {parts.map((part) => (
                <li key={part}>{t(INSTALL_PART_KEY[part])}</li>
              ))}
            </ul>
            <p className="break-keep text-body text-foreground">{t("devices.switch.install.grant", { name: ownName(devices, t) })}</p>
          </DialogBody>
          <DialogFooter>
            <Button variant="secondary" onClick={cancel}>{t("common.cancel")}</Button>
            <Button
              data-device-switch-install-go="true"
              onClick={() => {
                close();
                askSwitch(actions, deviceId, { install: true });
              }}
            >
              {t("devices.allowInstall")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    );
  }
  const by = answer.by ?? "";
  // The dialog closes on either button, so its close decides nothing: only
  // Cancel and Escape leave the question.
  return (
    <AlertDialog open onOpenChange={(next) => { if (!next) close(); }}>
      <AlertDialogContent data-device-switch-held={deviceId} onEscapeKeyDown={cancel}>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("devices.switch.heldTitle", { name })}</AlertDialogTitle>
          <AlertDialogDescription>{count > 0 ? t("devices.switch.held", { by, count }) : t("devices.switch.heldNone", { by })}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel onClick={cancel}>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction
            data-device-switch-held-go="true"
            onClick={() => {
              close();
              askSwitch(actions, deviceId, { ownFolder: true });
            }}
          >
            {t("devices.switch.go")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/**
 * Notes each device whose switch lands while this window watches it: a
 * device that was being switched and is its machine's now. Its screen then
 * says once what starts fresh there (D-48).
 */
function useSwitchLanded() {
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const switching = useRef(new Set<string>());
  useEffect(() => {
    for (const device of devices ?? []) {
      if (device.switch && device.switch.state !== "failed") switching.current.add(device.id);
      else if (switching.current.delete(device.id) && device.peer_key != null) useUiStore.getState().markSwitched(device.id);
    }
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
 * A landed switch's line is said once (D-48): it goes when the operator
 * leaves that machine's screen for another. Kept here, where the window
 * always draws, since the strip that says it moves from the main screen to
 * the remote screen as the switch lands, and unmounting there is no leaving.
 */
function useSwitchedSeen() {
  const front = useShownDevice();
  const shown = useRef(front);
  useEffect(() => {
    const left = shown.current;
    shown.current = front;
    if (left != null && left !== front) useUiStore.getState().clearSwitched(left);
  }, [front]);
}

/**
 * A device added over SSH or as a WSL distribution goes on to the new
 * connection (B41): the core keeps that it was added for its switch
 * (`Device.switch_first`), and once its helper is connected this window
 * asks that switch, once. A machine whose own Hide runs is then connected
 * as a Hide; one with none answers with what would be installed there,
 * which the operator allows or leaves.
 */
function useSwitchAfterAdd(actions: Actions) {
  const ownScreen = useShellStore((s) => ownWindow(s.pairing));
  const ready = useShellStore((s) => s.rest?.navigator?.devices?.find((device) => device.switch_first === true && switchOffered(device, deviceConnectOffered(s.daemon)) && device.state === "ready" && device.host?.state === "ready" && !device.switch)?.id ?? null);
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const asked = useRef(new Set<string>());
  // A device that was taken off is a new device when it is added again,
  // under the id it had: what was asked of it goes with it.
  useEffect(() => {
    if (!devices) return;
    for (const id of asked.current) if (!devices.some((device) => device.id === id)) asked.current.delete(id);
  }, [devices]);
  useEffect(() => {
    if (ready === null || !ownScreen || asked.current.has(ready)) return;
    asked.current.add(ready);
    askSwitch(actions, ready);
  }, [actions, ready, ownScreen]);
}

const INSTALL_PART_KEY = {
  core: "devices.switch.install.core",
  autostart: "devices.switch.install.autostart",
  linger: "devices.switch.install.linger",
} as const satisfies Record<InstallPart, string>;
