// The pieces a phone row and its detail share (PRD mobile-companion B20,
// B24): the status mark the desktop row draws, the provider mark, the
// task name, and the project · branch with the SSH device's chip.

import { ServerIcon } from "lucide-react";
import { AgentMark } from "../AgentMark";
import { StatusMark } from "../components/status-mark";
import type { PhoneAgent } from "./protocol";

/** The column a list row's detail lines start in: under the title, after the status mark alone. */
export const LIST_INSET = "pl-[calc(var(--size-agent-mark)+var(--spacing-sm))]";
/** The detail head also draws the logo, so its lines start under the title after both marks. */
export const DETAIL_INSET = "pl-(--size-mobile-row-inset)";

/** The row's head: the detail screen (`large`) names the agent with its logo, a list row does not (PRD agent-mark-quiet B12). */
export function AgentHead({ agent, large }: { agent: PhoneAgent; large: boolean }) {
  return (
    <span className="flex min-w-0 items-center gap-sm">
      {/* The desktop row's mark; a read turn is dimmed as it is there. */}
      {agent.read ? <StatusMark mark="turn" read data-phone-mark="turn" /> : <StatusMark mark={agent.mark} className={(agent.mark === "turn" || agent.mark === "look") && !agent.emphasized ? "opacity-(--opacity-read-status)" : undefined} data-phone-mark={agent.mark} />}
      {large ? <AgentMark kind={agent.agent_kind} /> : null}
      <span
        className={`min-w-0 truncate ${large ? "text-headline font-semibold" : "text-title font-medium"} ${
          agent.emphasized || large ? "text-foreground" : "text-subtle-foreground"
        }`}
      >
        {agent.title}
      </span>
    </span>
  );
}

export function Place({ agent, inset }: { agent: PhoneAgent; inset: string }) {
  if (!agent.place && !agent.device_label) return null;
  return (
    <span className={`flex min-w-0 items-center gap-sm ${inset} text-body text-muted-foreground`}>
      {agent.place ? <span className="min-w-0 truncate">{agent.place}</span> : null}
      {agent.device_label ? (
        <span className="inline-flex shrink-0 items-center gap-xs rounded-md bg-secondary px-sm text-body text-subtle-foreground" data-phone-device={agent.device_label}>
          <ServerIcon aria-hidden="true" className="size-(--size-icon-sm)" />
          {agent.device_label}
        </span>
      ) : null}
    </span>
  );
}
