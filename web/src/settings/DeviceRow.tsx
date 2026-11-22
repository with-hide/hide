import { MoreHorizontalIcon } from "lucide-react";
import type { Actions } from "../actions";
import { Button } from "../components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { Note, Row, Status } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import { formatDateTime } from "../i18n/format";
import { requireInterfaceLanguage } from "../i18n/locale";
import { canRetryDevice, deviceLine, deviceProblemLine, deviceSubtitle, hostLine, kitProblems, retrySignsIn } from "../settings";
import type { Device, RemoteStatus } from "../snapshot";
import { useShellStore } from "../store";
import { switchOffered } from "../deviceSwitch";
import { ownWindow } from "../pairing";
import { splitFailureLine } from "../coreLink";
import { DeviceSwitchButton, DeviceSwitchLine } from "./DeviceSwitch";
import { DeviceSignIn } from "./SignInPrompt";
import { FixCommand } from "./FixCommand";
import { KitProblem } from "./MachineKit";

/** What a row's menu and buttons ask the tab to open or record. */
export type DeviceRowHandlers = {
  /** Records that the operator just acted, so the tab reads the refusal that follows. */
  onAct: () => void;
  onDetails: (device: Device) => void;
  onAllow: (device: Device) => void;
  onRevoke: (device: Device) => void;
  onRemove: (device: Device) => void;
  /** From a node's window: asks to end its link to the core (B16). */
  onDisconnect: () => void;
};

/**
 * Where a row stands for this window (PRD core-host-node-move B2): its name
 * as this window calls it, whether it runs the core, whether it is the
 * window's own machine, whether that machine is a node, whether the core
 * reads it now, and the core machine's name for the node's menu.
 */
export type DeviceRowPlace = { name: string; own: boolean; windowIsNode: boolean; coreName: string };

/**
 * One device (PRD settings-cleanup B54, B55): its name, alias, platform
 * and connection state on one line with Test and the ⋯ menu. A line
 * grows under it only when something needs the operator: a refused connection,
 * a helper that is not allowed or not running, a part of Hide's kit that
 * failed, or the result of a test they asked for.
 */
export function DeviceRow({ device, status, focused, place, actions, handlers }: { device: Device; status: RemoteStatus | undefined; focused: boolean; place: DeviceRowPlace; actions: Actions; handlers: DeviceRowHandlers }) {
  const { t } = useInterfaceTranslation();
  const line = deviceLine(device, status, t);
  // A node that dials in has no SSH connection to test or retry here.
  const remote = device.kind === "remote" && device.dials_in !== true;
  // The core's machine seen from a node: its platform, since "local" would name the window's machine.
  const subtitle = device.kind !== "remote" && place.windowIsNode ? (device.host?.platform ?? "") : deviceSubtitle(device, t, place.coreName);
  // A helper device says it is on the old connection and offers the switch
  // (B35), from this computer's own window only (B52).
  const ownScreen = useShellStore((s) => ownWindow(s.pairing));
  const switchable = ownScreen && switchOffered(device);
  return (
    <Row
      data-device-row={device.id}
      label={
        <span className="flex min-w-0 flex-col">
          <span className="flex min-w-0 flex-wrap items-center gap-xs">
            <span className="break-words font-semibold" data-device-name={device.id}>{place.name}</span>
          </span>
          {subtitle ? (
            <span className="break-all font-mono text-caption text-muted-foreground" data-device-subtitle={device.id}>
              {subtitle}
            </span>
          ) : null}
        </span>
      }
      detail={<DeviceAttention device={device} place={place} actions={actions} switchable={switchable} onAct={handlers.onAct} onAllow={handlers.onAllow} />}
    >
      <Status tone={line.tone} data-device-state={`${device.id}:${device.state}`}>
        {line.text}
      </Status>
      {focused ? <Status tone="muted">{t("devices.selected")}</Status> : null}
      {switchable ? <Status tone="muted" data-device-old-link={device.id}>{t("devices.switch.old")}</Status> : null}
      {switchable && !device.switch ? <DeviceSwitchButton device={device} actions={actions} /> : null}
      {remote && canRetryDevice(device, status) ? (
        <Button
          variant="secondary"
          onClick={() => {
            handlers.onAct();
            actions.retryDevice(device.id);
          }}
          data-device-retry={device.id}
        >
          {retrySignsIn(device.problem) ? t("devices.signIn.start") : t("common.retry")}
        </Button>
      ) : null}
      {remote ? (
        <Button
          variant="secondary"
          disabled={device.test?.state === "running"}
          onClick={() => {
            handlers.onAct();
            actions.testDevice(device.id);
          }}
          data-device-test={device.id}
        >
          {device.test?.state === "running" ? t("devices.testing") : t("devices.test")}
        </Button>
      ) : null}
      <DeviceMenu device={device} focused={focused} place={place} actions={actions} handlers={handlers} />
    </Row>
  );
}

