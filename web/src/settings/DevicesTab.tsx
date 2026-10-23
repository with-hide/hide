import { PlusIcon } from "lucide-react";
import { useEffect, useState } from "react";
import type { Actions } from "../actions";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "../components/ui/alert-dialog";
import { Button } from "../components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { Group, Note, Row } from "../components/settings-rows";
import { latestDraft } from "../editor/draft";
import { useInterfaceTranslation } from "../i18n/client";
import { deviceConnected, frontDeviceId } from "../devices";
import type { MoveView } from "../coreMove";
import { coreMachineName, isCoreMachine, machineName, registeredName, windowDeviceId, windowFirst } from "../screenMachine";
import { deviceRemovalLines, draftExported, kitRemovalLine, unstoredDeviceDrafts } from "../settings";
import type { Device } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { AddDevice } from "./AddDevice";
import { askMove, CoreMoveDialog, requestOfView, type MoveRequest } from "./CoreMoveDialog";
import { DeviceDetails } from "./DeviceDetails";
import { DeviceRow, type DeviceRowHandlers } from "./DeviceRow";
import { KitTerms } from "./MachineKit";
import { coreErrorText } from "../coreError";
import { useErrorSince } from "../errorSince";

export function DevicesTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const focused = useShellStore((s) => frontDeviceId(s.rest));
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const [removing, setRemoving] = useState<Device | null>(null);
  const [allowing, setAllowing] = useState<Device | null>(null);
  const [revoking, setRevoking] = useState<Device | null>(null);
  const [details, setDetails] = useState<Device | null>(null);
  // The move the operator opened, and the frame it was asked beside (B3 to B5).
  // `opened` counts the strip's requests, so each opens a fresh dialog at its confirm.
  const [move, setMove] = useState<{ request: MoveRequest; asked: MoveView | null; giveUp?: boolean; opened?: number } | null>(null);
  const [disconnecting, setDisconnecting] = useState(false);
  // The device count when the Add dialog opened: a new row means the device was added, which closes it.
  const [adding, setAdding] = useState<number | null>(null);
  // The removal waits for the device's drafts to be stored (B26, B44).
  const [removalBusy, setRemovalBusy] = useState(false);
  const [actedAt, setActedAt] = useState<number | null>(null);
  const deviceError = useErrorSince(actedAt, ["device.", "remote.", "kit."]);
  // Each machine's kit is read once when this tab opens, so a part removed
  // by hand shows without a relaunch (B7).
  const live = useShellStore((s) => s.connection === "live");
  useEffect(() => {
    if (live) actions.checkKit();
  }, [actions, live]);
  const rows = windowFirst(devices);
  const own = windowDeviceId(devices);
  const windowIsNode = !isCoreMachine(devices, own);
  const coreName = coreMachineName(devices, t);
  const connectedIds = useShellStore((s) => (s.rest?.navigator?.devices ?? []).filter((device) => deviceConnected(s.rest, device.id)).map((device) => device.id).join("\n"));
  const localHost = rows.find((device) => device.kind !== "remote")?.host ?? null;
  const localRoot = localHost?.helper_root ?? null;
  const localCliDir = localHost?.cli_dir ?? null;
  const remoteRows = rows.filter((device) => device.kind === "remote");
  const registrations = useShellStore((s) => s.rest?.ui_state?.workspace_registrations);
  const editorTabs = useShellStore((s) => s.editor?.tabs);
  const recoveryDrafts = useShellStore((s) => s.recoveryDrafts);
  const exportedDrafts = useShellStore((s) => s.exportedDrafts);
  const bufferWarnings = useShellStore((s) => s.bufferWarnings);
  const released = draftExported(exportedDrafts, latestDraft);
  const removalLines = removing
    ? deviceRemovalLines(removing.id, registrations ?? [], editorTabs ?? [], recoveryDrafts, (tabId) => bufferWarnings.has(tabId) && released(tabId), t)
    : [];
  const unstored = removing ? unstoredDeviceDrafts(removing.id, editorTabs ?? [], bufferWarnings, released) : [];
  // Settings opened to add a device (the rail's `+`): the dialog opens once.
  const addRequested = useUiStore((s) => s.addDeviceRequested);
  useEffect(() => {
    if (!addRequested || devices === undefined) return;
    useUiStore.getState().clearAddDeviceRequest();
    setAdding(devices.length);
  }, [addRequested, devices]);
  useEffect(() => {
    if (adding !== null && rows.length > adding) setAdding(null);
  }, [adding, rows.length]);
  // Settings opened from the window's strip to give up a move's wait: the move dialog opens once, at its confirm.
  const giveUpRequested = useUiStore((s) => s.moveGiveUpRequested);
  useEffect(() => {
    if (!giveUpRequested) return;
    useUiStore.getState().clearMoveGiveUpRequest();
    const view = useShellStore.getState().coreMove;
    if (view) setMove((current) => ({ request: requestOfView(view), asked: null, giveUp: true, opened: (current?.opened ?? 0) + 1 }));
  }, [giveUpRequested]);
  const handlers: DeviceRowHandlers = {
    onAct: () => setActedAt(Date.now()),
    onDetails: setDetails,
    onAllow: setAllowing,
    onRevoke: setRevoking,
    onRemove: setRemoving,
    onMove: (device) => {
      const request: MoveRequest = { direction: "forward", device: device.id };
      setMove({ request, asked: askMove(actions, request, false) });
    },
    onMoveBack: () => {
      const request: MoveRequest = { direction: "back" };
      setMove({ request, asked: askMove(actions, request, false) });
    },
    onDisconnect: () => setDisconnecting(true),
  };
  return (
    <>
      <Group
        title={t("settings.tabs.devices")}
        action={
          <Button variant="secondary" size="sm" onClick={() => setAdding(rows.length)} data-device-add-open="true">
            <PlusIcon aria-hidden="true" />
            {t("devices.addTitle")}
          </Button>
        }
      >
        {rows.map((device) => (
          <DeviceRow
            key={device.id}
            device={device}
            status={remote?.find((row) => row.target_id === device.id)}
            focused={focused === device.id}
            place={{
              name: machineName(devices, device.id, t),
              core: isCoreMachine(devices, device.id),
              own: device.id === own,
              windowIsNode,
              connected: connectedIds.split("\n").includes(device.id),
              coreName,
            }}
            actions={actions}
            handlers={handlers}
          />
        ))}
        {remoteRows.length === 0 ? <Row label={<Note>{t("devices.noRemote")}</Note>} /> : null}
        {deviceError ? <Row label={<Note tone="error" data-device-error="true" data-core-error={deviceError.kind}>{coreErrorText(deviceError, t)}</Note>} /> : null}
      </Group>
      {adding !== null ? (
        <Dialog open onOpenChange={(next) => { if (!next) setAdding(null); }}>
          <DialogContent showCloseButton data-add-device-dialog="true">
            <DialogHeader>
              <DialogTitle>{t("devices.addTitle")}</DialogTitle>
            </DialogHeader>
            <DialogBody>
              <AddDevice actions={actions} devices={rows} helperRoot={localRoot} cliDir={localCliDir} />
            </DialogBody>
          </DialogContent>
        </Dialog>
      ) : null}
      {move ? (
        <CoreMoveDialog
          key={move.opened ?? 0}
          request={move.request}
          asked={move.asked}
          giveUp={move.giveUp}
          actions={actions}
          onClose={() => setMove(null)}
          onRequest={(request) => setMove((current) => (current ? { ...current, request } : current))}
        />
      ) : null}
      {disconnecting ? (
        <AlertDialog open onOpenChange={(next) => { if (!next) setDisconnecting(false); }}>
          <AlertDialogContent data-core-disconnect-confirm="true">
            <AlertDialogHeader>
              <AlertDialogTitle>{t("coreMove.disconnectTitle", { machine: coreName })}</AlertDialogTitle>
              <AlertDialogDescription>{t("coreMove.disconnectDescription", { machine: coreName })}</AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
              <AlertDialogAction data-core-disconnect-go="true" onClick={() => actions.coreLink("disconnect")}>
                {t("coreMove.disconnectGo")}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
      {details ? <DeviceDetails device={rows.find((device) => device.id === details.id) ?? details} onClose={() => setDetails(null)} /> : null}
      {allowing ? (
        <Dialog open onOpenChange={(next) => { if (!next) setAllowing(null); }}>
          <DialogContent data-device-host-allow-confirm={allowing.id}>
            <DialogHeader>
              <DialogTitle>{t("devices.installTitle", { name: registeredName(allowing) })}</DialogTitle>
            </DialogHeader>
            <DialogBody className="space-y-sm">
              {allowing.host?.state === "identity_changed" ? (
                <p className="text-body text-warning">
                  {t("devices.identityChangedDescription", { alias: allowing.ssh_alias ?? registeredName(allowing) })}
                </p>
              ) : null}
              <KitTerms helperRoot={allowing.host?.helper_root ?? localRoot} cliDir={allowing.host?.cli_dir ?? localCliDir} />
            </DialogBody>
            <DialogFooter>
              <Button variant="secondary" onClick={() => setAllowing(null)}>{t("devices.notNow")}</Button>
              <Button
                data-device-host-allow-go="true"
                onClick={() => {
                  setActedAt(Date.now());
                  actions.setDeviceHostConsent(allowing.id, true);
                  setAllowing(null);
                }}
              >
                {t("devices.allowInstall")}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      ) : null}
      {revoking ? (
        <AlertDialog open onOpenChange={(next) => { if (!next) setRevoking(null); }}>
          <AlertDialogContent data-device-host-revoke-confirm={revoking.id}>
            <AlertDialogHeader>
              <AlertDialogTitle>{t("devices.revokeTitle", { name: registeredName(revoking) })}</AlertDialogTitle>
              <AlertDialogDescription>{t("devices.revokeDescription", { alias: revoking.ssh_alias ?? registeredName(revoking) })}</AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{t("devices.keepAllowed")}</AlertDialogCancel>
              <AlertDialogAction
                data-device-host-revoke-go="true"
                onClick={() => {
                  setActedAt(Date.now());
                  actions.setDeviceHostConsent(revoking.id, false);
                }}
              >
                {t("devices.revokeHelper")}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
      {removing ? (
        <AlertDialog open onOpenChange={(next) => { if (!next && !removalBusy) setRemoving(null); }}>
          <AlertDialogContent data-device-remove-confirm={removing.id}>
            <AlertDialogHeader>
              <AlertDialogTitle>{t("devices.removeTitle", { name: registeredName(removing) })}</AlertDialogTitle>
              <AlertDialogDescription>{t("devices.removeDescription", { alias: removing.ssh_alias ?? registeredName(removing) })}</AlertDialogDescription>
            </AlertDialogHeader>
            <p className="text-body text-subtle-foreground" data-device-remove-kit={removing.id}>
              {kitRemovalLine(removing, t)}
            </p>
            {removalLines.map((line) => (
              <p key={line} className="text-body text-subtle-foreground" data-device-remove-effect="true">
                {line}
              </p>
            ))}
            {unstored.length > 0 ? (
              <Note tone="warn" data-device-remove-unstored="true">
                {t("devices.unstored", { count: unstored.length, paths: unstored.join(", ") })}
              </Note>
            ) : null}
            <AlertDialogFooter>
              {/* A removal already storing drafts goes out when they land, so it
                  cannot be kept from here. Plain Button, not AlertDialogAction:
                  it has to stay open through the async removal. */}
              <Button variant="secondary" disabled={removalBusy} onClick={() => setRemoving(null)}>{t("devices.keepDevice")}</Button>
              <Button
                variant="destructive"
                disabled={unstored.length > 0 || removalBusy}
                data-device-remove-go="true"
                onClick={() => {
                  const target = removing;
                  setActedAt(Date.now());
                  setRemovalBusy(true);
                  // A draft that failed to store while it was flushed keeps
                  // the dialog open, where its note now names it.
                  void actions
                    .removeDevice(target.id)
                    .then((kept) => {
                      if (kept.length === 0) setRemoving((current) => (current?.id === target.id ? null : current));
                    })
                    .finally(() => setRemovalBusy(false));
                }}
              >
                {removalBusy ? t("devices.storingDrafts") : t("devices.removeNamed", { name: registeredName(removing) })}
              </Button>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
    </>
  );
}
