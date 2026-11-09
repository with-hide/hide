// Whether a pane's device is connected now, and why not in the operator's
// words: what disables an action that has to reach that device, such as
// Open on a raised ask or Done on a person's turn (docs/UI_BEHAVIOR.md).

import type { TFunction } from "i18next";
import { remoteStateText } from "./coreError";
import { deviceConnected } from "./devices";
import { useInterfaceTranslation } from "./i18n/client";
import { remoteTargetOfPane } from "./remote";
import { machineName } from "./screenMachine";
import type { Device, RemoteStatus } from "./snapshot";
import { useShellStore } from "./store";

/** Why `device` is not connected, worded from its state; a device still connecting or holding its last report is plainly not connected. */
export function notConnectedText(remote: readonly RemoteStatus[] | undefined, devices: readonly Device[] | undefined, device: string, t: TFunction<"translation">): string {
  const state = remote?.find((row) => row.target_id === device)?.state;
  return !state || state === "not_connected" || state === "stale" ? t("devices.rail.notConnected") : remoteStateText(state, machineName(devices, device, t), t);
}

/** Why a pane on a device that is not connected cannot be opened now, or null when it can (B8). */
export function useUnreachable(target: { device: string | null } | { pane: string } | null): string | null {
  const { t } = useInterfaceTranslation();
  return useShellStore((state) => {
    if (target === null) return null;
    const device = "device" in target ? target.device : remoteTargetOfPane(state.rest, target.pane);
    if (device === null || deviceConnected(state.rest, device)) return null;
    return notConnectedText(state.rest?.status?.remote, state.rest?.navigator?.devices, device, t);
  });
}
