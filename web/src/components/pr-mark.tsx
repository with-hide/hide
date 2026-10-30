import type { HTMLAttributes, ReactNode } from "react";
import { recordPending, recordWords } from "../agentRow";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { cn } from "../lib/utils";
import { CLOSED_DIM, PR_LOOK, prMarkParts, pullsStaleness, reviewWord, staleLabel, type PrMarkParts, type PrStaleness } from "../prMark";
import type { PrChip } from "../projectBoard";
import { pullRequestCard } from "../projects";
import { catalogWorkspaces, type AgentRow, type PrState, type SessionPull } from "../snapshot";
import { useShellStore } from "../store";
import { AgentRecordTitle } from "./agent-record-title";
import { CheckoutCardHint } from "./pr-card";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuTrigger } from "./ui/dropdown-menu";
import { Hint, Tooltip, TooltipContent, TooltipTrigger, useHintOpen } from "./ui/tooltip";

/**
 * The one PR mark (docs/UI_BEHAVIOR.md, PR mark): the state's icon in its
 * colour and `#number`. No border; the icon's colour is the state. `compact`
 * is the same mark without the number, where a row already names the pull
 * request (a checkout's kind glyph, a search row's icon). A stale mark dims.
 */
export function PrMark({
  state,
  number,
  stale = false,
  compact = false,
  className,
  ...rest
}: { state: PrState; number?: number; stale?: boolean; compact?: boolean } & HTMLAttributes<HTMLSpanElement>) {
  const { icon: Icon, tone } = PR_LOOK[state];
  return (
    <span
      data-pr-mark={state}
      data-stale={stale ? "true" : undefined}
      className={cn("inline-flex shrink-0 items-center gap-xxs font-mono text-caption", stale && "opacity-(--opacity-dimmed)", className)}
      {...rest}
    >
      <Icon aria-hidden="true" className={cn("size-(--size-pr-icon) shrink-0", tone)} />
      {compact || number === undefined ? null : <span className="text-subtle-foreground">#{number}</span>}
    </span>
  );
}

/**
 * The mark of several PRs: each state's icon and how many PRs stand in it, in
 * that state's colour, worst first, so a mergeable PR beside a merged one
 * reads green 1 purple 1 rather than one colour for both. A stale mark dims.
 */
function PrStateCounts({ counts, stale = false, className, ...rest }: { counts: { state: PrState; count: number }[]; stale?: boolean } & HTMLAttributes<HTMLSpanElement>) {
  return (
    <span
      data-pr-counts={counts.map(({ state, count }) => `${state}:${count}`).join(" ")}
      data-stale={stale ? "true" : undefined}
      className={cn("inline-flex shrink-0 items-center gap-xs font-mono text-caption", stale && "opacity-(--opacity-dimmed)", className)}
      {...rest}
    >
      {counts.map(({ state, count }) => {
        const { icon: Icon, tone } = PR_LOOK[state];
        return (
          <span key={state} className={cn("inline-flex items-center gap-xxs", tone, state === "closed" && CLOSED_DIM)} data-pr-count={state}>
            <Icon aria-hidden="true" className="size-(--size-pr-icon) shrink-0" />
            {count}
          </span>
        );
      })}
    </span>
  );
}

/** A row's or a family's mark, one PR or several; a closed PR is dimmed. */
export function PartsMark({ parts, stale, className, ...rest }: { parts: PrMarkParts; stale?: boolean } & HTMLAttributes<HTMLSpanElement>) {
  return "counts" in parts ? <PrStateCounts counts={parts.counts} stale={stale} className={className} {...rest} /> : <PrMark state={parts.state} number={parts.number} stale={stale} className={cn(className, parts.state === "closed" && CLOSED_DIM)} {...rest} />;
}

/**
 * A checkout's pull request on a card, a lens or a cleanup row: the mark and,
 * while a reviewer asks for changes, that one word in the failed colour; the
 * mark's colour says the rest.
 */
export function PrChipMark({ chip, stale = false }: { chip: PrChip; stale?: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className="inline-flex shrink-0 items-center gap-xs">
      <PrMark state={chip.state} number={chip.number} stale={stale} />
      {chip.changesRequested ? (
        <span className={cn("font-sans text-caption", CHANGES.tone)} data-pr-review="changes_requested">
          {t(CHANGES.key)}
        </span>
      ) : null}
    </span>
  );
}

const CHANGES = reviewWord("changes_requested")!;


/** The words a mark reads out: its number and state, or how many in each state, and when GitHub was last read. */
function useMarkLabel(parts: PrMarkParts | null, stale: PrStaleness | undefined): string {
  const { t, i18n } = useInterfaceTranslation();
  if (!parts) return "";
  const head = "counts" in parts
    ? [t("agentSessions.pr.count", { count: parts.counts.reduce((sum, { count }) => sum + count, 0) }), ...parts.counts.map(({ state, count }) => `${t(PR_LOOK[state].word)} ${count}`)]
    : [`#${parts.number} ${t(PR_LOOK[parts.state].word)}`];
  return [...head, staleLabel(stale, t, requireInterfaceLanguage(i18n.language))].filter(Boolean).join(" · ");
}

/** The class that makes a mark pressable: the fill under the pointer, the focus ring. */
export const PR_MARK_PRESS = "flex shrink-0 items-center rounded-xs px-xxs outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-secondary disabled:opacity-(--opacity-disabled)";

/**
 * A row's pull requests as one mark (PRD sidebar-task-lens B14): the core's
 * list (`AgentRow.pulls`), its session's and every descendant's, or a
 * 프로젝트 tree child row's own. One PR opens on a press, several list each
 * PR, worst first, and choosing one opens it; ⌘-press opens one PR on
 * GitHub. Under the pointer or focus it shows what the 프로젝트 row's mark
 * shows: the PR card for one PR, the list for several.
 */
export function AgentPrMark({ agent, pulls, disabled = false, side, onOpen, onExternal }: { agent: AgentRow; pulls: SessionPull[]; disabled?: boolean; side?: "right" | "bottom"; onOpen: (pull: SessionPull) => void; onExternal?: (pull: SessionPull) => void }) {
  const { t } = useInterfaceTranslation();
  const parts = prMarkParts(pulls);
  const staleness = pullsStaleness(pulls);
  const baseLabel = useMarkLabel(parts, staleness);
  const label = [baseLabel, recordWords(t, agent)].filter(Boolean).join(" · ");
  if (!parts) return null;
  const open = (pull: SessionPull, external: boolean) => (external && onExternal ? onExternal(pull) : onOpen(pull));
  const openUrl = (url: string, external: boolean) => {
    const pull = pulls.find((entry) => entry.url === url);
    if (!pull) throw new Error("PR card names a pull request outside its mark");
    open(pull, external);
  };
  const only = pulls.length === 1 ? pulls[0]! : null;
  if (only) {
    return (
      <PrHoverCard agent={agent} pulls={pulls} staleness={staleness} side={side} onOpenPullRequest={openUrl}>
        <button type="button" disabled={disabled} aria-label={label} data-agent-pr={agent.pane_id} className={cn(PR_MARK_PRESS, recordPending(agent) && "opacity-(--opacity-dimmed)")} data-record-provisional={recordPending(agent) || undefined}
          onClick={(event) => {
            event.stopPropagation();
            open(only, event.metaKey || event.ctrlKey);
          }}>
          <PartsMark parts={parts} stale={staleness?.stale} />
        </button>
      </PrHoverCard>
    );
  }
  return (
    <DropdownMenu>
      <PrHoverCard agent={agent} pulls={pulls} staleness={staleness} side={side} onOpenPullRequest={openUrl}>
        <DropdownMenuTrigger asChild>
          <button type="button" disabled={disabled} aria-label={label} data-agent-pr={agent.pane_id} onClick={(event) => event.stopPropagation()} className={cn(PR_MARK_PRESS, recordPending(agent) && "opacity-(--opacity-dimmed)")} data-record-provisional={recordPending(agent) || undefined}>
            <PartsMark parts={parts} stale={staleness?.stale} />
          </button>
        </DropdownMenuTrigger>
      </PrHoverCard>
      <DropdownMenuContent align="start" className="w-(--size-pr-popover)" onClick={(event) => event.stopPropagation()} data-pr-list={agent.pane_id}>
        <PrListHead agent={agent} count={pulls.length} />
        {pulls.map((pull) => (
          <DropdownMenuItem key={pull.url} aria-label={[`#${pull.number} ${t(PR_LOOK[pull.state].word)}`, pull.title, holderNames(pull)].filter(Boolean).join(" · ")} onSelect={() => onOpen(pull)} data-pr-list-item={pull.number}>
            <PrListItem pull={pull} />
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function PrListHead({ agent, count }: { agent: AgentRow; count: number }) {
  const { t } = useInterfaceTranslation();
  return (
    <DropdownMenuLabel className="flex min-w-0 items-baseline gap-xs">
      <span className="shrink-0">{t("agentSessions.pr.count", { count })}</span>
      <AgentRecordTitle agent={agent} className="font-normal text-muted-foreground" />
    </DropdownMenuLabel>
  );
}

/** Who holds or made a listed PR, when someone other than the row's own session does. */
function holderNames(pull: SessionPull): string | null {
  return pull.own && pull.holders.length <= 1 ? null : pull.holders.join(" · ") || null;
}

function PrListItem({ pull }: { pull: SessionPull }) {
  const { t } = useInterfaceTranslation();
  const names = holderNames(pull);
  return (
    <>
      <PrMark role="img" aria-label={`#${pull.number} ${t(PR_LOOK[pull.state].word)}`} state={pull.state} number={pull.number} stale={pull.stale} className={pull.state === "closed" ? CLOSED_DIM : undefined} />
      <span className={cn("min-w-0 flex-1", pull.state === "closed" && CLOSED_DIM)} data-pr-closed={pull.state === "closed" || undefined}>
        <span className="block truncate" title={pull.title}>{pull.title}</span>
        {names ? <span className="block truncate text-micro text-muted-foreground" title={names}>{names}</span> : null}
      </span>
    </>
  );
}

/**
 * The sidebar row's and the tree popover's mark: the same mark, not a menu,
 * over the list the caller chose of the row's (`rowPulls`). Under the pointer
 * `card` draws what it opens (the PR card for one PR, the list for several);
 * with none, a stale mark still says when GitHub was read.
 */
export function AgentPrHover({ agent, pulls, card }: { agent: AgentRow; pulls: SessionPull[]; card?: (mark: ReactNode) => ReactNode }) {
  const { t } = useInterfaceTranslation();
  const parts = prMarkParts(pulls);
  const staleness = pullsStaleness(pulls);
  const baseLabel = useMarkLabel(parts, staleness);
  const label = [baseLabel, recordWords(t, agent)].filter(Boolean).join(" · ");
  if (!parts) return null;
  const node = <PartsMark role="img" aria-label={label} data-agent-pr={agent.pane_id} parts={parts} stale={staleness?.stale} className={recordPending(agent) ? "opacity-(--opacity-dimmed)" : undefined} data-record-provisional={recordPending(agent) || undefined} />;
  if (card) return <>{card(node)}</>;
  return staleness?.stale || recordPending(agent) ? <Hint label={label}>{node}</Hint> : node;
}

/**
 * Under the pointer, each of the mark's PRs worst first with its mark, title
 * and, for a root's family, the sessions that hold it, and when GitHub was
 * last read if it cannot be read now. Choosing a PR opens it; the list stays
 * open while the pointer moves into it.
 */
function PrHoverList({ agent, pulls, staleness, side = "right", onOpen, children }: { agent: AgentRow; pulls: SessionPull[]; staleness?: PrStaleness; side?: "right" | "bottom"; onOpen: (pull: SessionPull) => void; children: ReactNode }) {
  const { t, i18n } = useInterfaceTranslation();
  const { open, onOpenChange, triggerProps } = useHintOpen();
  const stale = staleLabel(staleness, t, requireInterfaceLanguage(i18n.language));
  return (
    <Tooltip open={open} onOpenChange={onOpenChange} disableHoverableContent={false}>
      <TooltipTrigger asChild {...triggerProps}>
        {children}
      </TooltipTrigger>
      <TooltipContent side={side} align="start" aria-label={t("agentSessions.pr.list", { name: agent.identity_label })} className="pointer-events-auto flex w-(--size-pr-popover) flex-col gap-xxs rounded-md p-xs text-left" data-pr-list={agent.pane_id}>
        {pulls.map((pull) => (
          <button key={pull.url} type="button" onClick={() => onOpen(pull)} data-pr-list-item={pull.number} className="flex min-w-0 items-center gap-xs rounded-xs px-xs py-xxs text-left outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring">
            <PrListItem pull={pull} />
          </button>
        ))}
        {stale ? <span className="px-xs text-caption text-muted-foreground">{stale}</span> : null}
      </TooltipContent>
    </Tooltip>
  );
}

/**
 * Under the pointer or focus, a PR mark shows the existing PR card for one
 * PR, and each PR with its state and title for several (B23), a root's
 * family naming who holds each. The 프로젝트 row, the 태스크 row and the pane
 * header draw the same card.
 */
export function PrHoverCard({ agent, pulls, staleness, side, onOpenPullRequest, children }: { agent: AgentRow; pulls: SessionPull[]; staleness?: PrStaleness; side?: "right" | "bottom"; onOpenPullRequest: (url: string, external: boolean) => void; children: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const trigger = <span className="relative z-10 flex shrink-0" data-pr-hover={agent.pane_id}>{children}</span>;
  const single = pulls.length === 1 ? pulls[0]! : null;
  const pr = useShellStore((s) => (single ? catalogWorkspaces(s.rest).flatMap((project) => project.pull_requests ?? []).find((row) => row.url === single.url) : undefined));
  if (pr && !staleness?.stale) {
    return (
      <CheckoutCardHint card={pullRequestCard(pr, t)} description={`#${pr.number} ${pr.title}`} side={side} onOpenPullRequest={onOpenPullRequest}>
        {trigger}
      </CheckoutCardHint>
    );
  }
  return <PrHoverList agent={agent} pulls={pulls} staleness={staleness} side={side} onOpen={(pull) => onOpenPullRequest(pull.url, false)}>{trigger}</PrHoverList>;
}
