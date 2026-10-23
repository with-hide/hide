import { ChevronDownIcon, ChevronRightIcon, EllipsisIcon, ExternalLinkIcon, PlayIcon } from "lucide-react";
import { useState, type KeyboardEvent, type MouseEvent } from "react";
import type { Actions } from "./actions";
import type { PrGroupId } from "./agentScope";
import { markTone, rowLine, lineTone } from "./agentRow";
import { AgentMessagePopover } from "./AgentMessagePopover";
import { PrMark } from "./components/pr-mark";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./components/ui/dropdown-menu";
import { Hint } from "./components/ui/tooltip";
import { Elapsed } from "./components/elapsed";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { holdsCommandKey } from "./host";
import { cn } from "./lib/utils";
import { issueDate } from "./projectBoard";
import { FOLDED_GROUPS, opensCard, type PrEntry, type PrGroupView, type PrListModel } from "./prWork";
import { GROUP_TONE, ReasonLine } from "./prReason";
import { IssuePicker } from "./PrLinks";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// The PRs tool's list (PRD work-tab-pr-lens B6-B19, B39-B43, B47; sidebar-task-lens D-17):
// the pull requests the core lists, grouped by whose move it is, two lines
// each. Nothing here decides a group or a reason; the core did.

const MARKS = 3;

/** Stops a part's click from also opening the row. */
function own(handler: (event: MouseEvent) => void) {
  return (event: MouseEvent) => {
    event.stopPropagation();
    handler(event);
  };
}

export function PrList({ model, actions, onlyCheckout, showProject, onAllCheckouts }: {
  model: PrListModel;
  actions: Actions;
  /** The scope chip narrows to one checkout. */
  onlyCheckout: boolean;
  /** Home gathers several projects: each row names its own. */
  showProject: boolean;
  onAllCheckouts: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const [opened, setOpened] = useState<ReadonlySet<string>>(new Set());
  const toggle = (key: string) => setOpened((before) => {
    const next = new Set(before);
    if (!next.delete(key)) next.add(key);
    return next;
  });
  const { reading } = model;
  if (reading.state === "blocked") return <GithubLine category={reading.category} actions={actions} />;
  if (reading.state === "reading" && model.groups.length === 0) {
    return (
      <div className="flex flex-col gap-xs px-md py-sm" role="status" aria-label={t("work.reading")} data-prs-view="reading">
        {[0, 1, 2].map((index) => <span key={index} className="block h-(--size-control-lg) rounded-sm bg-muted" data-pr-skeleton={index} />)}
      </div>
    );
  }
  const open = model.groups.filter((group) => group.group !== "merged");
  const keys = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    const step = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : null;
    if (step === null && event.key !== "Home" && event.key !== "End") return;
    const items = [...event.currentTarget.querySelectorAll<HTMLElement>("[data-pr-focus]")];
    const index = items.indexOf((event.target as HTMLElement).closest<HTMLElement>("[data-pr-focus]") as HTMLElement);
    const next = event.key === "Home" ? items[0] : event.key === "End" ? items.at(-1) : items[Math.max(0, Math.min(items.length - 1, index + (step ?? 0)))];
    if (next) {
      event.preventDefault();
      next.focus();
      next.scrollIntoView({ block: "nearest" });
    }
  };
  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-sm pb-md" data-prs-view="board" onKeyDown={keys}>
      {open.length === 0 ? (
        <div className="flex flex-col items-start gap-sm px-xs py-md text-caption text-muted-foreground" data-prs-empty={onlyCheckout && model.total === 0 ? "checkout" : "none"}>
          <p>{onlyCheckout && model.total === 0 ? t("work.empty.checkout") : t("work.empty.none")}</p>
          {onlyCheckout && model.total === 0 ? <Button variant="secondary" size="sm" onClick={onAllCheckouts}>{t("agentSessions.allCheckouts")}</Button> : null}
        </div>
      ) : null}
      {model.groups.map((view) => (
        <Group key={view.group} view={view} opened={opened} onToggle={toggle} actions={actions} showProject={showProject} />
      ))}
    </div>
  );
}

