import { useInterfaceTranslation } from "../i18n/client";
import type { AgentRow } from "../snapshot";
import { recordPending, recordWords } from "../agentRow";
import { cn } from "../lib/utils";

/**
 * The same short history provenance on every shared agent row. A title whose
 * connection is not confirmed is only dimmed, since every restored row checks
 * at launch and the worker retries on its own; the hint and accessible names
 * carry the connection words.
 */
export function AgentRecordTitle({ agent, className }: { agent: AgentRow; className?: string }) {
  const { t } = useInterfaceTranslation();
  const pending = recordPending(agent);
  return <span className="flex min-w-0 flex-1 items-center" title={[recordWords(t, agent), agent.record_summary].filter(Boolean).join("\n") || undefined}>
    <span className={cn("min-w-0 flex-1 truncate", className, pending && "opacity-(--opacity-dimmed)")} data-agent-title="true" data-record-connection={pending ? agent.record_connection : undefined}>{agent.identity_label}</span>
  </span>;
}
