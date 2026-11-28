import { ChevronRightIcon, HouseIcon } from "lucide-react";
import { createContext, useCallback, useContext, useEffect, useState, useSyncExternalStore, type ReactNode } from "react";
import type { DeskJob, DeskSessionRef, SessionFacts } from "../../desk";
import { AgentMark } from "../../AgentMark";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { Elapsed } from "../elapsed";
import { PrMark } from "../pr-mark";
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
  /** The pane of the session open beside Desk, whose chips are ringed as the open one (frame J); absent on a screen with no room. */
  open?: string | null;
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
 * side, a task handed on, a `@` the operator picked) as words, not a box
 * (mockup v3 section 7; B26, B49, B50, D-75, D-85): its agent's mark and its
 * name (its sidebar row's while the sidebar lists it) in the session blue,
 * the same in the composer, the chat, a thread and its history. Its state is
 * never drawn in the line: it shows only in the card. A session the sidebar
 * no longer lists whose record Hide keeps reads as that record, its title and
 * agent, one step quieter, and still opens (feedback 2, frame K); one with no
 * record is dimmed and opens nothing (D-86). Under the pointer the name is
 * underlined and nothing in the line moves; dwelling, or focus, shows a
 * listed or recorded session's card; a press, on the name or the card's
 * `스레드 열기 ›`, opens its thread. The name of the session open beside Desk
 * stays underlined (frame J). `look="band"` takes a letter band's own color
 * and leaves the agent's mark out, as the band draws its other side;
 * `card={false}` draws the name alone, as inside another card.
 */
export function SessionChip({ session, onPress, card: carded = true, look: drawn = "line", className }: { session: DeskSessionRef; onPress?: () => void; card?: boolean; look?: "line" | "band"; className?: string }) {
  const { t } = useInterfaceTranslation();
  const source = useContext(SourceContext);
  const facts = source?.facts(session) ?? null;
  const gone = facts !== null && !facts.listed;
  // A session the sidebar no longer lists, read from Hide's record of it.
  const ended = gone && facts.record !== null;
  const dimmed = gone && !ended;
  const known = facts !== null && (facts.listed || ended);
  const agent = known ? facts.agent_kind : gone ? null : session.agent_kind;
  // A listed session is called what its sidebar row calls it, an ended one what its record does, whatever name the words that named it wrote.
  const name = (known ? facts.name : null) ?? session.name;
  const lead = agent === null || drawn === "band" ? null : <AgentMark kind={agent} />;
  // The name of the session open beside Desk stays underlined, as a hovered one is (B16, frame J).
  const shown = !dimmed && source?.open != null && source.open === (facts ? facts.pane_id : session.pane_id);
  const look = cn("inline-flex max-w-full min-w-0 items-center gap-xxs align-middle decoration-1 underline-offset-2", drawn === "band" ? "text-inherit" : "text-mention-session", dimmed && "opacity-(--opacity-dimmed)", ended && "opacity-(--opacity-secondary)", shown && "underline", className);
  const marks = { "data-desk-session-chip": name, "data-desk-session-gone": dimmed ? "true" : undefined, "data-desk-session-ended": ended ? "true" : undefined, "data-desk-session-open": shown ? "true" : undefined };
  // A session no longer listed and not recorded has nothing left to open.
  const press = dimmed ? undefined : onPress;
  const chip = !press ? (
    <span className={look} aria-label={dimmed ? t("desk.chip.gone", { name }) : undefined} {...marks}>{lead}<span className="truncate">{name}</span></span>
  ) : (
    <Button variant="text" size="inline" className={cn(look, drawn === "band" ? "hover:text-inherit" : "hover:text-mention-session", "hover:underline")} aria-label={t("desk.mention.open", { name })} aria-current={shown ? "true" : undefined} {...marks} onClick={press}>
      {lead}
      <span className="truncate">{name}</span>
    </Button>
  );
  if (!carded || !known || !facts.card) return chip;
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
