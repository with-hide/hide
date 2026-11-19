// Switching a helper device to the connection of the Hide on its machine
// (PRD device-connect-iroh B35, B36, B41, B69; D-49, D-75): the offer and its
// button, the steps while hided runs it, why it stopped with Try again, and
// the one question a later computer is asked before its switch closes
// terminals. The device's row in Settings and the strip over the device's
// screen draw the same parts; the core owns where a switch stands
// (`Device.switch`) and hided what a press is answered (`pairingResult`).

import { useState } from "react";
import type { Actions } from "../actions";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "../components/ui/alert-dialog";
import { Button } from "../components/ui/button";
import { Status } from "../components/settings-rows";
import { deviceTerminalCount, SWITCH_STEP_KEY, switchFailureKey, switchOffered, switchStep } from "../deviceSwitch";
import { frontDeviceId } from "../devices";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { ownWindow } from "../pairing";
import { machineName } from "../screenMachine";
import type { Device } from "../snapshot";
import { useShellStore } from "../store";

/** One press: hided switches the device, and answers this window if it has to ask first. */
export function askSwitch(actions: Actions, deviceId: string, ownFolder = false) {
  actions.pairing("switch", { device: deviceId, request_id: `switch-${Date.now()}`, ...(ownFolder ? { own_folder: true } : {}) });
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
 * The strip over a helper device's screen (B35): the offer with its button,
 * then the switch's steps or its failure. Only this computer's own window
 * draws it, since no other screen may switch a device (B52).
 */
export function DeviceSwitchStrip({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const ownScreen = useShellStore((s) => ownWindow(s.pairing));
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const front = useShellStore((s) => frontDeviceId(s.rest));
  const device = devices?.find((row) => row.id === front);
  if (!ownScreen || !device || !switchOffered(device)) return null;
  return (
    <div className="flex flex-wrap items-center gap-x-sm gap-y-xs border-b border-border bg-card px-md py-xs text-caption text-subtle-foreground" data-device-switch-strip={device.id}>
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
 * What a press is asked before it goes on (B69, D-75): another computer
 * switched this device first, so this one gets a folder of its own and the
 * terminals it ran there close. Cancel leaves the device as it was.
 */
export function DeviceSwitchDialogs({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const answer = useShellStore((s) => (s.pairingResult?.verb === "switch" && s.pairingResult.ok && s.pairingResult.switch === "held" ? s.pairingResult : null));
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const count = useShellStore((s) => (answer?.device_id ? deviceTerminalCount(s.rest, answer.device_id) : 0));
  // The answer stays in the store after its question was closed.
  const [closed, setClosed] = useState<string | null>(null);
  const deviceId = answer?.device_id;
  if (!answer || !deviceId || !answer.request_id || answer.request_id === closed) return null;
  const by = answer.by ?? "";
  return (
    <AlertDialog open onOpenChange={(next) => { if (!next) setClosed(answer.request_id ?? null); }}>
      <AlertDialogContent data-device-switch-held={deviceId}>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("devices.switch.heldTitle", { name: machineName(devices, deviceId, t) })}</AlertDialogTitle>
          <AlertDialogDescription>{count > 0 ? t("devices.switch.held", { by, count }) : t("devices.switch.heldNone", { by })}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction data-device-switch-held-go="true" onClick={() => askSwitch(actions, deviceId, true)}>
            {t("devices.switch.go")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
