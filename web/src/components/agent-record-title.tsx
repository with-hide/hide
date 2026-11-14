import { useInterfaceTranslation } from "../i18n/client";
import type { AgentRow } from "../snapshot";
import { recordPending, recordWords } from "../agentRow";
import { cn } from "../lib/utils";
import { SubagentDots } from "./subagent-dots";

/**
 * The same short history provenance on every shared agent row. A title whose
 * connection is not confirmed is only dimmed, since every restored row checks
 * at launch and the worker retries on its own; the hint and accessible names
 * carry the connection words.
 *
 * A list that draws the agent as a row passes `subagents`, and the subagents
 * its session runs stand right after the title as dots (`SubagentDots`);
 * `inRow` when the row's own name already says them.
 */
export function AgentRecordTitle({ agent, className, subagents }: { agent: AgentRow; className?: string; subagents?: "inRow" | "hint" }) {
  const { t } = useInterfaceTranslation();
  const pending = recordPending(agent);
  return <span className="flex min-w-0 flex-1 items-center gap-xs" title={[recordWords(t, agent), agent.record_summary].filter(Boolean).join("\n") || undefined}>
    <span className={cn("min-w-0 truncate", !subagents && "flex-1", className, pending && "opacity-(--opacity-dimmed)")} data-agent-title="true" data-record-connection={pending ? agent.record_connection : undefined}>{agent.identity_label}</span>
    {subagents ? <SubagentDots count={agent.subagents} inRow={subagents === "inRow"} /> : null}
  </span>;
}
