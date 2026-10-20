import { type ReactNode } from "react";
import { AgentMark } from "./AgentMark";
import { agentProductName } from "./agentAdapters";
import { markTone } from "./agentRow";
import { AgentRecordTitle } from "./components/agent-record-title";
import { Elapsed } from "./components/elapsed";
import { StatusMark } from "./components/status-mark";
import { Tooltip, TooltipContent, TooltipTrigger, useHintOpen } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import type { AgentRow } from "./snapshot";

/** Where an agent stands, for the graph row's popover (agents-graph-view B19): its checkout and tab, its tab's other agents, and who delegated it. */
export type PopoverContext = { checkout: string; tab: string | null; peers: readonly string[]; parent: string | null; line: string | null };

/**
 * Everything an agent last said, after a half-second rest on `children`
 * (D-50): on its line, or on its mark in a PRs tool row (overview-lenses-prs
 * B7). `fallback` is what is said when the agent has no message.
 */
export function AgentMessagePopover({ agent, place, fallback, tone, onOpen, context, children }: { agent: AgentRow; place: string; fallback: string; tone: string; onOpen: () => void; context?: PopoverContext; children: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const { open, onOpenChange, triggerProps } = useHintOpen();
  const message = agent.message?.trim() || fallback;
  return (
    <Tooltip open={open} onOpenChange={onOpenChange} disableHoverableContent={false}>
      <TooltipTrigger asChild {...triggerProps}>
        {children}
      </TooltipTrigger>
      <TooltipContent side="bottom" align="start" className="pointer-events-auto w-(--size-pr-popover) max-w-(--radix-tooltip-content-available-width) text-left text-wrap rounded-md p-md" data-lens-message={agent.pane_id}>
        <div className="flex flex-col gap-sm">
          <span className="flex min-w-0 items-center gap-xs text-body font-medium text-foreground">
            <StatusMark mark={agent.mark} className={markTone(agent)} />
            <AgentMark kind={agent.agent_kind} />
            <AgentRecordTitle agent={agent} />
            <span className="shrink-0 text-caption font-normal text-muted-foreground">{agentProductName(agent.agent_kind)}</span>
            <Elapsed since={agent.changed_at_unix_ms} className="font-mono text-caption text-muted-foreground" />
          </span>
          <p className={cn("whitespace-pre-wrap break-words text-caption", tone)}>{message}</p>
          {context ? (
            <dl className="flex flex-col gap-xxs text-caption text-muted-foreground" data-lens-message-context={agent.pane_id}>
              <div className="flex gap-sm">
                <dt className="shrink-0">{t("overview.location")}</dt>
                <dd className="min-w-0 break-words text-foreground">{[context.checkout, context.tab].filter(Boolean).join(" · ")}</dd>
              </div>
              {context.peers.length > 0 ? (
                <div className="flex gap-sm">
                  <dt className="shrink-0">{t("overview.context.sameTab")}</dt>
                  <dd className="min-w-0 break-words text-foreground">{context.peers.join(" · ")}</dd>
                </div>
              ) : null}
              <div className="flex gap-sm">
                <dt className="shrink-0">{t("overview.context.delegator")}</dt>
                <dd className="min-w-0 break-words text-foreground">{context.parent ?? t("overview.context.manualStart")}</dd>
              </div>
              {context.line ? (
                <div className="flex gap-sm">
                  <dt className="shrink-0">{t("overview.context.line")}</dt>
                  <dd className="min-w-0 break-words text-foreground">{context.line}</dd>
                </div>
              ) : null}
            </dl>
          ) : null}
          <span className="flex items-center gap-sm text-caption text-muted-foreground">
            <span className="min-w-0 flex-1 truncate font-mono">{place}</span>
            <button
              type="button"
              data-lens-message-open={agent.pane_id}
              className="shrink-0 rounded-xs font-medium text-foreground outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring"
              onClick={(event) => {
                event.stopPropagation();
                onOpen();
              }}
            >
              {t("overview.replyInPanel")}
            </button>
          </span>
        </div>
      </TooltipContent>
    </Tooltip>
  );
}
