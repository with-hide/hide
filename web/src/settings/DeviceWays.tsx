// What a device row of Settings › Devices opens on (operator decision
// 2026-11-24, Doop canvas 1N77v1reLZ; drawn side by side 2026-11-26): the two
// ways a device is connected as two cards, which one it uses now, what each
// keeps doing while this computer sleeps, and the condition its machine is
// in, beside the one button that switches it. Everything it draws is hided's
// answer or the core's row (`switchWays.ts`); it decides nothing about a
// switch itself.

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
import { useOnScreen } from "../lib/useOnScreen";
import { cn } from "../lib/utils";
import type { SwitchCondition } from "../pairing";
import type { DeviceSwitchStep } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import "./deviceWays.css";
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
 * uses now, `direct` the login it is (or was, before a switch) reached
 * over, `connected` whether its machine can be read, `step` the switch the
 * core says is running and `pressed` one this window just asked for.
 */
export type WaysState = {
  deviceId: string;
  name: string;
  way: "ssh" | "wsl" | "server";
  direct: "ssh" | "wsl";
  /** The machine runs Windows, which is not switched: one line stands in the cards' place. */
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
  const [ref, playing] = useOnScreen<HTMLDivElement>();
  const condition = state.look?.state === "read" ? state.look.condition : null;
  if (!server && (state.windows || condition?.kind === "windows")) {
    return <Note data-device-ways={`${deviceId}:windows`}>{t("devices.ways.windows")}</Note>;
  }
  // A switch under way draws its steps in the server card, which takes the
  // outline from the card in use until the switch ends.
  const switching = !server && (state.step !== null || state.pressed);
  // A distribution runs on this PC and sleeps with it, so the picture of
  // what keeps running while this computer sleeps is not drawn for it.
  const sleeps = state.direct === "ssh";
  return (
    <div className="space-y-sm" data-device-ways={`${deviceId}:${way}`}>
      <SwitchedNotice deviceId={deviceId} name={name} />
      <div ref={ref} className="dw-ways @container/ways" data-playing={playing ? "true" : "false"}>
        <div className="grid gap-sm @lg/ways:grid-cols-2">
          <WayCard
            title={t(state.direct === "wsl" ? "devices.ways.wsl" : "devices.ways.ssh")}
            now={!server}
            lit={!server && !switching}
            data-device-way-row="direct"
            foot={
              server ? (
                <Note data-device-ways-back={deviceId}>{t("devices.ways.back")}</Note>
              ) : (
                <Note>{t(switching ? "devices.ways.switchingEnds" : "devices.ways.directTool")}</Note>
              )
            }
          >
            {sleeps ? <Headline lead={t("devices.ways.directHead")} turn={t("devices.ways.directHeadStop")} tone="warn" /> : <Note>{t("devices.ways.sshLine")}</Note>}
            {sleeps && !switching ? (
              <>
                <Picture way="direct" name={name} />
                <Keeps way="direct" name={name} />
                <p className="break-keep text-caption text-muted-foreground">{t("devices.ways.directFit")}</p>
              </>
            ) : null}
          </WayCard>
          <WayCard
            title={t("devices.ways.server")}
            now={server}
            lit={server || switching}
            data-device-way-row="server"
            foot={
              switching ? null : (
                <div className="w-full min-w-0" data-device-condition={deviceId}>
                  {server ? <ServerCondition state={state} /> : <SwitchPlace state={state} outcome={outcome} actions={actions} />}
                </div>
              )
            }
          >
            {sleeps ? <Headline lead={t("devices.ways.serverHead")} turn={t("devices.ways.serverHeadGo", { device: name })} tone="ok" /> : <Note>{t("devices.ways.serverLine")}</Note>}
            {switching ? (
              <SwitchSteps name={name} current={state.step ?? "checking"} />
            ) : (
              <>
                {sleeps ? (
                  <>
                    <Picture way="server" name={name} />
                    <Keeps way="server" name={name} />
                    <p className="break-keep text-caption text-muted-foreground">{t("devices.ways.serverFit")}</p>
                  </>
                ) : null}
                <p className="break-keep text-caption text-muted-foreground">{t("devices.ways.insteadLine")}</p>
              </>
            )}
          </WayCard>
        </div>
      </div>
    </div>
  );
}

/**
 * One way of the two as a card: its name with `지금` on the one in use, what
 * it is, and at its foot what to do about it. `lit` outlines the card the
 * device is on, or is moving to while a switch runs.
 */
function WayCard({ title, now, lit, foot, children, ...data }: { title: string; now: boolean; lit: boolean; foot: ReactNode; children: ReactNode } & Record<`data-${string}`, string>) {
  const { t } = useInterfaceTranslation();
  return (
    <section className={cn("flex min-w-0 flex-col gap-sm rounded-md border px-md py-sm", lit ? "border-muted-foreground bg-accent" : "border-border")} data-device-way-now={String(now)} {...data}>
      <div className="flex items-baseline gap-xs">
        <span className="text-body font-semibold text-foreground">{title}</span>
        {now ? <Badge>{t("devices.ways.now")}</Badge> : null}
      </div>
      {children}
      {foot ? <div className="mt-auto flex min-h-(--size-control) items-center border-t border-border pt-sm text-body">{foot}</div> : null}
    </section>
  );
}

