import { MoreHorizontalIcon } from "lucide-react";
import type { Actions } from "../actions";
import { Button } from "../components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { Note, Row, Status } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import { formatDateTime } from "../i18n/format";
import { requireInterfaceLanguage } from "../i18n/locale";
import { canRetryDevice, deviceLine, deviceProblemLine, deviceReason, kitProblems, retrySignsIn } from "../settings";
import type { Device, RemoteStatus } from "../snapshot";
import { useShellStore } from "../store";
import { ownWindow } from "../pairing";
import { splitFailureLine } from "../coreLink";
import { DeviceSignIn } from "./SignInPrompt";
import { FixCommand } from "./FixCommand";
import { KitProblem } from "./MachineKit";
import { machineSystem } from "./agentRows";
import { SmartAppControlHint } from "./SmartAppControlHint";

/** What a row's menu and buttons ask the tab to open or record. */
export type DeviceRowHandlers = {
  /** Records that the operator just acted, so the tab reads the refusal that follows. */
  onAct: () => void;
  onDetails: (device: Device) => void;
  onRename: (device: Device) => void;
  onAllow: (device: Device) => void;
  onRevoke: (device: Device) => void;
  onRemove: (device: Device) => void;
  /** From a node's window: asks to end its link to the core (B16). */
  onDisconnect: () => void;
  /** This computer's own row: opens the share window. */
  onShare: () => void;
};

/**
 * Where a row stands for this window (PRD core-host-node-move B2): its name
 * as this window calls it, whether it is the window's own machine, whether
 * that machine is a node, and the core machine's name for the node's menu.
 */
export type DeviceRowPlace = { name: string; own: boolean; windowIsNode: boolean; coreName: string };

/**
 * One machine of Settings › Devices that is not a paired Hide (operator
 * review 2026-11-23): this computer, or a device reached over SSH or WSL.
 * A row says its name and system and one state; a device that is not
 * connected says why in one plain line, and only what the operator acts on
 * grows under it (a sign-in, a changed
 * server key, the helper's consent, a part of Hide's kit). Test, the
 * technical detail and the rename are in ⋯.
 */
export function DeviceRow({ device, status, place, actions, handlers }: { device: Device; status: RemoteStatus | undefined; place: DeviceRowPlace; actions: Actions; handlers: DeviceRowHandlers }) {
  const { t } = useInterfaceTranslation();
  const ownScreen = useShellStore((s) => ownWindow(s.pairing));
  const corePlatform = useShellStore((s) => s.rest?.navigator?.devices?.find((row) => row.kind !== "remote")?.host?.platform ?? null);
  // A node that dials in has no SSH connection to retry here.
  const dialed = device.kind === "remote" && device.dials_in !== true;
  const reached = dialed && !place.own;
  // A WSL distribution is named where it is, the core's PC (PRD wsl-device B5).
  const system = device.wsl_distro ? place.coreName : machineSystem(device);
  const line = deviceLine(device, status, t);
  const reason = reached ? deviceReason(device, status, t, corePlatform) : null;
  // One button: a sign-in the device waits on, else Retry where trying again can help.
  const retry = dialed && canRetryDevice(device, status);
  return (
    <Row
      data-device-row={device.id}
      label={
        <span className="flex min-w-0 flex-col">
          <span className="break-words font-semibold" data-device-name={device.id}>{place.name}</span>
          {system ? (
            <span className="break-words text-caption text-muted-foreground" data-device-subtitle={device.id}>
              {system}
            </span>
          ) : null}
          {reason ? (
            <Note tone="warn" data-device-reason={device.id}>
              {reason}
              {device.problem === "windows_not_started" ? <> · <SmartAppControlHint deviceId={device.id} /></> : null}
            </Note>
          ) : null}
        </span>
      }
      detail={<DeviceAttention device={device} place={place} actions={actions} onAct={handlers.onAct} onAllow={handlers.onAllow} />}
    >
      {place.own ? null : (
        <Status tone={line.tone === "ok" ? "live" : line.tone} data-device-state={`${device.id}:${device.state}`}>
          {line.text}
        </Status>
      )}
      {place.own && ownScreen ? (
        <Button variant="secondary" size="sm" onClick={handlers.onShare} data-pair-share-make="true">
          {t("devices.share")}
        </Button>
      ) : null}
      {retry ? (
        <Button
          variant="secondary"
          size="sm"
          onClick={() => {
            handlers.onAct();
            actions.retryDevice(device.id);
          }}
          data-device-retry={device.id}
        >
          {retrySignsIn(device.problem) ? t("devices.signIn.start") : t("common.retry")}
        </Button>
      ) : null}
      <DeviceMenu device={device} place={place} actions={actions} handlers={handlers} ownScreen={ownScreen} />
    </Row>
  );
}

