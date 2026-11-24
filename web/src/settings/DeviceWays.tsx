// What a device row of Settings › Devices opens on (operator decision
// 2026-11-24, Doop canvas 1N77v1reLZ): the two ways a device is connected,
// which one it uses now, what the other costs, and the condition its
// machine is in, beside the one button that switches it. Everything it
// draws is hided's answer or the core's row (`switchWays.ts`); it decides
// nothing about a switch itself.

import { CheckIcon, LoaderCircleIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Trans } from "react-i18next";
import type { Actions } from "../actions";
import { Note, Status } from "../components/settings-rows";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Popover, PopoverAnchor, PopoverContent, PopoverTrigger } from "../components/ui/popover";
import { Separator } from "../components/ui/separator";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import type { SwitchCondition } from "../pairing";
import type { DeviceSwitchStep } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { FixCommand } from "./FixCommand";
import { stepState, SWITCH_STEPS, switchButton, switchFailureKey, type SwitchLook, type SwitchOutcome } from "./switchWays";

/** The one word beside a device's name that says how it is connected. */
export function WayBadge({ way, deviceId }: { way: "ssh" | "wsl" | "server"; deviceId: string }) {
  const { t } = useInterfaceTranslation();
  return (
    <Badge variant="outline" data-device-way={`${deviceId}:${way}`}>
      {t(`devices.badge.${way}`)}
    </Badge>
  );
}

/**
 * How one device stands for its opened row: `way` is the connection it
 * uses now, `connected` whether its machine can be read, `step` the switch
 * the core says is running and `pressed` one this window just asked for.
 */
export type WaysState = {
  deviceId: string;
  name: string;
  way: "ssh" | "wsl" | "server";
  /** The machine runs Windows, which is not switched: one line stands in the box's place. */
  windows: boolean;
  connected: boolean;
  look: SwitchLook | null;
  step: DeviceSwitchStep | null;
  pressed: boolean;
  onRead: () => void;
  onSwitch: () => void;
};

