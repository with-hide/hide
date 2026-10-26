import { CrownIcon, MoreHorizontalIcon } from "lucide-react";
import type { Actions } from "../actions";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { Note, Row, Status } from "../components/settings-rows";
import { Hint } from "../components/ui/tooltip";
import { useInterfaceTranslation } from "../i18n/client";
import { formatDateTime } from "../i18n/format";
import { requireInterfaceLanguage } from "../i18n/locale";
import { canRetryDevice, deviceLine, deviceProblemLine, deviceSubtitle, hostLine, kitProblems } from "../settings";
import type { Device, RemoteStatus } from "../snapshot";
import { KitProblem } from "./MachineKit";

/** What a row's menu and buttons ask the tab to open or record. */
export type DeviceRowHandlers = {
  /** Records that the operator just acted, so the tab reads the refusal that follows. */
  onAct: () => void;
  onDetails: (device: Device) => void;
  onAllow: (device: Device) => void;
  onRevoke: (device: Device) => void;
  onRemove: (device: Device) => void;
  /** Opens the move of the core to this device (PRD core-host-node-move B2). */
  onMove: (device: Device) => void;
  /** From a node's window: opens the move of the core back to this machine. */
  onMoveBack: () => void;
  /** From a node's window: asks to end its link to the core (B16). */
  onDisconnect: () => void;
};

/**
 * Where a row stands for this window (PRD core-host-node-move B2): its name
 * as this window calls it, whether it runs the core, whether it is the
 * window's own machine, whether that machine is a node, whether the core
 * reads it now, and the core machine's name for the node's menu.
 */
export type DeviceRowPlace = { name: string; core: boolean; own: boolean; windowIsNode: boolean; connected: boolean; coreName: string };

/**
 * One device (PRD settings-cleanup B54, B55): its name, alias, platform, Herdr
 * version and connection state on one line with Test and the ⋯ menu. A line
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
  const subtitle = device.kind !== "remote" && place.windowIsNode ? (device.host?.platform ?? "") : deviceSubtitle(device, status, t);
  return (
    <Row
      data-device-row={device.id}
      label={
        <span className="flex min-w-0 flex-col">
          <span className="flex min-w-0 flex-wrap items-center gap-xs">
            <span className="break-words font-semibold" data-device-name={device.id}>{place.name}</span>
            {place.core ? (
              <Hint label={t("coreMove.runs")}>
                <Badge variant="outline" data-device-core={device.id}>
                  <CrownIcon aria-hidden="true" />
                  {t("coreMove.badge")}
                </Badge>
              </Hint>
            ) : null}
          </span>
          {subtitle ? (
            <span className="break-all font-mono text-caption text-muted-foreground" data-device-subtitle={device.id}>
              {subtitle}
            </span>
          ) : null}
        </span>
      }
      detail={<DeviceAttention device={device} actions={actions} onAct={handlers.onAct} onAllow={handlers.onAllow} />}
    >
      <Status tone={line.tone} data-device-state={`${device.id}:${device.state}`}>
        {line.text}
      </Status>
      {focused ? <Status tone="muted">{t("devices.selected")}</Status> : null}
      {remote && canRetryDevice(device, status) ? (
        <Button
          variant="secondary"
          onClick={() => {
            handlers.onAct();
            actions.retryDevice(device.id);
          }}
          data-device-retry={device.id}
        >
          {t("common.retry")}
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
 * The row's ⋯ (PRD core-host-node-move B2, B16): a device this core dials
 * offers to take the core, dimmed with why while it is not connected; the
 * window's own machine, on a node, offers the core back and the end of its
 * link. The core moves only from the core's own window (issue 951).
 */
