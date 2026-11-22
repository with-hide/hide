import { PlusIcon } from "lucide-react";
import { useEffect, useState } from "react";
import type { Actions } from "../actions";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "../components/ui/alert-dialog";
import { Button } from "../components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { Group, Note, Row } from "../components/settings-rows";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { latestDraft } from "../editor/draft";
import { useInterfaceTranslation } from "../i18n/client";
import { frontDeviceId } from "../devices";
import { coreMachineName, isCoreMachine, machineName, registeredName, windowDeviceId, windowFirst } from "../screenMachine";
import { deviceRemovalLines, draftExported, kitRemovalLine, unstoredDeviceDrafts } from "../settings";
import type { Device } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { switchFailureKey } from "../deviceSwitch";
import { AddByLink, AddDevice } from "./AddDevice";
import { DeviceDetails } from "./DeviceDetails";
import { DeviceRow, type DeviceRowHandlers } from "./DeviceRow";
import { PairedRows } from "./PairingRows";
import { deviceConnectOffered, listedDevices, ownWindow } from "../pairing";
import { KitTerms } from "./MachineKit";
import { coreErrorText } from "../coreError";
import { useErrorSince } from "../errorSince";

type AddKind = "hide" | "ssh";

export function DevicesTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const focused = useShellStore((s) => frontDeviceId(s.rest));
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const [removing, setRemoving] = useState<Device | null>(null);
  const [allowing, setAllowing] = useState<Device | null>(null);
  const [revoking, setRevoking] = useState<Device | null>(null);
  const [details, setDetails] = useState<Device | null>(null);
  const [disconnecting, setDisconnecting] = useState(false);
  // The device count when the Add dialog opened: a new row means the device was added, which closes it.
  const [adding, setAdding] = useState<number | null>(null);
  // Which kind of machine the Add dialog adds; it opens on a Hide's link.
  const [addKind, setAddKind] = useState<AddKind>("hide");
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
  const localHost = rows.find((device) => device.kind !== "remote")?.host ?? null;
  const localRoot = localHost?.helper_root ?? null;
  const localCliDir = localHost?.cli_dir ?? null;
  // A paired machine's row is its pairing's; its device has none of its own.
  const listed = listedDevices(rows);
  // A build that does not offer connections between two Hides shows no
  // pairing, and any screen adds an SSH device as before pairing.
  const pairs = useShellStore((s) => deviceConnectOffered(s.daemon));
  // What the last Add over SSH did not add, and why (B41).
  const notAdded = useShellStore((s) => s.rest?.navigator?.device_not_added ?? null);
  // Adding a machine is this computer's own window's to do (B52).
  const ownScreen = useShellStore((s) => ownWindow(s.pairing));
  const canAdd = ownScreen || !pairs;
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
  useEffect(() => {
    if (adding === null) setAddKind("hide");
  }, [adding]);
  const handlers: DeviceRowHandlers = {
    onAct: () => setActedAt(Date.now()),
    onDetails: setDetails,
    onAllow: setAllowing,
    onRevoke: setRevoking,
    onRemove: setRemoving,
    onDisconnect: () => setDisconnecting(true),
  };
  const row = (device: Device) => (
    <DeviceRow
      key={device.id}
      device={device}
      status={remote?.find((line) => line.target_id === device.id)}
      focused={focused === device.id}
      place={{
        name: machineName(devices, device.id, t),
        // Every computer runs a core of its own once two Hides connect, so no row is marked as the core's (PRD device-connect-iroh D-80).
        core: !pairs && isCoreMachine(devices, device.id),
        own: device.id === own,
        windowIsNode,
        coreName,
      }}
      actions={actions}
      handlers={handlers}
    />
  );
  // The list stands in three groups (operator review 2026-11-22): this
  // computer, the Hides it is paired with, and the machines it reaches over
  // SSH or WSL; one Add device above them adds either kind.
  // This window's own machine leads, even where it is a node that dials in (B16).
  const localRows = listed.filter((device) => device.id === own || device.kind !== "remote");
  const sshRows = listed.filter((device) => !localRows.includes(device));
  const sshTitle = sshRows.some((device) => device.wsl_distro) ? t("devices.section.sshWsl") : t("devices.section.ssh");
  return (
    <>
      {canAdd ? (
        <div className="mb-sm flex justify-end">
          <Button variant="secondary" size="sm" onClick={() => setAdding(rows.length)} data-device-add-open="true">
            <PlusIcon aria-hidden="true" />
            {t("devices.addTitle")}
          </Button>
        </div>
      ) : null}
      <Group title={t("devices.section.thisComputer")} data-device-section="this">
        {localRows.map(row)}
        {deviceError ? <Row label={<Note tone="error" data-device-error="true" data-core-error={deviceError.kind}>{coreErrorText(deviceError, t)}</Note>} /> : null}
      </Group>
      {pairs ? <PairedRows actions={actions} /> : null}
      <Group title={sshTitle} data-device-section="ssh">
        {sshRows.map(row)}
        {notAdded ? (
          <Row
            label={
              <Note tone="warn" data-device-not-added={notAdded.reason}>
                {t("devices.notAdded", { name: notAdded.label })} · {t(switchFailureKey(notAdded.reason))}
              </Note>
            }
          />
        ) : null}
        {sshRows.length === 0 && !notAdded ? <Row label={<Note>{t("devices.noRemote")}</Note>} /> : null}
      </Group>
      {adding !== null && canAdd ? (
        <Dialog open onOpenChange={(next) => { if (!next) setAdding(null); }}>
          <DialogContent showCloseButton data-add-device-dialog="true">
            <DialogHeader>
              <DialogTitle>{t("devices.addTitle")}</DialogTitle>
            </DialogHeader>
            <DialogBody className="space-y-md">
              {/* Which kind of machine is added: one that runs its own Hide, by its link, or one reached over SSH or WSL (PRD device-connect-iroh B2, B7). */}
              {pairs ? (
                <ToggleGroup type="single" value={addKind} onValueChange={(next) => { if (next) setAddKind(next as AddKind); }} aria-label={t("devices.addTitle")} className="w-full" data-add-kind={addKind}>
                  <ToggleGroupItem value="hide" size="sm" className="flex-1" data-add-kind-choice="hide">{t("pairing.add.kindHide")}</ToggleGroupItem>
                  <ToggleGroupItem value="ssh" size="sm" className="flex-1" data-add-kind-choice="ssh">{t("pairing.add.kindSsh")}</ToggleGroupItem>
                </ToggleGroup>
              ) : null}
              {pairs && addKind === "hide" ? (
                <AddByLink actions={actions} onStarted={() => setAdding(null)} />
              ) : (
                <AddDevice actions={actions} devices={rows} helperRoot={localRoot} cliDir={localCliDir} onPaired={() => setAdding(null)} />
              )}
            </DialogBody>
          </DialogContent>
        </Dialog>
      ) : null}
      {disconnecting ? (
        <AlertDialog open onOpenChange={(next) => { if (!next) setDisconnecting(false); }}>
          <AlertDialogContent data-core-disconnect-confirm="true">
            <AlertDialogHeader>
              <AlertDialogTitle>{t("coreLink.disconnectTitle", { machine: coreName })}</AlertDialogTitle>
              <AlertDialogDescription>{t("coreLink.disconnectDescription", { machine: coreName })}</AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
              <AlertDialogAction data-core-disconnect-go="true" onClick={() => actions.coreLink("disconnect")}>
                {t("coreLink.disconnectGo")}
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
