import { ArrowUpRightIcon } from "lucide-react";
import type { MouseEvent, ReactNode } from "react";
import { badgeParts } from "../agentRow";
import { holdsCommandKey } from "../host";
import { useInterfaceTranslation } from "../i18n/client";
import { cardSingleValue, type CheckoutCard } from "../projects";
import { PR_LOOK } from "../prMark";
import { BadgeMarks } from "./status-badge";
import { Badge } from "./ui/badge";
import { Hint, Tooltip, TooltipContent, TooltipTrigger, useHintOpen } from "./ui/tooltip";

/** What a press on the card's link means: ⌘ (Ctrl off macOS) asks for the default browser (PRD checkout-pr-glyph-card D-02). */
export function pullRequestOpenExternal(event: MouseEvent): boolean {
  return holdsCommandKey(event);
}

/**
 * The card a checkout row opens on hover and keyboard focus (PRD
 * checkout-pr-glyph-card D-03, D-04): a pull request's badge, number and
 * Open PR link over its title, then the rows `checkoutCard` gave it. It is
 * the shell's tooltip with hoverable content, so it opens after the same
 * delay, stays while the pointer crosses onto it, and closes when the pointer
 * leaves the row and the card, on Escape, and on a press on the row. Escape
 * closes it through the tooltip's own dismiss, not the shell's Escape owner:
 * a card that a resting pointer opened must not take the Escape a focused
 * terminal is about to receive, which the shell's owner would consume. The
 * hidden tooltip text a screen reader gets is `description`, the detail
 * sentence the row's tooltip carried before. A card with one value and no
 * header is the plain `Hint` with that value instead (D-09).
 */
export function CheckoutCardHint({
  card,
  description,
  side = "right",
  onOpenPullRequest,
  onOpenWorkspace,
  onShowPullRequestRow,
  children,
}: {
  card: CheckoutCard;
  /** The screen reader's description of the row: the pull request, the agents, the branch and the path. */
  description: string;
  /** Where the card opens: beside a sidebar row, or under a pane header's mark. */
  side?: "right" | "bottom";
  /** Opens the pull request the header names; `external` asks for the default browser. */
  onOpenPullRequest: (url: string, external: boolean) => void;
  /** Where the card was opened from goes to the checkout's Workspace; the card then offers `↵ Workspace`, the same move (PRD overview-lenses-tiles-agents B16). */
  onOpenWorkspace?: () => void;
  /** The pull request's row on its Project's PRs tab; the header then offers `View on the PRs tab` (PRD overview-lenses-prs B21). */
  onShowPullRequestRow?: (number: number) => void;
  children: ReactNode;
}) {
  const { t } = useInterfaceTranslation();
  const { open, onOpenChange, triggerProps } = useHintOpen();
  const single = onOpenWorkspace ? null : cardSingleValue(card);
  if (single !== null) return <Hint label={single}>{children}</Hint>;
  const header = card.header;
  return (
    <Tooltip open={open} onOpenChange={onOpenChange} disableHoverableContent={false}>
      <TooltipTrigger asChild {...triggerProps}>
        {children}
      </TooltipTrigger>
      <TooltipContent
        side={side}
        align="start"
        aria-label={description}
        className="pointer-events-auto w-(--size-pr-popover) max-w-(--radix-tooltip-content-available-width) text-left text-wrap rounded-md p-md"
        data-checkout-card={header?.kind ?? "plain"}
      >
        <div className="flex flex-col gap-sm">
          {header?.kind === "pull_request" ? <PullRequestHeader header={header} onOpen={onOpenPullRequest} /> : null}
          {header?.kind === "pull_request" && onShowPullRequestRow ? (
            <Hint label={t("board.prLineage")}>
              <button
                type="button"
                data-checkout-card-prs-tab={header.number}
                className="inline-flex items-center gap-xxs self-start rounded-xs text-caption font-medium text-foreground outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring"
                onClick={(event) => {
                  event.stopPropagation();
                  onShowPullRequestRow(header.number);
                }}
              >
                {t("board.showOnPrsTab")}
              </button>
            </Hint>
          ) : null}
          {header?.kind === "missing" ? (
            <div className="text-subhead font-medium text-destructive" data-checkout-card-missing="true">
              {header.label}
            </div>
          ) : null}
          {onOpenWorkspace ? (
            <button
              type="button"
              data-checkout-card-workspace="true"
              className="inline-flex items-center gap-xxs self-start rounded-xs text-caption font-medium text-foreground outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring"
              onClick={(event) => {
                event.stopPropagation();
                onOpenWorkspace();
              }}
            >
              {t("graph.workspaceHint")}
            </button>
          ) : null}
          {header?.kind === "pull_request" ? <div className="border-t border-border" /> : null}
          <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-md gap-y-xs text-caption">
            {card.rows.map((row) => (
              <CardRow key={row.key} row={row} />
            ))}
          </dl>
        </div>
      </TooltipContent>
    </Tooltip>
  );
}

function PullRequestHeader({
  header,
  onOpen,
}: {
  header: Extract<NonNullable<CheckoutCard["header"]>, { kind: "pull_request" }>;
  onOpen: (url: string, external: boolean) => void;
}) {
  const { t } = useInterfaceTranslation();
  // The same mark the sidebar row draws: the icon carries the state's colour,
  // the word only says what the pull request is.
  const { icon: Icon, tone } = PR_LOOK[header.state];
  return (
    <>
      <div className="flex items-center gap-sm">
        <Badge variant="outline" className="gap-xs text-foreground" data-checkout-card-badge={header.badge.label}>
          <Icon aria-hidden="true" className={tone} />
          {header.badge.label}
        </Badge>
        {header.badge.draft ? (
          <span className="text-caption text-muted-foreground" data-checkout-card-draft="true">
            {t("agentSessions.pr.draft")}
          </span>
        ) : null}
        <span className="font-mono text-caption text-muted-foreground">#{header.number}</span>
        <span className="flex-1" />
        <button
          type="button"
          data-checkout-card-open={header.number}
          className="inline-flex items-center gap-xxs rounded-xs text-caption font-medium text-foreground outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring"
          onClick={(event) => {
            event.stopPropagation();
            onOpen(header.url, pullRequestOpenExternal(event));
          }}
        >
          {t("board.openPr")}
          <ArrowUpRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />
        </button>
      </div>
      {header.title ? (
        <div className="line-clamp-2 text-subhead font-medium text-foreground" data-checkout-card-title="true">
          {header.title}
        </div>
      ) : null}
    </>
  );
}

function CardRow({ row }: { row: CheckoutCard["rows"][number] }) {
  return (
    <>
      <dt className="text-muted-foreground">{row.label}</dt>
      <dd className="min-w-0" data-checkout-card-row={row.key}>
        {row.key === "agents" ? (
          <span className="inline-flex items-center gap-sm font-mono">
            <BadgeMarks parts={badgeParts(row.marks)} />
          </span>
        ) : row.key === "path" ? (
          <span className="break-all font-mono text-muted-foreground">{row.value}</span>
        ) : row.key === "branch" ? (
          <span className="break-all font-mono text-foreground">{row.value}</span>
        ) : row.key === "review" || row.key === "checks" ? (
          <span className={row.tone}>{row.value}</span>
        ) : (
          <span className="text-foreground">{row.value}</span>
        )}
      </dd>
    </>
  );
}
