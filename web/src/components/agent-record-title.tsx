import { useInterfaceTranslation } from "../i18n/client";
import type { AgentRow } from "../snapshot";
import { recordPending, recordWords } from "../agentRow";
import { cn } from "../lib/utils";

/** The same short history provenance on every shared agent row. */
export function AgentRecordTitle({ agent, className }: { agent: AgentRow; className?: string }) {
  const { t } = useInterfaceTranslation();
  const words = recordWords(t, agent);
  return <span className="flex min-w-0 flex-1 items-center gap-xs" title={[words, agent.record_summary].filter(Boolean).join("\n") || undefined}>
    <span className={cn("min-w-0 flex-1 truncate", className, recordPending(agent) && "opacity-(--opacity-dimmed)")} data-agent-title="true">{agent.identity_label}</span>
    {words ? <span className="min-w-0 max-w-full shrink-0 truncate whitespace-nowrap text-micro font-normal text-muted-foreground" data-record-connection={agent.record_connection}>{words}</span> : null}
  </span>;
}
