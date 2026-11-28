import { ChevronRightIcon, LaptopIcon, PlusIcon, SmartphoneIcon } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Actions } from "../actions";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "../components/ui/alert-dialog";
import { Button } from "../components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { Group, Note, Row } from "../components/settings-rows";
import { RowButton } from "../components/ui/row-button";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { latestDraft } from "../editor/draft";
import { Trans } from "react-i18next";
import { useInterfaceTranslation } from "../i18n/client";
import { coreMachineName, isCoreMachine, machineName, registeredName, windowDeviceId, windowFirst } from "../screenMachine";
import { deviceRemovalLines, draftExported, kitRemovalLine, unstoredDeviceDrafts } from "../settings";
import type { Device } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { AddByLink, AddDevice } from "./AddDevice";
import { DeviceDetails } from "./DeviceDetails";
import { DeviceRow, type DeviceRowHandlers } from "./DeviceRow";
import { PeerRows, ShareDialog } from "./PairingRows";
import { listedDevices, ownWindow, pairedMachines } from "../pairing";
import { openPractice } from "../practice/channel";
import { PhoneGroup } from "./PhoneRows";
import { RenameDevice } from "./RenameDevice";
import { KitTerms } from "./MachineKit";
import { coreErrorText } from "../coreError";
import { useErrorSince } from "../errorSince";

type AddKind = "hide" | "ssh";
/** What the Add dialog shows: the question computer or phone, then a computer's two ways. */
type AddStep = "choose" | "computer";

