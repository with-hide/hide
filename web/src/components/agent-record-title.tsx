import { useInterfaceTranslation } from "../i18n/client";
import type { AgentRow } from "../snapshot";
import { recordPending, recordWords } from "../agentRow";
import { cn } from "../lib/utils";
import { SubagentMark } from "./subagent-mark";

/**
 * The same short history provenance on every shared agent row. A title whose
 * connection is not confirmed is only dimmed, since every restored row checks
 * at launch and the worker retries on its own; the hint and accessible names
 * carry the connection words.
 *
 * A list that draws the agent as a row passes `subagents`, and the subagents
 * its session runs stand right after the title (`SubagentMark`); `inRow`
 * when the row's own name already says them. The title keeps a few letters
 * first: a slot narrower than that drops the mark (`subagent-mark-room`).
 */
export function AgentRecordTitle({ agent, className, subagents }: { agent: AgentRow; className?: string; subagents?: "inRow" | "hint" }) {
  const { t } = useInterfaceTranslation();
  const pending = recordPending(agent);
  return <span className={cn("flex min-w-0 flex-1 items-center gap-xs", subagents && "@container/agent-title")} title={[recordWords(t, agent), agent.record_summary].filter(Boolean).join("\n") || undefined}>
    <span className={cn("min-w-0 truncate", !subagents && "flex-1", className, pending && "opacity-(--opacity-dimmed)")} data-agent-title="true" data-record-connection={pending ? agent.record_connection : undefined}>{agent.identity_label}</span>
    {subagents ? <SubagentMark count={agent.subagents} inRow={subagents === "inRow"} className="hidden subagent-mark-room:inline-flex" /> : null}
  </span>;
}