/**
 * The row's ⋯ (PRD core-host-node-move B16): the window's own machine, on a
 * node, offers the end of its link to the core.
 */
function DeviceMenu({ device, focused, place, actions, handlers }: { device: Device; focused: boolean; place: DeviceRowPlace; actions: Actions; handlers: DeviceRowHandlers }) {
  const { t } = useInterfaceTranslation();
  const dialed = device.kind === "remote" && device.dials_in !== true;
  const remote = device.kind === "remote" && !place.own;
  const granted = device.host?.consent === "granted" && device.host.state !== "identity_changed";
  const nodeOwn = place.own && place.windowIsNode;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon" aria-label={t("devices.moreAria", { name: place.name })} data-device-menu={device.id}>
          <MoreHorizontalIcon aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" data-device-menu-content={device.id}>
        {focused ? null : (
          <DropdownMenuItem onSelect={() => actions.focusDevice(device.id)} data-device-select={device.id}>
            {t("common.select")}
          </DropdownMenuItem>
        )}
        <DropdownMenuItem onSelect={() => handlers.onDetails(device)} data-device-details={device.id}>
          {t("devices.detailsMenu")}
        </DropdownMenuItem>
        {dialed ? (
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

/** The lines under a device that something needs the operator for; a healthy device has none. */
function DeviceAttention({ device, place, actions, switchable, onAct, onAllow }: { device: Device; place: DeviceRowPlace; actions: Actions; switchable: boolean; onAct: () => void; onAllow: (device: Device) => void }) {
  const { t, i18n } = useInterfaceTranslation();
  // A node whose machine was not made a Hide of its own says why on its own row, with the Retry (PRD device-connect-iroh B45).
  const splitFailed = useShellStore((s) => (place.own && place.windowIsNode ? s.coreSplit : null));
  // SSH trust, sign-in and helper consent belong to a device this core dials, not to a node that dials in.
  const remote = device.kind === "remote" && device.dials_in !== true;
  const signIn = remote ? (device.sign_in ?? null) : null;
  const host = device.host;
  const needsAllow = host !== undefined && (host.consent !== "granted" || host.state === "identity_changed");
  // Consent starts the helper that installs or updates Hide's kit, including
  // when the engine cannot yet supply a readable session. A failed helper kit
  // must keep its reason and Retry visible before that session connects.
  // An unsupported platform keeps its reason visible too, without a Retry.
  // The helper is the link, so an ssh failure, an unsupported system ssh
  // reached or a sign-in is its line, said once below.
  const linkSays = signIn !== null || device.problem != null;
  const helper = remote && (device.state === "ready" || needsAllow || ((host?.state === "unavailable" || host?.state === "unsupported") && !linkSays)) && host && host.consent !== "this_machine" && host.state !== "ready" ? hostLine(host, t) : null;
  const ownWords = remote && device.state !== "ready" && !signIn && device.message && !device.problem && !(helper && device.message === host?.message) ? device.message : null;
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
    switchable && device.switch ? <DeviceSwitchLine key="switch" device={device} actions={actions} /> : null,
    deviceLinkShown(device) ? <DeviceLink key="link" device={device} actions={actions} /> : null,
    ownWords ? (
      <p key="message" className="break-words font-mono text-caption text-muted-foreground" data-device-message={device.id}>
        {ownWords}
      </p>
    ) : null,
    helper ? (
      <div key="helper" className="flex flex-wrap items-center gap-x-sm gap-y-xs" data-device-host={`${device.id}:${host?.state ?? "unknown"}`}>
        <Status tone={helper.tone}>{helper.text}</Status>
        {host?.state === "unavailable" ? (
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
        {host?.message ? <Note tone={helper.tone === "muted" ? "muted" : "warn"}>{host.message}</Note> : null}
      </div>
    ) : null,
    kitShown ? <KitProblem key="kit" device={device} actions={actions} onAct={onAct} /> : null,
    device.test ? <DeviceTest key="test" test={device.test} language={requireInterfaceLanguage(i18n.language)} /> : null,
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

function DeviceTest({ test, language }: { test: NonNullable<Device["test"]>; language: ReturnType<typeof requireInterfaceLanguage> }) {
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
