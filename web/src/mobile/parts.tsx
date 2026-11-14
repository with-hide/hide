// The pieces a phone row and its detail share (PRD mobile-companion B20,
// B24): the status mark the desktop row draws, the provider mark, the
// task name, and the project · branch with the SSH device's chip.

import { ServerIcon } from "lucide-react";
import type { ReactNode } from "react";
import { AgentMark } from "../AgentMark";
import { StatusMark } from "../components/status-mark";
import { RevisitStarShape } from "../components/revisit-mark";
import { DeviceChip } from "../components/device-chip";
import { HumanTurnCopy } from "../components/human-turn-copy";
import { browserClipboard } from "../clipboard";
import { Button } from "../components/ui/button";
import { humanTurnText } from "../humanTurn";
import { useInterfaceTranslation } from "../i18n/client";
import { finishHumanTurn } from "./connection";
import type { PhoneAgent } from "./protocol";
import { usePhone } from "./store";

/** The phone's own clipboard, which Copy of a person's turn's place writes; a refusal is the log's. */
const clipboard = browserClipboard((message) => console.warn("phone.copy_failed", { message }));

/** The column a list row's detail lines start in: under the title, after the status mark alone. */
export const LIST_INSET = "pl-[calc(var(--size-agent-mark)+var(--spacing-sm))]";
/** The detail head also draws the logo, so its lines start under the title after both marks. */
export const DETAIL_INSET = "pl-(--size-mobile-row-inset)";

/** The row's head: the detail screen (`large`) names the agent with its logo, a list row does not (PRD agent-mark-quiet B12). */
export function AgentHead({ agent, large }: { agent: PhoneAgent; large: boolean }) {
  return (
    <span className={`flex min-w-0 gap-sm ${large ? "items-start" : "items-center"}`}>
      {/* The desktop row's mark; a read turn is dimmed as it is there. */}
      {/* A starred row waiting in 다시 볼 것 wears its ★ in the mark's place, as the desktop's does (PRD revisit-group D-03). */}
      {agent.starred && agent.group === "revisit" ? <RevisitStarShape slot data-phone-mark="star" /> : agent.read ? <StatusMark mark="turn" read human={agent.human_turn != null} data-phone-mark="turn" /> : <StatusMark mark={agent.mark} human={agent.human_turn != null} className={(agent.mark === "turn" || agent.mark === "look") && !agent.emphasized ? "opacity-(--opacity-read-status)" : undefined} data-phone-mark={agent.mark} />}
      {large ? <AgentMark kind={agent.agent_kind} /> : null}
      <span
        className={`min-w-0 ${large ? "whitespace-normal break-keep text-title font-medium wrap-anywhere" : "truncate text-title font-medium"} ${
          agent.emphasized || large ? "text-foreground" : "text-subtle-foreground"
        }`}
      >
        {agent.title}
      </span>
      {/* Anywhere else the ★ is small after the title. */}
      {agent.starred && agent.group !== "revisit" ? <RevisitStarShape className={large ? "mt-sm" : undefined} /> : null}
    </span>
  );
}

/** The SSH device's chip: where an agent or a mission runs when that is not this Mac. */
export function DeviceTag({ label }: { label: string }) {
  return (
    <span className="inline-flex shrink-0 items-center gap-xs rounded-md bg-secondary px-sm text-body text-subtle-foreground" data-phone-device={label}>
      <ServerIcon aria-hidden="true" className="size-(--size-icon-sm)" />
      {label}
    </span>
  );
}

export function Place({ agent, inset }: { agent: PhoneAgent; inset: string }) {
  if (!agent.place && !agent.device_label && !agent.tool) return null;
  return (
    <span className={`flex min-w-0 items-center gap-sm ${inset} text-body text-muted-foreground`}>
      {/* The tool whose work this is leads the line, as on the desktop's sidebar row (PRD hide-external-control B46). */}
      {agent.tool ? <span className="flex min-w-0 max-w-2/5 shrink-0 items-baseline gap-xs" data-phone-tool={agent.tool}><span className="min-w-0 truncate">{agent.tool}</span>{agent.place ? <span aria-hidden="true">·</span> : null}</span> : null}
      {agent.place ? <span className="min-w-0 truncate">{agent.place}</span> : null}
      {agent.device_label ? <DeviceTag label={agent.device_label} /> : null}
    </span>
  );
}

/** A phone sheet's select: one touch target tall, the same in every sheet. */
export const SELECT_CLASS =
  "h-(--size-touch-target) w-full min-w-0 truncate rounded-lg border border-border bg-input px-md text-title text-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50";

/** A phone sheet's labelled field. */
export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="flex min-w-0 flex-col gap-xs text-body text-muted-foreground">
      <span>{label}</span>
      {children}
    </label>
  );
}

/**
 * A person's turn on the phone (PRD ask-human B9, B10, B19, B21): its kind
 * and place as text the operator can select, Copy beside the place, which
 * puts it on this phone's clipboard, the device it waits on, and
 * Done, which waits while the Mac or that device is away and says when a
 * press did not land.
 */
export function PhoneHumanTurn({ agent, inset }: { agent: PhoneAgent; inset: string }) {
  const { t } = useInterfaceTranslation();
  const turn = agent.human_turn;
  const connected = usePhone((state) => state.connected);
  const deviceAway = usePhone((state) => state.devices.some((device) => device.device_id === agent.device_id && !device.connected));
  const pending = usePhone((state) => (turn && state.humanTurn?.declaration === turn.id ? state.humanTurn : null));
  if (!turn) return null;
  const what = humanTurnText(t, turn);
  const away = !connected || deviceAway;
  return (
    <div className={`flex min-w-0 flex-col gap-xs ${inset}`} data-phone-human-turn={turn.id}>
      <span className="flex min-w-0 items-baseline gap-sm text-body">
        {/* Copy follows the place's last word, however the place wraps. */}
        <span className="min-w-0 break-words text-warning select-text" data-phone-human-place={turn.id}>{what}<HumanTurnCopy turn={turn} copy={clipboard.write} touch className="-my-md ml-xxs align-middle" /></span>
        {agent.device_label ? <DeviceChip label={agent.device_label} variant="inline" className="shrink-0" /> : null}
      </span>
      <span className="flex min-w-0 items-center gap-sm">
        <Button
          variant="secondary"
          className="min-h-(--size-touch-target) shrink-0 text-body"
          disabled={away || !!pending?.requestId}
          aria-label={t("agents.human.doneName", { what })}
          data-phone-human-done={turn.id}
          onClick={() => finishHumanTurn(agent)}
        >
          {t("agents.human.done")}
        </Button>
        {away ? <span className="min-w-0 truncate text-subhead text-muted-foreground">{t("devices.rail.notConnected")}</span> : pending?.failed ? <span role="status" className="min-w-0 break-words text-subhead text-warning">{t("mobile.humanTurn.failed")}</span> : null}
      </span>
    </div>
  );
}