/** The opened place of a device's row. */
export function DeviceWays({ state, actions }: { state: WaysState; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { deviceId, name, way } = state;
  const server = way === "server";
  const outcome = useUiStore((s) => s.switchOutcomes[deviceId] ?? null);
  const condition = state.look?.state === "read" ? state.look.condition : null;
  if (!server && (state.windows || condition?.kind === "windows")) {
    return <Note data-device-ways={`${deviceId}:windows`}>{t("devices.ways.windows")}</Note>;
  }
  return (
    <div className="space-y-sm" data-device-ways={`${deviceId}:${way}`}>
      <SwitchedNotice deviceId={deviceId} name={name} />
      <div className="rounded-md border border-border">
        <div className="grid grid-cols-[auto_minmax(0,1fr)] divide-y divide-border">
          <Way name={t(way === "wsl" ? "devices.ways.wsl" : "devices.ways.ssh")} line={t("devices.ways.sshLine")} now={!server} data-device-way-row="direct" />
          <Way name={t("devices.ways.server")} line={t("devices.ways.serverLine")} now={server} data-device-way-row="server" />
        </div>
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-md gap-y-xs border-t border-border px-md py-sm text-body">
          <dt className="text-muted-foreground">{t("devices.ways.instead")}</dt>
          <dd className="break-keep text-subtle-foreground">{t("devices.ways.insteadLine")}</dd>
          <dt className="text-muted-foreground">{t("devices.ways.condition")}</dt>
          <dd className="min-w-0" data-device-condition={deviceId}>
            {server ? <ServerCondition state={state} /> : <SwitchPlace state={state} outcome={outcome} actions={actions} />}
          </dd>
        </dl>
        {server ? (
          <div className="border-t border-border px-md py-xs">
            <Note data-device-ways-back={deviceId}>{t("devices.ways.back")}</Note>
          </div>
        ) : null}
      </div>
    </div>
  );
}

/** One way of the two: its name, the mark on the one in use, and when to choose it. */
function Way({ name, line, now, ...data }: { name: string; line: string; now: boolean } & Record<`data-${string}`, string>) {
  const { t } = useInterfaceTranslation();
  return (
    <div className={cn("col-span-2 grid grid-cols-subgrid gap-x-md px-md py-sm", now && "bg-secondary/50")} data-device-way-now={String(now)} {...data}>
      <div className="flex items-baseline gap-xs">
        <span className="text-body font-semibold text-foreground">{name}</span>
        {now ? <Badge>{t("devices.ways.now")}</Badge> : null}
      </div>
      <p className="break-keep text-body text-subtle-foreground">{line}</p>
    </div>
  );
}

/**
 * Said once at the top of a device that was just switched (D-48): what
 * starts fresh on its machine, and that its projects are that Hide's too.
 * `확인` ends it for this window.
 */
function SwitchedNotice({ deviceId, name }: { deviceId: string; name: string }) {
  const { t } = useInterfaceTranslation();
  const landed = useUiStore((s) => s.switchedDevices.includes(deviceId));
  const projects = useShellStore((s) => s.rest?.ui_state?.workspace_registrations?.filter((project) => project.device_id === deviceId).length ?? 0);
  if (!landed) return null;
  return (
    <div className="flex items-start justify-between gap-md rounded-md border border-border px-md py-sm" data-device-switched={deviceId}>
      <div className="min-w-0">
        <p className="break-keep text-body text-foreground">{t("devices.ways.fresh", { device: name })}</p>
        {projects > 0 ? <Note>{t("devices.ways.registered", { count: projects, device: name })}</Note> : null}
      </div>
      <Button variant="secondary" size="sm" className="shrink-0" onClick={() => useUiStore.getState().clearSwitched(deviceId)} data-device-switched-seen={deviceId}>
        {t("devices.ways.seen")}
      </Button>
    </div>
  );
}

/** The condition of a machine that was switched: what it is in now, with nothing to press. */
function ServerCondition({ state }: { state: WaysState }) {
  const { t } = useInterfaceTranslation();
  const { look, name } = state;
  if (look === null || look.state === "unread") return <Note>{t("devices.ways.whenConnected")}</Note>;
  if (look.state === "reading") return <Status tone="pending">{t("devices.ways.reading")}</Status>;
  const condition = look.condition;
  switch (condition.kind) {
    case "auto_login_on":
      return <Status tone="ok">{t("devices.ways.autoLoginOn")}</Status>;
    case "auto_login_off":
      // No button to open under: the steps open at 방법 itself.
      return (
        <Popover onOpenChange={(open) => { if (!open) state.onRead(); }}>
          <AutoLoginOff name={name} how={condition.how} />
          {condition.how ? <AutoLoginSteps name={name} user={look.user} /> : null}
        </Popover>
      );
    case "signed_out":
      return <Note tone="warn">{t("devices.ways.loginReconnects", { device: name })}</Note>;
    case "linger_on":
      return <Status tone="ok">{t("devices.ways.lingerOn")}</Status>;
    case "linger_off":
    case "linger_admin":
      return <Status tone="muted">{t("devices.ways.lingerOff")}</Status>;
    default:
      return <Note>{t("devices.ways.whenConnected")}</Note>;
  }
}

/**
 * The condition of a device that can be switched, with the button: the
 * steps while a switch runs, else how the last press ended, else what the
 * machine said and whether it can be pressed.
 */
function SwitchPlace({ state, outcome, actions }: { state: WaysState; outcome: SwitchOutcome | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { deviceId, name, look } = state;
  if (state.step !== null || state.pressed) return <SwitchSteps name={name} current={state.step ?? "checking"} />;
  const button = (label: "devices.ways.server" | "common.retry", disabled = false) => (
    <Button variant="secondary" size="sm" disabled={disabled} onClick={state.onSwitch} data-device-switch={deviceId}>
      {t(label)}
    </Button>
  );
  const recheck = (
    <Button variant="text" size="inline" onClick={state.onRead} data-device-condition-read={deviceId}>
      {t("devices.ways.recheck")}
    </Button>
  );
  if (outcome !== null) {
    const line =
      outcome.kind === "failed"
        ? t(switchFailureKey(outcome.reason), { device: name })
        : outcome.terminals === null
          ? t("devices.ways.heldUnread", { by: outcome.by, device: name })
          : t("devices.ways.held", { by: outcome.by, device: name, count: outcome.terminals });
    return (
      <Lines data-device-switch-outcome={`${deviceId}:${outcome.kind}`}>
        <Note tone="warn">{line}</Note>
        {button("common.retry")}
      </Lines>
    );
  }
  if (!state.connected) return <Note>{t("devices.ways.whenConnected")}</Note>;
  if (look === null || look.state === "reading") return <Status tone="pending">{t("devices.ways.reading")}</Status>;
  if (look.state === "unread") {
    // A login that asks for an answer is never switched; any other read that failed can be tried again.
    if (look.reason === "ssh_not_reached") return <Note tone="warn" data-device-condition-kind="password">{t("devices.ways.password")}</Note>;
    return (
      <Lines data-device-condition-kind="unread">
        <Note tone="warn">{t("devices.ways.unread")}</Note>
        {recheck}
      </Lines>
    );
  }
  const condition = look.condition;
  const pressable = switchButton(condition);
  // 방법 opens under the button, which stays in sight; closing it reads the
  // Mac again, so automatic login turned on meanwhile takes the warning away.
  return (
    <Popover onOpenChange={(open) => { if (!open) state.onRead(); }}>
      <Lines data-device-condition-kind={condition.kind}>
        <ConditionLine condition={condition} name={name} wsl={state.way === "wsl"} actions={actions} deviceId={deviceId} />
        {pressable === "none" ? null : (
          <PopoverAnchor asChild>
            <div className="flex flex-wrap items-center gap-sm">
              {button("devices.ways.server", pressable === "off")}
              {condition.kind === "linger_admin" ? recheck : null}
            </div>
          </PopoverAnchor>
        )}
      </Lines>
      {condition.kind === "auto_login_off" && condition.how ? <AutoLoginSteps name={name} user={look.user} /> : null}
    </Popover>
  );
}

/** The lines of the condition place, top to bottom. */
function Lines({ children, ...data }: { children: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <div className="flex flex-col items-start gap-xs" {...data}>
      {children}
    </div>
  );
}

/** What the machine said, in the words of its system. */
function ConditionLine({ condition, name, wsl, actions, deviceId }: { condition: SwitchCondition; name: string; wsl: boolean; actions: Actions; deviceId: string }) {
  const { t } = useInterfaceTranslation();
  switch (condition.kind) {
    case "auto_login_on":
      return <Status tone="ok">{t("devices.ways.autoLoginOn")}</Status>;
    case "auto_login_off":
      return <AutoLoginOff name={name} how={condition.how} />;
    case "signed_out":
      return <Note tone="warn">{t("devices.ways.signedOut", { device: name })}</Note>;
    case "linger_on":
      return <Status tone="ok">{t("devices.ways.lingerOn")}</Status>;
    case "linger_off":
      return (
        <>
          <Status tone="muted">{t("devices.ways.lingerOff")}</Status>
          <Note>{t("devices.ways.lingerTry")}</Note>
        </>
      );
    case "linger_admin":
      return (
        <>
          <Status tone="warn">{t("devices.ways.lingerAdmin")}</Status>
          <Note>{t("devices.ways.lingerRun", { device: name })}</Note>
          <FixCommand command={condition.command} what="linger command" actions={actions} data-device-linger-command={deviceId} />
        </>
      );
    case "no_systemd":
      return <Note tone="warn">{t(wsl ? "devices.ways.noSystemdWsl" : "devices.ways.noSystemd")}</Note>;
    case "windows":
    case "unknown":
      return <Note tone="warn">{t("devices.ways.unknown")}</Note>;
  }
}

/**
 * A Mac that does not log its account in by itself: the warning a switch
 * goes on with, and 방법, which opens how to turn automatic login on. A Mac
 * whose disk FileVault locks offers no automatic login, so it gets the
 * line without 방법.
 */
function AutoLoginOff({ name, how }: { name: string; how: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <Note tone="warn" data-device-auto-login="off">
      {t("devices.ways.autoLoginOff", { device: name })}
      {how ? (
        <>
          {" · "}
          <PopoverTrigger asChild>
            <Button variant="text" size="inline" data-device-auto-login-how="true">
              {t("devices.ways.how")}
            </Button>
          </PopoverTrigger>
        </>
      ) : null}
    </Note>
  );
}

/** How to turn automatic login on, on that Mac. */
function AutoLoginSteps({ name, user }: { name: string; user: string | null }) {
  const { t, i18n } = useInterfaceTranslation();
  const bold = { b: <span className="font-medium text-foreground" /> };
  return (
    <PopoverContent side="bottom" align="start" className="space-y-sm" data-device-auto-login-steps="true">
      <p className="text-body font-semibold text-foreground">{t("devices.ways.howTitle", { device: name })}</p>
      <ol className="list-decimal space-y-xxs pl-lg text-body text-subtle-foreground">
        <li><Trans i18n={i18n} t={t} i18nKey="devices.ways.howStep1" values={{ device: name }} components={bold} /></li>
        <li><Trans i18n={i18n} t={t} i18nKey="devices.ways.howStep2" components={bold} /></li>
        <li>{user ? <Trans i18n={i18n} t={t} i18nKey="devices.ways.howStep3" values={{ user }} components={bold} /> : <Trans i18n={i18n} t={t} i18nKey="devices.ways.howStep3NoUser" components={bold} />}</li>
      </ol>
      <Separator />
      <p className="text-caption text-muted-foreground">{t("devices.ways.howAfter")}</p>
    </PopoverContent>
  );
}

/** A running switch: each step hided runs, the ones behind checked, the one under way turning, the rest dim. */
function SwitchSteps({ name, current }: { name: string; current: DeviceSwitchStep }) {
  const { t } = useInterfaceTranslation();
  return (
    <ol aria-label={t("devices.ways.stepsAria")} className="flex flex-col gap-xs" data-device-switch-step={current}>
      {SWITCH_STEPS.map((step) => {
        const state = stepState(step, current);
        return (
          <li key={step} className={cn("flex items-center gap-sm text-body", state === "on" ? "font-medium text-foreground" : "text-muted-foreground", state === "later" && "opacity-(--opacity-dimmed)")} data-switch-step={`${step}:${state}`}>
            <span className="flex size-(--size-icon) shrink-0 items-center justify-center" aria-hidden="true">
              {state === "done" ? <CheckIcon className="size-(--size-icon) text-success" /> : state === "on" ? <LoaderCircleIcon className="size-(--size-icon) animate-spin" /> : <span className="size-(--size-subagent-dot) rounded-full bg-muted-foreground" />}
            </span>
            <span className="min-w-0 break-keep">{t(`devices.ways.step.${step}`, { device: name })}</span>
            <span className="sr-only">{t(state === "done" ? "devices.ways.stepDone" : state === "on" ? "devices.ways.stepOn" : "devices.ways.stepLater")}</span>
          </li>
        );
      })}
    </ol>
  );
}
