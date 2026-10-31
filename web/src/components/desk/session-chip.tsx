import { ChevronRightIcon, HouseIcon } from "lucide-react";
import { createContext, useCallback, useContext, useEffect, useState, useSyncExternalStore, type ReactNode } from "react";
import type { DeskJob, DeskSessionRef, SessionFacts } from "../../desk";
import { AgentMark } from "../../AgentMark";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { Elapsed } from "../elapsed";
import { PrMark } from "../pr-mark";
import { StatusMark } from "../status-mark";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger, useHintOpen } from "../ui/tooltip";
import { DeskPlanChip, JobLead, jobStateWords } from "./desk-job-card";

/**
 * Where the chips of a screen read their sessions: Desk's room answers from
 * the snapshot; a screen without one draws each chip from its own values,
 * with no card. `facts` answers null for a session it cannot tell, such as
 * one named only by a name no row has.
 */
export type SessionChipSource = {
  facts: (session: DeskSessionRef) => SessionFacts | null;
  onOpen: (session: DeskSessionRef) => void;
};

const SourceContext = createContext<SessionChipSource | null>(null);

export function SessionChipProvider({ source, children }: { source: SessionChipSource; children: ReactNode }) {
  return <SourceContext.Provider value={source}>{children}</SourceContext.Provider>;
}

/** The screen's source as it is now, held for chips drawn in a React root of their own. */
type Held = { current: SessionChipSource | null; listeners: Set<() => void> };

function BridgedSource({ held, children }: { held: Held; children: ReactNode }) {
  const subscribe = useCallback((listener: () => void) => {
    held.listeners.add(listener);
    return () => held.listeners.delete(listener);
  }, [held]);
  const source = useSyncExternalStore(subscribe, () => held.current);
  return <TooltipProvider><SourceContext.Provider value={source}>{children}</SourceContext.Provider></TooltipProvider>;
}

/**
 * Carries the screen's chip source into chips another component mounts in a
 * React root of its own (`MarkdownText` draws a reply's chips so), where no
 * context reaches. The wrapper it returns is stable, so the text around the
 * chips is not drawn again when a row moves; only the chips follow it.
 */
export function useSessionChipBridge(): (node: ReactNode) => ReactNode {
  const source = useContext(SourceContext);
  const [held] = useState<Held>(() => ({ current: source, listeners: new Set() }));
  useEffect(() => {
    if (held.current === source) return;
    held.current = source;
    for (const listener of held.listeners) listener();
  }, [held, source]);
  return useCallback((node: ReactNode) => <BridgedSource held={held}>{node}</BridgedSource>, [held]);
}

/**
 * A session named inside a line (a reply's `@session`, a letter's other
 * side, a task handed on) on the chip ground (B26, B50, D-75): its status,
 * its agent and its name, the same chip in the chat, a thread and its
 * history. A session the sidebar no longer lists is dimmed with no status
 * (D-86). Under the pointer or focus a listed session shows its card; a
 * press, on the chip or the card's `스레드 열기 ›`, opens its thread.
 * `card={false}` draws the chip alone, as inside another card.
 */
export function SessionChip({ session, onPress, card: carded = true, className }: { session: DeskSessionRef; onPress?: () => void; card?: boolean; className?: string }) {
  const { t } = useInterfaceTranslation();
  const source = useContext(SourceContext);
  const facts = source?.facts(session) ?? null;
  const gone = facts !== null && !facts.listed;
  const mark = facts?.listed ? facts.mark : gone ? null : session.mark;
  const agent = facts?.listed ? facts.agent_kind : gone ? null : session.agent_kind;
  const lead = agent === null ? null : <><StatusMark mark={mark} /><AgentMark kind={agent} /></>;
  const look = cn("inline-flex max-w-full min-w-0 items-center gap-xxs rounded-xs bg-secondary px-xxs align-middle text-foreground", gone && "opacity-(--opacity-dimmed)", className);
  const chip = !onPress ? (
    <span className={look} aria-label={gone ? t("desk.chip.gone", { name: session.name }) : undefined} data-desk-session-chip={session.name} data-desk-session-gone={gone ? "true" : undefined}>{lead}<span className="truncate">{session.name}</span></span>
  ) : (
    <Button variant="text" size="inline" className={cn(look, "hover:bg-accent hover:text-foreground")} aria-label={gone ? t("desk.chip.gone", { name: session.name }) : t("desk.mention.open", { name: session.name })} data-desk-session-chip={session.name} data-desk-session-gone={gone ? "true" : undefined} onClick={onPress}>
      {lead}
      <span className="truncate">{session.name}</span>
    </Button>
  );
  if (!carded || !facts?.listed || !facts.card) return chip;
  return <SessionHover facts={{ ...facts, card: facts.card }} onOpen={onPress} onParent={source?.onOpen}>{chip}</SessionHover>;
}