/** The card's first words, the reason to choose it: what this computer's sleep does, and its outcome in the way's tone. */
function Headline({ lead, turn, tone }: { lead: string; turn: string; tone: "warn" | "ok" }) {
  return (
    <p className="break-keep text-subhead font-semibold text-foreground" data-device-way-headline={tone}>
      <span className="block">{lead}</span>
      <span className={cn("block", tone === "warn" ? "text-warning" : "text-success")}>{turn}</span>
    </p>
  );
}

/** The most a name takes in the picture, counting a wide (CJK) character as two. */
const PICTURE_NAME_UNITS = 16;

/** `name` cut to what fits under the drawn machine, ending in `…` when cut. */
function pictureName(name: string): string {
  let units = 0;
  let kept = "";
  for (const character of name) {
    units += (character.codePointAt(0) ?? 0) >= 0x2e80 ? 2 : 1;
    if (units > PICTURE_NAME_UNITS) return `${kept.trimEnd()}…`;
    kept += character;
  }
  return kept;
}

/** The labels' size in the drawing's units. */
const LABEL = 10;

/**
 * Where Hide runs in this way, and what is left of it while this computer
 * sleeps: an SVG in its own coordinates, so it scales down with the card and
 * stays at its drawn size in a wide one; the motion and colours are
 * `deviceWays.css`. For a reader it is one image
 * with one sentence of what it shows.
 */
function Picture({ way, name }: { way: "direct" | "server"; name: string }) {
  const { t } = useInterfaceTranslation();
  const server = way === "server";
  return (
    <svg viewBox="0 0 280 92" role="img" aria-label={t(server ? "devices.ways.pic.serverAria" : "devices.ways.pic.directAria")} className="mx-auto w-full max-w-(--size-device-way-picture)" data-device-way-picture={way}>
      <text className="dw-phase dw-awake" x="56" y="10" textAnchor="middle" fontSize={LABEL}>{t("devices.ways.pic.awake")}</text>
      <text className="dw-phase dw-asleep" x="56" y="10" textAnchor="middle" fontSize={LABEL}>{t("devices.ways.pic.asleep")}</text>
      <g className="dw-lid">
        <rect className="dw-frame" x="18" y="16" width="76" height="46" rx="4" strokeWidth="1.3" />
        <rect className="dw-hide" x="34" y="30" width="44" height="18" rx="9" />
        <text className="dw-hide-text" x="56" y="43" textAnchor="middle" fontSize={LABEL}>Hide</text>
      </g>
      <rect className="dw-base" x="10" y="62" width="92" height="5" rx="2" />
      <text className="dw-zz dw-asleep" x="98" y="54" fontSize={11}>z</text>
      <text className="dw-label" x="56" y="84" textAnchor="middle" fontSize={LABEL}>{t("devices.ways.pic.this")}</text>
      <line className="dw-wire" x1="108" y1="40" x2="176" y2="40" strokeWidth="1.2" />
      {server ? (
        <>
          <line className="dw-flow" x1="176" y1="37" x2="108" y2="37" strokeWidth="2.4" />
          <line className="dw-flow" x1="108" y1="43" x2="176" y2="43" strokeWidth="2.4" />
        </>
      ) : (
        <line className="dw-flow" x1="176" y1="40" x2="108" y2="40" strokeWidth="2.4" />
      )}
      <rect className="dw-frame" x="182" y="22" width="88" height="40" rx="9" strokeWidth="1.3" />
      {server ? (
        <>
          <rect className="dw-ring" x="204" y="32" width="44" height="18" rx="9" strokeWidth="1.2" />
          <rect className="dw-hide" x="204" y="32" width="44" height="18" rx="9" />
          <text className="dw-hide-text" x="226" y="45" textAnchor="middle" fontSize={LABEL}>Hide</text>
        </>
      ) : (
        <>
          <rect className="dw-tool" x="198" y="32" width="56" height="18" rx="9" strokeWidth="1" />
          <text className="dw-tool-text" x="226" y="44.5" textAnchor="middle" fontSize={9.5}>{t("devices.ways.pic.tool")}</text>
        </>
      )}
      <text className="dw-label" x="226" y="84" textAnchor="middle" fontSize={LABEL}>{pictureName(name)}</text>
    </svg>
  );
}

type Moment = { word: string; on: boolean };

/**
 * What keeps going in this way, row by row: each row's word while this
 * computer is awake and while it sleeps, sharing the picture's beat; the
 * last row says what the device itself offers and does not move. A reader
 * hears only the asleep word, the one the headline is about.
 */
