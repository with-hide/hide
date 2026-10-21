import { useInterfaceTranslation } from "../i18n/client";
import type { AgentRow } from "../snapshot";
import { recordPending, recordWords } from "../agentRow";
import { cn } from "../lib/utils";

/**
 * The same short history provenance on every shared agent row. A check in
 * progress only dims the title, since every restored row checks at launch;
 * the words show once a check has not confirmed, and the hint and accessible
 * names carry both states.
 */
export function AgentRecordTitle({ agent, className, stacked = false }: { agent: AgentRow; className?: string; stacked?: boolean }) {
  const { t } = useInterfaceTranslation();
  const words = recordWords(t, agent);
  const shown = agent.record_connection === "unconfirmed" ? words : null;
  return <span className={cn("flex min-w-0 flex-1", stacked && shown ? "flex-col items-start justify-center" : "items-center gap-xs")} title={[words, agent.record_summary].filter(Boolean).join("\n") || undefined}>
    <span className={cn("min-w-0 truncate", stacked && shown ? "w-full text-micro leading-(--text-micro)" : "flex-1", className, recordPending(agent) && "opacity-(--opacity-dimmed)")} data-agent-title="true">{agent.identity_label}</span>
    {shown ? <span className={cn("min-w-0 max-w-full shrink-0 truncate whitespace-nowrap text-micro font-normal text-muted-foreground", stacked && "w-full leading-(--text-micro)")} data-record-connection={agent.record_connection}>{shown}</span> : null}
  </span>;
}