/** A chip's card under the pointer or focus; it can be hovered, so its `스레드 열기 ›` and parent chip can be pressed. */
function SessionHover({ facts, onOpen, onParent, children }: { facts: SessionFacts & { card: DeskJob }; onOpen?: () => void; onParent?: (session: DeskSessionRef) => void; children: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const { open, onOpenChange, triggerProps } = useHintOpen();
  return (
    <Tooltip open={open} onOpenChange={onOpenChange} disableHoverableContent={false}>
      <TooltipTrigger asChild {...triggerProps}>{children}</TooltipTrigger>
      <TooltipContent side="bottom" align="start" aria-label={t("desk.chip.card", { name: facts.card.title })} className="pointer-events-auto w-(--size-pr-popover) max-w-(--size-pr-popover) p-none text-left">
        <SessionCard job={facts.card} since={facts.since} parent={facts.parent} onOpen={onOpen} onParent={onParent} />
      </TooltipContent>
    </Tooltip>
  );
}

/**
 * A session's card (D-86, mockup 5): its status, agent, name, state and
 * time; ⌂ its project and branch; what it does now; its plan, pull request
 * and outputs; its parent; and `스레드 열기 ›`. Every fact is its sidebar
 * row's, through its card; a fact the session has none of is left out.
 */
export function SessionCard({ job, since, parent, onOpen, onParent }: { job: DeskJob; since: number | null; parent: DeskSessionRef | null; onOpen?: () => void; onParent?: (session: DeskSessionRef) => void }) {
  const { t } = useInterfaceTranslation();
  const pr = job.pull_requests[0] ?? null;
  const outputs = job.outputs.filter((output) => output.kind !== "pr").length;
  const place = job.location.label && job.location.label !== job.project.label ? `${job.project.label} · ${job.location.label}` : job.project.label;
  const facts = job.plan || pr || outputs > 0;
  return (
    <div className="flex min-w-0 flex-col gap-xs px-sm py-xs text-caption" data-desk-session-card={job.pane_id ?? job.id}>
      <div className="flex min-w-0 items-center gap-xxs">
        <JobLead job={job} />
        <span className="min-w-0 truncate font-semibold text-foreground">{job.title}</span>
        <span className="ml-auto flex shrink-0 items-center gap-xxs pl-xs text-micro text-muted-foreground" data-desk-session-card-state={job.state}>
          {jobStateWords(t, job)}
          {since !== null ? <><span aria-hidden="true">·</span><Elapsed since={since} /></> : null}
        </span>
      </div>
      {place ? (
        <span className="flex min-w-0 items-center gap-xxs text-subtle-foreground" data-desk-session-card-place="true">
          <HouseIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
          <span className="truncate">{place}</span>
        </span>
      ) : null}
      {job.now ? <span className="line-clamp-2 break-keep text-foreground [overflow-wrap:anywhere]" data-desk-session-card-now="true">{job.now}</span> : null}
      {facts ? (
        <span className="flex min-w-0 flex-wrap items-center gap-xxs">
          {job.plan ? <DeskPlanChip plan={job.plan} review={job.ask === "plan_review"} /> : null}
          {pr ? (
            <Badge variant="secondary" className="gap-xxs" data-desk-session-card-pr={pr.number}>
              <PrMark state={pr.state} compact />#{pr.number}{job.pull_requests.length > 1 ? ` +${job.pull_requests.length - 1}` : ""}
            </Badge>
          ) : null}
          {outputs > 0 ? <Badge variant="secondary" data-desk-session-card-outputs={outputs}>{t("desk.chip.outputs", { count: outputs })}</Badge> : null}
        </span>
      ) : null}
      <div className="flex min-w-0 items-center gap-xs border-t border-border pt-xs text-muted-foreground">
        {parent ? (
          <span className="flex min-w-0 items-center gap-xxs" data-desk-session-card-parent={parent.name}>
            <span className="shrink-0">{t("desk.chip.parent")} ·</span>
            <SessionChip session={parent} card={false} onPress={onParent ? () => onParent(parent) : undefined} className="text-micro" />
          </span>
        ) : null}
        {onOpen ? (
          <Button variant="text" size="inline" className="ml-auto shrink-0 gap-xxs text-foreground" data-desk-session-card-open="true" onClick={onOpen}>
            {t("desk.chip.openThread")}
            <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />
          </Button>
        ) : null}
      </div>
    </div>
  );
}