function Keeps({ way, name }: { way: "direct" | "server"; name: string }) {
  const { t } = useInterfaceTranslation();
  const server = way === "server";
  const running: Moment = { word: t("devices.ways.state.running"), on: true };
  const rows: { key: string; label: string; asleep: Moment }[] = [
    { key: "agents", label: t("devices.ways.row.agents"), asleep: { word: t(server ? "devices.ways.state.keepsRunning" : "devices.ways.state.goesOn"), on: true } },
    { key: "missions", label: t("devices.ways.row.missions"), asleep: { word: t(server ? "devices.ways.state.keepsRunning" : "devices.ways.state.stops"), on: server } },
    { key: "automations", label: t("devices.ways.row.automations"), asleep: { word: t(server ? "devices.ways.state.keepsRunning" : "devices.ways.state.stops"), on: server } },
  ];
  return (
    <dl className="grid grid-cols-[fit-content(45%)_minmax(0,1fr)] gap-x-md gap-y-xxs text-caption" data-device-way-keeps={way}>
      {rows.map((row) => (
        <div key={row.key} className="contents" data-device-keep={row.key}>
          <dt className="truncate text-muted-foreground">{row.label}</dt>
          <dd className="grid min-w-0">
            <Word moment={running} className="dw-moment dw-awake" hidden />
            <Word moment={row.asleep} className="dw-moment dw-asleep" data-keep-asleep="true" />
          </dd>
        </div>
      ))}
      <div className="contents" data-device-keep="there">
        <dt className="truncate text-muted-foreground">{t("devices.ways.row.there", { device: name })}</dt>
        <dd className={cn("min-w-0 break-keep", server ? "text-foreground" : "text-muted-foreground")}>{t(server ? "devices.ways.state.app" : "devices.ways.state.noScreen")}</dd>
      </div>
    </dl>
  );
}

/** One row's word at one moment: running in the success tone with a dot, stopped in the warning tone with a pause mark. */
function Word({ moment, className, hidden = false, ...data }: { moment: Moment; className: string; hidden?: boolean } & Record<`data-${string}`, string>) {
  return (
    <span className={cn("flex min-w-0 items-baseline gap-xs", moment.on ? "text-success" : "text-warning", className)} aria-hidden={hidden || undefined} {...data}>
      <span aria-hidden="true" className="font-mono">{moment.on ? "●" : "‖"}</span>
      <span className="min-w-0 break-keep">{moment.word}</span>
    </span>
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
 * The condition of a device that can be switched, with the button, at the
 * server card's foot: how the last press ended, else what the machine said
 * and whether it can be pressed. A switch under way draws its steps in the
 * card instead.
 */
function SwitchPlace({ state, outcome, actions }: { state: WaysState; outcome: SwitchOutcome | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { deviceId, name, look } = state;
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
    return <Beside lines={<Note tone="warn">{line}</Note>} press={button("common.retry")} data-device-switch-outcome={`${deviceId}:${outcome.kind}`} />;
  }
  if (!state.connected) return <Note>{t("devices.ways.whenConnected")}</Note>;
  if (look === null || look.state === "reading") return <Status tone="pending">{t("devices.ways.reading")}</Status>;
  if (look.state === "unread") {
    // A login that asks for an answer is never switched; any other read that failed can be tried again.
    if (look.reason === "ssh_not_reached") return <Note tone="warn" data-device-condition-kind="password">{t("devices.ways.password")}</Note>;
    return <Beside lines={<Note tone="warn">{t("devices.ways.unread")}</Note>} press={recheck} data-device-condition-kind="unread" />;
  }
  const condition = look.condition;
  const pressable = switchButton(condition);
  // 방법 opens under the button, which stays in sight; closing it reads the
  // Mac again, so automatic login turned on meanwhile takes the warning away.
  return (
    <Popover onOpenChange={(open) => { if (!open) state.onRead(); }}>
      <Beside
        lines={<ConditionLine condition={condition} name={name} wsl={state.direct === "wsl"} actions={actions} deviceId={deviceId} />}
        press={
          pressable === "none" ? null : (
            <PopoverAnchor asChild>
              <div className="flex flex-wrap items-center gap-sm">
                {condition.kind === "linger_admin" ? recheck : null}
                {button("devices.ways.server", pressable === "off")}
              </div>
            </PopoverAnchor>
          )
        }
        data-device-condition-kind={condition.kind}
      />
      {condition.kind === "auto_login_off" && condition.how ? <AutoLoginSteps name={name} user={look.user} /> : null}
    </Popover>
  );
}

/**
 * The condition place at a card's foot: what was read on the left and what
 * to press on the right; words longer than one line take the whole width
 * and the button goes under them.
 */
function Beside({ lines, press, ...data }: { lines: ReactNode; press: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <div className="flex w-full flex-wrap items-center justify-between gap-x-md gap-y-xs" {...data}>
      <div className="flex min-w-0 flex-auto flex-col items-start gap-xs">{lines}</div>
      {press ? <div className="flex shrink-0 flex-wrap items-center gap-sm">{press}</div> : null}
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
