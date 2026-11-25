// The pieces a phone row and its detail share (PRD mobile-companion B20,
// B24): the status mark the desktop row draws, the provider mark, the
// task name, and the project · branch with the SSH device's chip.

import { LaptopIcon, ServerIcon } from "lucide-react";
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
import { cn } from "../lib/utils";

/** The phone's own clipboard, which Copy of a person's turn's place writes; a refusal is the log's. */
const clipboard = browserClipboard((message) => console.warn("phone.copy_failed", { message }));


/** The desktop row's mark for an agent; a read turn is dimmed as it is there, and a starred row waiting in 다시 볼 것 wears its ★ in the mark's place (PRD revisit-group D-03). */
export function AgentStatusMark({ agent }: { agent: PhoneAgent }) {
  if (agent.starred && agent.group === "revisit") return <RevisitStarShape slot data-phone-mark="star" />;
  if (agent.read) return <StatusMark mark="turn" read human={agent.human_turn != null} data-phone-mark="turn" />;
  return <StatusMark mark={agent.mark} human={agent.human_turn != null} className={(agent.mark === "turn" || agent.mark === "look") && !agent.emphasized ? "opacity-(--opacity-read-status)" : undefined} data-phone-mark={agent.mark} />;
}

/**
 * The phone's one chip, in the desktop chip's shape (a small-cornered fill): the children chip, a mission's and a pull request's
 * on the list and the detail all wear it, each with the face the desktop draws inside (`TreeChipFace`, `MissionChipFace`, the PR
 * mark). Its border shows only when a child asks the operator (`raised`); the caller makes it a press.
 */
export function PhoneChip({ raised = false, className, children, ...data }: { raised?: boolean; className?: string; children: ReactNode } & Record<`data-${string}`, string | undefined>) {
  return (
    <span className={cn("flex min-w-0 shrink-0 items-center gap-xxs rounded-xs border bg-secondary px-xs py-xxs text-body font-normal", raised ? "border-warning/60" : "border-transparent", className)} {...data}>
      {children}
    </span>
  );
}

/** What leads the detail's title: the mark and the agent's logo, which a list row leaves out (PRD agent-mark-quiet B12). */
export function AgentLead({ agent }: { agent: PhoneAgent }) {
  return (
    <>
      <AgentStatusMark agent={agent} />
      <AgentMark kind={agent.agent_kind} />
    </>
  );
}

/** A list row's title, cut to one line, with the ★ small after it anywhere but 다시 볼 것. */
export function AgentTitle({ agent }: { agent: PhoneAgent }) {
  return (
    <>
      <span className={`min-w-0 truncate text-title font-medium ${agent.emphasized ? "text-foreground" : "text-subtle-foreground"}`}>{agent.title}</span>
      {agent.starred && agent.group !== "revisit" ? <RevisitStarShape /> : null}
    </>
  );
}

/**
 * The SSH device's chip: where an agent or a mission runs when that is not this Mac. `computer` is the gathered view's mark of the
 * paired computer whose work the row is (`gather.ts`), drawn outlined so it reads apart from a device the work runs on.
 */
export function DeviceTag({ label, computer = false }: { label: string; computer?: boolean }) {
  const Icon = computer ? LaptopIcon : ServerIcon;
  return (
    <span
      className={`inline-flex shrink-0 items-center gap-xs rounded-md px-sm text-body text-subtle-foreground ${computer ? "border border-border" : "bg-secondary"}`}
      {...(computer ? { "data-phone-computer": label } : { "data-phone-device": label })}
    >
      <Icon aria-hidden="true" className="size-(--size-icon-sm)" />
      {label}
    </span>
  );
}

/**
 * Where an agent works, as pieces of a line its caller lays out: the gathered view's computer (`computer`), the tool whose work
 * the row is, leading as on the desktop's sidebar row (PRD hide-external-control B46), the place, and the SSH device's chip. A
 * list row names the project alone (`project`), from a Mac that predates it the whole place; the detail names project and branch.
 */
export function PlaceParts({ agent, project = false, computer }: { agent: PhoneAgent; project?: boolean; computer?: string }) {
  const place = project ? (agent.project ?? agent.place) : agent.place;
  return (
    <>
      {computer ? <DeviceTag label={computer} computer /> : null}
      {/* The tool whose work this is leads the line, as on the desktop's sidebar row (PRD hide-external-control B46). */}
      {agent.tool ? (
        <span className="flex min-w-0 max-w-2/5 shrink-0 items-baseline gap-xs text-muted-foreground" data-phone-tool={agent.tool}>
          <span className="min-w-0 truncate">{agent.tool}</span>
          {place ? <span aria-hidden="true">·</span> : null}
        </span>
      ) : null}
      {place ? <span className="min-w-0 max-w-1/2 shrink-0 truncate text-muted-foreground" data-phone-place={place}>{place}</span> : null}
      {agent.device_label ? <DeviceTag label={agent.device_label} /> : null}
    </>
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
        {/* An install an agent asked for is confirmed on the computer, where its files show (PRD skill-mcp-manager B32). */}
        {turn.kind === "install" ? <span className="min-w-0 break-words text-subhead text-muted-foreground" data-phone-human-install={turn.id}>{t("mobile.humanTurn.installOnComputer")}</span> : <Button
          variant="secondary"
          className="min-h-(--size-touch-target) shrink-0 text-body"
          disabled={away || !!pending?.requestId}
          aria-label={t("agents.human.doneName", { what })}
          data-phone-human-done={turn.id}
          onClick={() => finishHumanTurn(agent)}
        >
          {t("agents.human.done")}
        </Button>}
        {turn.kind === "install" ? null : away ? <span className="min-w-0 truncate text-subhead text-muted-foreground">{t("devices.rail.notConnected")}</span> : pending?.failed ? <span role="status" className="min-w-0 break-words text-subhead text-warning">{t("mobile.humanTurn.failed")}</span> : null}
      </span>
    </div>
  );
}