function DeviceMenu({ device, focused, place, actions, handlers }: { device: Device; focused: boolean; place: DeviceRowPlace; actions: Actions; handlers: DeviceRowHandlers }) {
  const { t } = useInterfaceTranslation();
  const dialed = device.kind === "remote" && device.dials_in !== true;
  const remote = device.kind === "remote" && !place.own;
  const granted = device.host?.consent === "granted" && device.host.state !== "identity_changed";
  const movable = dialed && !place.windowIsNode;
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
        {movable ? <DropdownMenuSeparator /> : null}
        {movable ? (
          <DropdownMenuItem disabled={!place.connected} onSelect={() => handlers.onMove(device)} className="flex-col items-start gap-0" data-device-move={device.id}>
            <span>{t("coreMove.moveHere")}</span>
            {place.connected ? null : <span className="text-caption text-muted-foreground">{t("devices.rail.notConnected")}</span>}
          </DropdownMenuItem>
        ) : null}
        {nodeOwn ? <DropdownMenuSeparator /> : null}
        {nodeOwn ? (
          <DropdownMenuItem onSelect={() => handlers.onMoveBack()} data-device-move-back={device.id}>
            {t("coreMove.moveBackMenu")}
          </DropdownMenuItem>
        ) : null}
        {nodeOwn ? (
          <DropdownMenuItem onSelect={() => handlers.onDisconnect()} data-device-core-disconnect={device.id}>
            {t("coreMove.disconnectMenu", { machine: place.coreName })}
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
function DeviceAttention({ device, actions, onAct, onAllow }: { device: Device; actions: Actions; onAct: () => void; onAllow: (device: Device) => void }) {
  const { t, i18n } = useInterfaceTranslation();
  // SSH trust, sign-in and helper consent belong to a device this core dials, not to a node that dials in.
  const remote = device.kind === "remote" && device.dials_in !== true;
  const problem = remote && device.state !== "ready" ? deviceProblemLine(device.problem, device.ssh_alias, t) : null;
  const host = device.host;
  const needsAllow = host !== undefined && (host.consent !== "granted" || host.state === "identity_changed");
  // Consent starts the helper that installs or updates Herdr, including
  // when Herdr cannot yet supply a readable session.
  const helper = remote && (device.state === "ready" || needsAllow) && host && host.consent !== "this_machine" && host.state !== "ready" ? hostLine(host, t) : null;
  const kit = device.kit;
  const kitShown = kit !== undefined && (Boolean(kit.unavailable) || Boolean(kit.refused) || kit.busy || kitProblems(kit).length > 0);
  const rows = [
    problem ? (
      <div key="problem" className="space-y-xxs" data-device-problem={`${device.id}:${device.problem}`}>
        <Status tone="warn">{problem.headline}</Status>
        <Note tone="warn">{problem.action}</Note>
      </div>
    ) : null,
    remote && device.state !== "ready" && device.message ? (
      <p key="message" className="break-words font-mono text-caption text-muted-foreground">
        {device.message}
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
    // The line is about the server last reached; an unreachable device says that instead.
    remote && device.state === "ready" && device.herdr_behind ? (
      <div key="herdr" className="space-y-xxs" data-device-herdr-behind={`${device.id}:${device.herdr_behind.cause}`}>
        <Status tone="warn">{t("devices.herdrBehind", { server: device.herdr_behind.server_version, version: device.herdr_behind.version })}</Status>
        <Note tone="warn">
          {device.herdr_behind.cause === "cannot_hand_off" ? t("devices.herdrBehindUpdate") : t("devices.herdrBehindRestart", { version: device.herdr_behind.version })}
        </Note>
      </div>
    ) : null,
    remote && kit?.herdr_start_failed ? (
      <div key="herdr-start" className="flex flex-wrap items-center gap-x-sm gap-y-xs" data-device-herdr-start-failed={device.id}>
        <Status tone="warn">{t("devices.herdrStartFailed")}</Status>
        <Button
          variant="secondary"
          size="sm"
          disabled={kit.busy}
          onClick={() => {
            onAct();
            actions.reinstallKit(device.id);
          }}
          data-device-herdr-start-retry={device.id}
        >
          {t("common.retry")}
        </Button>
      </div>
    ) : null,
    kitShown ? <KitProblem key="kit" device={device} actions={actions} onAct={onAct} /> : null,
    device.test ? <DeviceTest key="test" test={device.test} language={requireInterfaceLanguage(i18n.language)} /> : null,
  ].filter((row) => row !== null);
  return rows.length === 0 ? null : <div className="space-y-xs">{rows}</div>;
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