/**
 * The row's ⋯: Rename, Test and Connection details, the helper's consent
 * for a device this core dials, and Remove; on a node, its own machine's
 * row offers the end of its link to the core (PRD core-host-node-move B16).
 */
function DeviceMenu({ device, place, actions, handlers, ownScreen }: { device: Device; place: DeviceRowPlace; actions: Actions; handlers: DeviceRowHandlers; ownScreen: boolean }) {
  const { t } = useInterfaceTranslation();
  const dialed = device.kind === "remote" && device.dials_in !== true;
  const remote = device.kind === "remote" && !place.own;
  const granted = device.host?.consent === "granted" && device.host.state !== "identity_changed";
  const nodeOwn = place.own && place.windowIsNode;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon-sm" aria-label={t("devices.moreAria", { name: place.name })} data-device-menu={device.id}>
          <MoreHorizontalIcon aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" data-device-menu-content={device.id}>
        {remote && ownScreen ? (
          <DropdownMenuItem onSelect={() => handlers.onRename(device)} data-device-rename={device.id}>
            {t("devices.renameMenu")}
          </DropdownMenuItem>
        ) : null}
        {dialed && !place.own ? (
          <DropdownMenuItem
            disabled={device.test?.state === "running"}
            onSelect={() => {
              handlers.onAct();
              actions.testDevice(device.id);
              handlers.onDetails(device);
            }}
            data-device-test={device.id}
          >
            {device.test?.state === "running" ? t("devices.testing") : t("devices.test")}
          </DropdownMenuItem>
        ) : null}
        <DropdownMenuItem onSelect={() => handlers.onDetails(device)} data-device-details={device.id}>
          {t("devices.detailsMenu")}
        </DropdownMenuItem>
        {dialed && !place.own ? (
          granted ? (
            <DropdownMenuItem onSelect={() => handlers.onRevoke(device)} data-device-host-revoke={device.id}>
              {t("devices.revokeHelperMenu")}
            </DropdownMenuItem>
          ) : (
            <DropdownMenuItem onSelect={() => handlers.onAllow(device)} data-device-host-allow={device.id}>
              {t("devices.allowInstallMenu")}
            </DropdownMenuItem>
          )
        ) : null}
        {nodeOwn ? <DropdownMenuSeparator /> : null}
        {nodeOwn ? (
          <DropdownMenuItem onSelect={() => handlers.onDisconnect()} data-device-core-disconnect={device.id}>
            {t("coreLink.disconnectMenu", { machine: place.coreName })}
          </DropdownMenuItem>
        ) : null}
        {remote ? <DropdownMenuSeparator /> : null}
        {remote ? (
          <DropdownMenuItem variant="destructive" onSelect={() => handlers.onRemove(device)} data-device-remove={device.id}>
            {t("devices.removeMenu")}
          </DropdownMenuItem>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * What grows under a device because the operator acts on it now: a node's
 * split that did not happen, a sign-in ssh
 * asks for, a changed server key with the command that checks it, the
 * helper's consent or its retry, and a part of Hide's kit. Why a device is
 * not connected is the row's one reason line; ssh's and the helper's own
 * words and a test's stages are Connection details' (`DeviceDetails`).
 */
function DeviceAttention({ device, place, actions, onAct, onAllow }: { device: Device; place: DeviceRowPlace; actions: Actions; onAct: () => void; onAllow: (device: Device) => void }) {
  const { t } = useInterfaceTranslation();
  // A node whose machine was not made a Hide of its own says why on its own row, with the Retry (PRD device-connect-iroh B45).
  const splitFailed = useShellStore((s) => (place.own && place.windowIsNode ? s.coreSplit : null));
  const remote = device.kind === "remote" && device.dials_in !== true;
  const signIn = remote ? (device.sign_in ?? null) : null;
  const host = device.host;
  const needsAllow = remote && host !== undefined && host.consent !== "this_machine" && (host.consent !== "granted" || host.state === "identity_changed");
  const helperRetry = remote && host?.state === "unavailable" && host.consent === "granted" && device.problem == null && signIn === null;
  const keyCommand = remote && device.state !== "ready" && device.problem === "host_key_changed" && device.ssh_alias ? `ssh ${device.ssh_alias}` : null;
  const kit = device.kit;
  const kitShown = kit !== undefined && (Boolean(kit.unavailable) || Boolean(kit.refused) || kit.busy || kitProblems(kit).length > 0);
  const rows = [
    splitFailed ? (
      <div key="split" className="flex flex-wrap items-center gap-x-sm gap-y-xs" data-device-split={splitFailed}>
        <Status tone="warn">{splitFailureLine(splitFailed, place.coreName, t)}</Status>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => {
            onAct();
            actions.coreLink("retry");
          }}
          data-device-split-retry={device.id}
        >
          {t("common.retry")}
        </Button>
      </div>
    ) : null,
    signIn ? <DeviceSignIn key={`sign-in-${signIn.question}`} deviceId={device.id} wait={signIn} actions={actions} /> : null,
    keyCommand ? (
      <div key="key" className="flex flex-wrap items-center gap-x-sm gap-y-xxs" data-device-problem={`${device.id}:host_key_changed`}>
        <span className="text-caption text-muted-foreground">{t("devices.problem.checkInTerminal")}</span>
        <FixCommand command={keyCommand} what="ssh check command" actions={actions} data-device-problem-command={device.id} />
      </div>
    ) : null,
    needsAllow || helperRetry ? (
      <div key="helper" className="flex flex-wrap items-center gap-x-sm gap-y-xs" data-device-host={`${device.id}:${host?.state ?? "unknown"}`}>
        {helperRetry ? (
          <Button
            variant="secondary"
            size="sm"
            onClick={() => {
              onAct();
              actions.retryDeviceHost(device.id);
            }}
            data-device-host-retry={device.id}
          >
            {t("devices.retryHelper")}
          </Button>
        ) : null}
        {needsAllow ? (
          <Button variant="secondary" size="sm" onClick={() => onAllow(device)} data-device-host-allow-inline={device.id}>
            {t("devices.allowInstallMenu")}
          </Button>
        ) : null}
      </div>
    ) : null,
    kitShown ? <KitProblem key="kit" device={device} actions={actions} onAct={onAct} /> : null,
  ].filter((row) => row !== null);
  return rows.length === 0 ? null : <div className="space-y-xs">{rows}</div>;
}

/** Whether a device's ssh asks the operator something now, or stopped on why it did not connect. */
export function deviceLinkShown(device: Device): boolean {
  const remote = device.kind === "remote" && device.dials_in !== true;
  return remote && (device.sign_in != null || (device.state !== "ready" && device.problem != null));
}

/**
 * What a dialed device's ssh asks the operator now, or, once an attempt
 * stopped, one of its three states (PRD system-ssh-device-link B4-B13): a
 * sign-in, a changed server key with the `ssh <alias>` a terminal checks it
 * with, or a connection that could not be made over ssh's last line. The
 * device's row and the first-run step's row each draw it; the action beside
 * it (Sign in, Retry, none for a changed key) is the row's own.
 */
export function DeviceLink({ device, actions }: { device: Device; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  // ssh runs on the core's machine, so that system says how to install it (B20).
  const corePlatform = useShellStore((s) => s.rest?.navigator?.devices?.find((row) => row.kind !== "remote")?.host?.platform ?? null);
  const remote = device.kind === "remote" && device.dials_in !== true;
  const signIn = remote ? (device.sign_in ?? null) : null;
  // An attempt waiting on the operator shows that wait, not the last one's failure.
  if (signIn) return <DeviceSignIn key={`sign-in-${signIn.question}`} deviceId={device.id} wait={signIn} actions={actions} />;
  const problem = remote && device.state !== "ready" ? deviceProblemLine(device.problem, device.ssh_alias, t, corePlatform) : null;
  if (!problem) return null;
  // A connection that could not be made shows ssh's last line under its
  // headline, and a failure that is not ssh's keeps its own words; a sign-in
  // or a changed server key needs none of ssh's, which stay in the log.
  const sshLine = device.problem === "cannot_connect" ? device.message : null;
  return (
    <div className="space-y-xxs" data-device-problem={`${device.id}:${device.problem}`}>
      <Status tone="warn">{problem.headline}</Status>
      {problem.note ? <Note tone="warn">{problem.note}</Note> : null}
      {sshLine ? <p className="break-words font-mono text-caption text-muted-foreground" data-device-message={device.id}>{sshLine}</p> : null}
      {problem.command ? (
        <div className="flex flex-wrap items-center gap-x-sm gap-y-xxs">
          <span className="text-caption text-muted-foreground">{t("devices.problem.checkInTerminal")}</span>
          <FixCommand command={problem.command} what="ssh check command" actions={actions} data-device-problem-command={device.id} />
        </div>
      ) : null}
    </div>
  );
}

export function DeviceTest({ test, language }: { test: NonNullable<Device["test"]>; language: ReturnType<typeof requireInterfaceLanguage> }) {
  const { t } = useInterfaceTranslation();
  const tone = test.state === "running" ? "pending" : test.state === "passed" ? "ok" : "warn";
  // A finished test names when it ran: it is that attempt's result, and it
  // stays beside the row after the connection itself has changed.
  const time = test.checked_at_unix_ms === null ? null : formatDateTime(language, test.checked_at_unix_ms, { timeStyle: "medium" });
  const headline =
    test.state === "running"
      ? t("devices.testRunning")
      : test.state === "passed"
        ? time === null ? t("devices.testPassed") : t("devices.testPassedAt", { time })
        : time === null ? t("devices.testFailed") : t("devices.testFailedAt", { time });
  return (
    <div className="space-y-xxs" data-device-test-state={test.state}>
      <Status tone={tone}>{headline}</Status>
      {test.stages.map((stage) => (
        <div key={stage.stage} className="flex gap-xs text-caption">
          <span className={stage.state === "passed" ? "text-success" : stage.state === "pending" ? "text-muted-foreground" : "text-warning"} aria-hidden="true">
            {stage.state === "passed" ? "✓" : stage.state === "pending" ? "…" : "✕"}
          </span>
          <span className="sr-only">{stage.state === "passed" ? t("devices.stage.passed") : stage.state === "pending" ? t("devices.stage.notRun") : t("devices.stage.failed")}</span>
          <span className="w-[var(--size-device-test-stage-col)] shrink-0 font-mono text-foreground">{stage.stage}</span>
          <span className="min-w-0 break-words text-subtle-foreground">{stage.detail}</span>
        </div>
      ))}
    </div>
  );
}