function Group({ view, opened, onToggle, actions, showProject }: { view: PrGroupView; opened: ReadonlySet<string>; onToggle: (key: string) => void; actions: Actions; showProject: boolean }) {
  const { t } = useInterfaceTranslation();
  const count = view.entries.length + view.team.length + view.old.length;
  // Everything after Blocked starts folded (B7); a click turns it the other way.
  const startsFolded = FOLDED_GROUPS.has(view.group);
  const expanded = startsFolded ? opened.has(view.group) : !opened.has(view.group);
  const quiet = view.group === "bots" || view.group === "waiting" || view.group === "drafts" || view.group === "merged";
  return (
    <section data-pr-group={view.group} data-folded={expanded ? undefined : "true"} aria-label={t(`work.group.${view.group}`)}>
      <button type="button" data-pr-focus="group" aria-expanded={expanded} onClick={() => onToggle(view.group)}
        className={cn("flex w-full items-center gap-xs px-xs py-sm text-left text-caption font-medium outline-none focus-visible:ring-1 focus-visible:ring-ring", quiet ? "text-subtle-foreground" : GROUP_TONE[view.group] === "text-warning" ? "text-warning" : "text-subtle-foreground")}>
        {expanded ? <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
        <span>{t(`work.group.${view.group}`)}</span>
        <span className="tabular-nums text-muted-foreground">{count}</span>
      </button>
      {expanded ? (
        <ul className="flex min-w-0 flex-col" role="list">
          {view.entries.map((entry) => <PrRow key={entry.key} entry={entry} actions={actions} showProject={showProject} />)}
          <Fold view={view} kind="team" entries={view.team} opened={opened} onToggle={onToggle} actions={actions} showProject={showProject} />
          <Fold view={view} kind="old" entries={view.old} opened={opened} onToggle={onToggle} actions={actions} showProject={showProject} />
        </ul>
      ) : null}
    </section>
  );
}

/** A folded line inside a group: team review requests, or pull requests nothing moved on (B15, B16). */
function Fold({ view, kind, entries, opened, onToggle, actions, showProject }: { view: PrGroupView; kind: "team" | "old"; entries: PrEntry[]; opened: ReadonlySet<string>; onToggle: (key: string) => void; actions: Actions; showProject: boolean }) {
  const { t } = useInterfaceTranslation();
  if (entries.length === 0) return null;
  const key = `${view.group}:${kind}`;
  const expanded = opened.has(key);
  return (
    <>
      <li className="flex min-w-0" data-pr-fold={kind}>
        <button type="button" data-pr-focus="fold" aria-expanded={expanded} onClick={() => onToggle(key)}
          className="flex min-w-0 flex-1 items-center gap-xs rounded-xs py-xs pl-lg pr-xs text-left text-caption text-subtle-foreground outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring">
          {expanded ? <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" /> : <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />}
          <span className="min-w-0 truncate">{t(`work.fold.${kind}`, { count: entries.length })}</span>
        </button>
      </li>
      {expanded ? entries.map((entry) => <PrRow key={entry.key} entry={entry} actions={actions} showProject={showProject} folded={kind} />) : null}
    </>
  );
}

/**
 * One pull request on two lines (B19): its mark and number, its title, the
 * agent marks or a teammate's name and the time; then the reason. The row is
 * one button that opens the card (⌘-click and ⌘↵: GitHub); every part above
 * it is its own destination.
 */
function PrRow({ entry, actions, showProject, folded }: { entry: PrEntry; actions: Actions; showProject: boolean; folded?: "team" | "old" }) {
  const { t, i18n } = useInterfaceTranslation();
  const devices = useShellStore((state) => state.rest?.navigator?.devices);
  const { pr, project, row, reason } = entry;
  const group = row.group as PrGroupId;
  const github = () => actions.openPullRequest(pr.url, project.device_id, true);
  const card = opensCard(project, devices);
  const open = () => (card ? actions.openPrCard(project.id, pr.number, activeCheckout(entry)) : github());
  const [linking, setLinking] = useState(false);
  const mine = pr.work?.mine === true;
  const author = !mine && pr.work?.author ? pr.work.author : null;
  const lastMoved = folded === "old" && pr.updated_at_unix_ms != null ? issueDate(pr.updated_at_unix_ms, requireInterfaceLanguage(i18n.language)) : null;
  return (
    <li className="flex flex-col" data-pr={pr.number} data-pr-project={project.id} data-pr-row-group={group}>
      <div className={cn("group/pr-row relative flex min-w-0 flex-col gap-xxs rounded-xs px-xs py-xs hover:bg-accent focus-within:bg-accent", group === "merged" && "opacity-(--opacity-secondary)")}>
        <button type="button" data-pr-focus="row" data-pr-row={pr.number} aria-label={t("work.row.open", { number: pr.number, title: pr.title })}
          className="absolute inset-0 rounded-xs outline-none focus-visible:ring-1 focus-visible:ring-ring"
          onClick={(event) => (holdsCommandKey(event) ? github() : open())}
          onKeyDown={(event) => {
            if (event.key === "Enter" && holdsCommandKey(event)) {
              event.preventDefault();
              github();
            }
          }} />
        <div className="pointer-events-none relative flex min-h-(--size-sidebar-line) min-w-0 items-center gap-xs">
          <PrMark state={pr.state} number={pr.number} />
          <Hint label={pr.title} reveals>
            <span className="pointer-events-auto min-w-0 flex-1 cursor-pointer truncate text-caption font-medium text-foreground" onClick={(event) => (holdsCommandKey(event) ? github() : open())} data-pr-title={pr.number}>{pr.title}</span>
          </Hint>
          {showProject ? <span className="max-w-[40%] shrink truncate rounded-xs bg-secondary px-xs text-micro text-muted-foreground" data-pr-project-name={project.id}>{entry.device ? `${entry.device} · ${project.label}` : project.label}</span> : null}
          {author ? <span className="max-w-[30%] shrink-0 truncate text-micro text-muted-foreground group-focus-within/pr-row:invisible group-hover/pr-row:invisible group-has-data-[state=open]/pr-row:invisible" data-pr-author={author}>{t("work.row.author", { name: author })}</span> : null}
          <AgentMarks entry={entry} actions={actions} />
          <span className="relative flex shrink-0 items-center justify-end">
            <Elapsed since={pr.updated_at_unix_ms} className={cn("shrink-0 text-micro text-muted-foreground", "group-focus-within/pr-row:invisible group-hover/pr-row:invisible group-has-data-[state=open]/pr-row:invisible", entry.agents.length > 0 && "absolute right-0")} data-pr-age={pr.number} />
            <RowActions entry={entry} actions={actions} linking={linking} onLinking={setLinking} inFlow={entry.agents.length > 0} />
          </span>
        </div>
        <div className="pointer-events-none relative flex min-w-0 pl-[calc(var(--size-pr-icon)+var(--spacing-xs))]">
          <ReasonLine reason={reason} group={group} noAgent={group === "blocked"} className="whitespace-normal break-words" />
          {lastMoved ? <span className="ml-xs shrink-0 truncate text-micro text-muted-foreground" data-pr-last-moved="true">· {t("work.fold.lastMoved", { date: lastMoved })}</span> : null}
        </div>
      </div>
    </li>
  );
}

/** The checkout a row asks the card to open in, when the project's own front checkout is one of the PR's. */
function activeCheckout(entry: PrEntry): string | undefined {
  return entry.row.checkout_ids[0] ?? entry.row.checkout_id ?? undefined;
}

/** Up to three marks of the pull request's live agents and `+N`; one agent's mark opens its pane (B19). */
function AgentMarks({ entry, actions }: { entry: PrEntry; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const agents = entry.agents;
  if (agents.length === 0) return null;
  return (
    <span className="pointer-events-auto relative flex shrink-0 items-center gap-xxs" data-pr-agents={agents.length}>
      {agents.slice(0, MARKS).map((agent) => {
        const said = rowLine(agent);
        return (
          <AgentMessagePopover key={agent.pane_id} agent={agent} place={entry.pr.head_branch ?? ""} fallback={said?.text ?? agent.identity_label} tone={said ? lineTone(said, agent) : "text-muted-foreground"} onOpen={() => actions.openAgent(agent.pane_id)}>
            <button type="button" aria-label={t("work.row.agent", { name: agent.identity_label })} className="inline-flex rounded-xs outline-none focus-visible:ring-1 focus-visible:ring-ring" onClick={own(() => actions.openAgent(agent.pane_id))} data-pr-agent={agent.pane_id}>
              <StatusMark mark={agent.mark} className={markTone(agent)} />
            </button>
          </AgentMessagePopover>
        );
      })}
      {agents.length > MARKS ? <span className="font-mono text-caption text-muted-foreground">+{agents.length - MARKS}</span> : null}
    </span>
  );
}

/**
 * The buttons that stand in the time slot under the pointer or the keyboard:
 * `▷ Assign` on a blocked row, `Clean up` on a merged one whose worktree is
 * still here, else GitHub, each with `⋯` for Link issue, New issue, Copy
 * branch name and Assign (B19).
 */
function RowActions({ entry, actions, linking, onLinking, inFlow }: { entry: PrEntry; actions: Actions; linking: boolean; onLinking: (open: boolean) => void; inFlow: boolean }) {
  const { t } = useInterfaceTranslation();
  const { pr, project, row } = entry;
  // Over the time slot, or, beside agent marks, holding its own width so a
  // hover never covers a mark the pointer is about to press (B41).
  const reveal = cn("invisible flex items-center gap-xxs group-focus-within/pr-row:visible group-hover/pr-row:visible has-data-[state=open]:visible", !inFlow && "absolute inset-y-0 right-0");
  const pointer = "pointer-events-auto";
  const merged = row.group === "merged" && pr.state === "merged";
  const checkout = row.checkout_id ? project.checkouts.find((candidate) => candidate.id === row.checkout_id) ?? null : null;
  const folderGone = checkout !== null && (!checkout.exists || checkout.worktree?.missing === true);
  const worktree = checkout?.is_worktree === true && checkout.is_primary !== true;
  const local = project.tasks?.source?.kind === "local";
  const linkable = !merged && row.issue === null && (!local || (worktree && !folderGone));
  const delegate = () => useUiStore.getState().setWorkspaceDialog({ kind: "pr_delegate", workspaceId: project.id, prNumber: pr.number });
  const cleanup = merged && worktree ? () => useUiStore.getState().setWorkspaceDialog({ kind: "delete_worktree", workspaceId: project.id, checkoutId: checkout!.id }) : null;
  const rowMenu = (
    <IssuePicker number={pr.number} project={project} open={linking} onOpenChange={onLinking}>
      <span className="inline-flex">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon-sm" aria-label={t("prList.actions")} onClick={(event) => event.stopPropagation()} data-pr-menu={pr.number}>
              <EllipsisIcon aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" onClick={(event) => event.stopPropagation()}>
            {row.group !== "merged" ? <DropdownMenuItem onSelect={delegate} data-pr-menu-delegate="true">{t("prList.delegate")}</DropdownMenuItem> : null}
            {linkable ? <DropdownMenuItem onSelect={() => onLinking(true)} data-pr-menu-link="true">{t("prList.linkIssue")}</DropdownMenuItem> : null}
            {linkable ? <DropdownMenuItem onSelect={() => useUiStore.getState().setWorkspaceDialog({ kind: "pr_new_issue", workspaceId: project.id, prNumber: pr.number })} data-pr-menu-new-issue="true">{t("prList.newIssue")}</DropdownMenuItem> : null}
            <DropdownMenuItem onSelect={() => actions.openPullRequest(pr.url, project.device_id, true)} data-pr-menu-github="true">{t("work.card.github")}</DropdownMenuItem>
            <DropdownMenuItem onSelect={() => void actions.copyText(pr.head_branch ?? "", "branch name")} data-pr-menu-copy="true">{t("prList.copyBranch")}</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </span>
    </IssuePicker>
  );
  if (row.group === "blocked") {
    return (
      <span className={cn(reveal, pointer)} data-pr-actions="delegate">
        <Hint label={t("prList.takeHint")}>
          <Button variant="secondary" size="sm" onClick={own(delegate)} data-pr-delegate-open={pr.number}>
            <PlayIcon aria-hidden="true" />
            {t("prList.delegate")}
          </Button>
        </Hint>
        {rowMenu}
      </span>
    );
  }
  if (cleanup) {
    return (
      <span className={cn(reveal, pointer)} data-pr-actions="cleanup">
        <Hint label={t("prList.cleanHint")}>
          <Button variant="ghost" size="sm" onClick={own(cleanup)} data-pr-cleanup={folderGone ? "record" : "worktree"}>{t("work.row.cleanup")}</Button>
        </Hint>
        {rowMenu}
      </span>
    );
  }
  return (
    <span className={cn(reveal, pointer)} data-pr-actions="default">
      <Hint label={t("prList.gitHubHint")}>
        <Button variant="ghost" size="icon-sm" aria-label="GitHub" onClick={own(() => actions.openPullRequest(pr.url, project.device_id, true))} data-pr-github={pr.number}>
          <ExternalLinkIcon aria-hidden="true" />
        </Button>
      </Hint>
      {rowMenu}
    </span>
  );
}

/** `gh` is not installed or not signed in: one line and where to fix it (B42). */
function GithubLine({ category, actions }: { category: "not_installed" | "not_logged_in"; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex flex-col items-start gap-sm px-md py-md text-caption text-muted-foreground" data-prs-view="blocked" data-prs-gh={category}>
      <p>{t(category === "not_installed" ? "issueSettings.ghNotInstalled" : "issueSettings.ghNotLoggedIn")}</p>
      <Button variant="secondary" size="sm" onClick={() => actions.openSettings("github")} data-prs-open-settings="true">{t("work.gh.settings")}</Button>
    </div>
  );
}