export function DevicesTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const [removing, setRemoving] = useState<Device | null>(null);
  const [allowing, setAllowing] = useState<Device | null>(null);
  const [revoking, setRevoking] = useState<Device | null>(null);
  const [details, setDetails] = useState<Device | null>(null);
  const [renaming, setRenaming] = useState<Device | null>(null);
  const [sharing, setSharing] = useState(false);
  const [disconnecting, setDisconnecting] = useState(false);
  // The Add dialog, which closes itself once what it added is listed or connected.
  const [adding, setAdding] = useState(false);
  // Which kind of machine the Add dialog adds; it opens on a Hide's link.
  const [addKind, setAddKind] = useState<AddKind>("hide");
  const [addStep, setAddStep] = useState<AddStep>("choose");
  // The QR window, which Add device › 폰 opens: the one way to pair a phone (operator review 2026-11-23).
  const [pairingPhone, setPairingPhone] = useState(false);
  const phonesRef = useRef<HTMLDivElement>(null);
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
  // Adding a machine is this computer's own window's to do (B52).
  const ownScreen = useShellStore((s) => ownWindow(s.pairing));
  const canAdd = ownScreen;
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
  // Settings opened to add a device (the rail's `+`, Add a project's host
  // list, which adds a computer) or to a phone's settings: done once.
  const request = useUiStore((s) => s.devicesRequest);
  useEffect(() => {
    if (!request || devices === undefined) return;
    useUiStore.getState().clearDevicesRequest();
    if (request === "phones") {
      phonesRef.current?.scrollIntoView({ block: "start" });
      return;
    }
    setAddStep(request === "add_computer" ? "computer" : "choose");
    setAdding(true);
  }, [request, devices]);
  // hided rechecks the phone checklist while this machine's own window shows
  // the tab; showing it makes no pairing code and voids none (B58). A
  // restarted daemon has forgotten it, so every live connection says so again.
  useEffect(() => {
    if (!live || !ownScreen) return;
    actions.observeMobile(true);
    return () => actions.observeMobile(false);
  }, [actions, live, ownScreen]);
  useEffect(() => {
    if (adding) return;
    setAddKind("hide");
    setAddStep("choose");
  }, [adding]);
  // A row opened on how its device is connected is folded again when the tab closes, so the next opening reads its machine again.
  useEffect(() => () => useUiStore.getState().closeDevices(), []);
  const handlers: DeviceRowHandlers = {
    onAct: () => setActedAt(Date.now()),
    onDetails: setDetails,
    onRename: setRenaming,
    onAllow: setAllowing,
    onRevoke: setRevoking,
    onRemove: setRemoving,
    onDisconnect: () => setDisconnecting(true),
    onShare: () => setSharing(true),
  };
  const row = (device: Device) => (
    <DeviceRow
      key={device.id}
      device={device}
      status={remote?.find((line) => line.target_id === device.id)}
      place={{
        name: machineName(devices, device.id, t),
        own: device.id === own,
        windowIsNode,
        coreName,
      }}
      actions={actions}
      handlers={handlers}
    />
  );
  // The list stands in two groups (operator review 2026-11-23): this
  // computer, with its share; then every other machine, the Hides it is
  // paired with and the devices it reaches over SSH or WSL, with no mark
  // that tells the two apart. One Add device above them adds either kind.
  const ownRows = listed.filter((device) => device.id === own);
  const otherRows = listed.filter((device) => device.id !== own);
  const peerCount = useShellStore((s) => pairedMachines(s.rest).length);
  const ownName = machineName(devices, own, t);
  const empty = otherRows.length === 0 && peerCount === 0;
  return (
    <>
      <div className="mb-md flex items-start justify-between gap-md">
        <p className="min-w-0 break-keep text-body text-muted-foreground" data-devices-intro="true">
          {t("devices.intro", { machine: ownName })}{" "}
          <Button variant="text" size="inline" onClick={() => openPractice("connect", true)} data-devices-try="true">
            {t("devices.introTry")} ›
          </Button>
        </p>
        {canAdd ? (
          <Button variant="secondary" size="sm" className="shrink-0" onClick={() => { setAddStep("choose"); setAdding(true); }} data-device-add-open="true">
            <PlusIcon aria-hidden="true" />
            {t("devices.addTitle")}
          </Button>
        ) : null}
      </div>
      <Group title={t("devices.section.thisComputer")} data-device-section="this">
        {ownRows.map(row)}
        {deviceError ? <Row label={<Note tone="error" data-device-error="true" data-core-error={deviceError.kind}>{coreErrorText(deviceError, t)}</Note>} /> : null}
      </Group>
      <Group title={t("devices.section.other")} data-device-section="other">
        <PeerRows actions={actions} />
        {otherRows.map(row)}
        {empty ? <Row label={canAdd ? <FirstDeviceGuide machine={ownName} /> : <Note>{t("devices.empty.none")}</Note>} data-devices-empty="true" /> : null}
      </Group>
      <div ref={phonesRef} className="scroll-mt-md">
        <PhoneGroup actions={actions} own={ownScreen} pairing={pairingPhone} onPairingClose={() => setPairingPhone(false)} />
      </div>
      {sharing && ownScreen ? <ShareDialog actions={actions} onClose={() => setSharing(false)} /> : null}
      {renaming ? (
        <RenameDevice
          id={renaming.id}
          current={machineName(devices, renaming.id, t)}
          description={t("devices.renameDescription")}
          onSave={(name) => {
            setActedAt(Date.now());
            actions.renameDevice(renaming.id, name);
          }}
          onClose={() => setRenaming(null)}
        />
      ) : null}
      {adding && canAdd ? (
        <Dialog open onOpenChange={setAdding}>
          <DialogContent showCloseButton data-add-device-dialog="true">
            <DialogHeader>
              <DialogTitle>{t("devices.addTitle")}</DialogTitle>
            </DialogHeader>
            <DialogBody className="space-y-md">
              {addStep === "choose" ? (
                // A computer or a phone first (operator review 2026-11-23): a phone goes to the QR window, the one way to pair it.
                <div className="grid grid-cols-2 gap-sm" data-add-choose="true">
                  <AddChoice icon={<LaptopIcon aria-hidden="true" />} title={t("devices.addKind.computer")} line={t("devices.addKind.computerLine")} onPick={() => setAddStep("computer")} data-add-choice="computer" />
                  <AddChoice
                    icon={<SmartphoneIcon aria-hidden="true" />}
                    title={t("devices.addKind.phone")}
                    line={t("devices.addKind.phoneLine")}
                    onPick={() => {
                      setAdding(false);
                      setPairingPhone(true);
                    }}
                    data-add-choice="phone"
                  />
                </div>
              ) : null}
              {/* Which kind of machine is added: one that runs its own Hide, by its link, or one reached over SSH or WSL (PRD device-connect-iroh B2, B7). */}
              {addStep === "computer" ? <ToggleGroup type="single" value={addKind} onValueChange={(next) => { if (next) setAddKind(next as AddKind); }} aria-label={t("devices.addTitle")} className="w-full" data-add-kind={addKind}>
                <ToggleGroupItem value="hide" size="sm" className="flex-1" data-add-kind-choice="hide">{t("pairing.add.kindHide")}</ToggleGroupItem>
                <ToggleGroupItem value="ssh" size="sm" className="flex-1" data-add-kind-choice="ssh">{t("pairing.add.kindSsh")}</ToggleGroupItem>
              </ToggleGroup> : null}
              {addStep !== "computer" ? null : addKind === "hide" ? (
                <AddByLink actions={actions} onStarted={() => setAdding(false)} />
              ) : (
                <AddDevice
                  actions={actions}
                  devices={rows}
                  helperRoot={localRoot}
                  cliDir={localCliDir}
                  onClose={() => setAdding(false)}
                  onShowDevice={(id) => {
                    setAdding(false);
                    useUiStore.getState().openDevice(id);
                  }}
                />
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

/** One answer to Add device's question, computer or phone: its mark, its name and how it connects. */
function AddChoice({ icon, title, line, onPick, ...data }: { icon: ReactNode; title: string; line: string; onPick: () => void } & Record<`data-${string}`, string>) {
  return (
    <RowButton className="items-start gap-sm rounded-md border border-border bg-card px-md py-md [&_svg]:size-(--size-icon)" onClick={onPick} {...data}>
      <span className="mt-xxs shrink-0 text-muted-foreground">{icon}</span>
      <span className="flex min-w-0 flex-1 flex-col gap-xxs">
        <span className="text-subhead font-semibold text-foreground">{title}</span>
        <span className="break-keep text-body text-muted-foreground">{line}</span>
      </span>
      <ChevronRightIcon aria-hidden="true" className="mt-xxs shrink-0 text-muted-foreground" />
    </RowButton>
  );
}

/**
 * What the other devices' group says while there is none (operator review
 * 2026-11-23): three steps, the last that the other computer is then used
 * as this one is.
 */
function FirstDeviceGuide({ machine }: { machine: string }) {
  const { t, i18n } = useInterfaceTranslation();
  return (
    <div className="space-y-xs" data-devices-guide="true">
      <p className="text-body font-semibold text-foreground">{t("devices.empty.title")}</p>
      <ol className="list-decimal space-y-xxs pl-lg text-body text-muted-foreground">
        <li><Trans i18n={i18n} t={t} i18nKey="devices.empty.step1" components={{ b: <span className="font-semibold text-foreground" /> }} /></li>
        <li>{t("devices.empty.step2")}</li>
        <li>{t("devices.empty.step3", { machine })}</li>
      </ol>
    </div>
  );
}
