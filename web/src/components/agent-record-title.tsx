import { useInterfaceTranslation } from "../i18n/client";
import type { AgentRow } from "../snapshot";
import { recordPending, recordWords } from "../agentRow";
import { cn } from "../lib/utils";

/** The same short history provenance on every shared agent row. */
export function AgentRecordTitle({ agent, className, stacked = false }: { agent: AgentRow; className?: string; stacked?: boolean }) {
  const { t } = useInterfaceTranslation();
  const words = recordWords(t, agent);
  return <span className={cn("flex min-w-0 flex-1", stacked && words ? "flex-col items-start justify-center" : "items-center gap-xs")} title={[words, agent.record_summary].filter(Boolean).join("\n") || undefined}>
    <span className={cn("min-w-0 truncate", stacked && words ? "w-full leading-none" : "flex-1", className, recordPending(agent) && "opacity-(--opacity-dimmed)")} data-agent-title="true">{agent.identity_label}</span>
    {words ? <span className={cn("min-w-0 max-w-full shrink-0 truncate whitespace-nowrap text-micro font-normal text-muted-foreground", stacked && "w-full leading-none")} data-record-connection={agent.record_connection}>{words}</span> : null}
  </span>;
}
