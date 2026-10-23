import { CheckCircle2Icon, LoaderCircleIcon, ChevronDownIcon, ChevronRightIcon, EllipsisIcon, ExternalLinkIcon, SquareTerminalIcon, TriangleAlertIcon } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./components/ui/dropdown-menu";
import { Hint } from "./components/ui/tooltip";
import { PrMark } from "./components/pr-mark";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { LinkTimeline } from "./LinkSessions";
import { renderMarkdown } from "./mobile/markdown";
import { boardProjects } from "./navigation";
import { checkSummary, confirmHolds, defaultMethod, fileOpen, mergeWhy, milestones, prFacts, primaryAction, splitFiles, type Confirmed, type PrFact, type PrimaryAction } from "./prCardModel";
import { IssuePicker, PrLinks } from "./PrLinks";
import { ReasonLine } from "./prReason";
import { badgeWord, staleLabel, staleness } from "./prMark";
import { findPrEntry, opensCard, type PrEntry } from "./prWork";
import { requireInterfaceLanguage } from "./i18n/locale";
import { catalogWorkspaces, frontCheckout, type LinkPanel, type PrCard, type PrMergeMethod, type PullRequest, type Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { useRequestViewDemand } from "./requestViewDemand";

// The PR card keeps its list identity, CI counts, merge controls, timeline,
// changed files and Markdown body in the lane's existing detail slot.
// Agent actions belong to the timeline; each remotely read part keeps its
// own loading, failure and retry state.

/**
 * Whether the operator opened the files line, per card. The card leaves the
 * screen when a changed file turns the lane to Files, and the 보기 slot
 * brings it back as it was left (B49); the page keeps it while it runs,
 * never stored.
 */
const OPENED = new Map<string, boolean>();
/** As many cards as the slot steps back through (`DETAIL_HISTORY_LIMIT`); past it the least recently changed card forgets its line. */
const OPENED_LIMIT = 20;

function useFilesOpen(key: string): [boolean, () => void] {
  const [open, setOpen] = useState(() => OPENED.get(key) ?? false);
  useEffect(() => setOpen(OPENED.get(key) ?? false), [key]);
  const toggle = () => {
    const next = !open;
    OPENED.delete(key);
    OPENED.set(key, next);
    if (OPENED.size > OPENED_LIMIT) OPENED.delete(OPENED.keys().next().value!);
    setOpen(next);
  };
  return [open, toggle];
}

/** The catalog project `workspaceId` and its pull request `number`, with the list's row for it. */
function useCardTarget(workspaceId: string, number: number) {
  const rest = useShellStore((state) => state.rest);
  const agents = useShellStore((state) => state.agents);
  return useMemo(() => {
    const project = catalogWorkspaces(rest).find((row) => row.id === workspaceId) ?? null;
    if (!project) return { project: null, entry: null, pr: null };
    const scope = boardProjects(rest, agents, project.device_id).find((item) => item.workspace.id === project.id);
    const pr = project.pull_requests?.find((row) => row.number === number) ?? null;
    return { project, entry: scope ? findPrEntry(scope, number) : null, pr };
  }, [rest, agents, workspaceId, number]);
}

export function PrCardView({ workspaceId, number, actions }: { workspaceId: string; number: number; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { project, entry, pr } = useCardTarget(workspaceId, number);
  const card = useShellStore((state) => (project ? state.rest?.pr_work?.cards?.find((row) => row.workspace_id === project.id && row.number === number) ?? null : null));
  const record = useShellStore((state) => state.linkPanel);
  const devices = useShellStore((state) => state.rest?.navigator?.devices);
  const live = useShellStore((state) => state.connection === "live");
  const [filesOpen, toggleFiles] = useFilesOpen(`${workspaceId}#${number}`);
  const projectId = project?.id ?? null;
  const updatedAt = pr?.updated_at_unix_ms ?? null;
  const checks = pr?.checks ?? null;
  // The card's own parts are read again when its pull request moved (B44);
  // the core reads them on the event that opened the slot. The check and
  // review times of the timeline ride on the same read (D-15).
  const reads = project ? opensCard(project, devices) : false;
  useEffect(() => {
    if (live && projectId && reads) actions.readPrCard(projectId, number);
  }, [actions, live, projectId, reads, number, updatedAt, checks]);
  // Share the visible-board demand so pending checks are read again every minute (B44).
  useRequestViewDemand(actions, live);
  // The link record: the sessions that made or worked on it and what it is linked to.
  useEffect(() => {
    if (!projectId) return;
    actions.openLinks(projectId, { kind: "pr", number });
    return () => actions.closeLinks();
  }, [actions, projectId, number]);
  const scroller = useRef<HTMLElement>(null);
  if (!project) return <p className="p-lg text-caption text-muted-foreground" data-pr-card-missing={number}>{t("work.card.unlisted", { number })}</p>;
  const panel: LinkPanel | null = record && record.workspace_id === project.id && record.target.kind === "pr" && record.target.number === number ? record : null;
  return (
    <article ref={scroller} aria-label={t("work.card.aria", { number })} className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto [overflow-wrap:anywhere] text-foreground" data-pr-card={number} data-pr-card-project={project.id} data-pr-card-head={card?.head ?? undefined}>
      <div className="flex w-full flex-col gap-lg p-lg">
        {pr ? <Head project={project} pr={pr} entry={entry} card={card} panel={panel} actions={actions} /> : <p className="text-caption text-muted-foreground" data-pr-card-unlisted={number}>{t("work.card.unlisted", { number })}</p>}
        {pr ? <DecisionBox project={project} pr={pr} entry={entry} card={card} actions={actions} /> : null}
        <Timeline project={project} pr={pr} card={card} panel={panel} number={number} actions={actions} />
        {pr ? <FilesRow pr={pr} project={project} card={card} actions={actions} open={filesOpen} onToggle={toggleFiles} /> : null}
        {pr ? <Body card={card} actions={actions} projectId={project.id} number={number} scroller={scroller} /> : null}
      </div>
    </article>
  );
}

// --- head ---------------------------------------------------------------------

/** PR identity and GitHub access, wrapping title, branch facts and actual links. */
function Head({ project, pr, entry, card, panel, actions }: { project: Workspace; pr: PullRequest; entry: PrEntry | null; card: PrCard | null; panel: LinkPanel | null; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const checkoutId = entry?.row.checkout_ids[0] ?? entry?.row.checkout_id ?? null;
  const checkout = checkoutId ? project.checkouts.find((row) => row.id === checkoutId && row.exists) ?? null : null;
  const author = pr.work && pr.work.mine !== true ? pr.work.author : null;
  const files = card?.files ?? null;
  const added = files?.reduce((sum, file) => sum + file.additions, 0) ?? 0;
  const removed = files?.reduce((sum, file) => sum + file.deletions, 0) ?? 0;
  const stale = staleness(project.checkouts.find((row) => row.github)?.github);
  const staleText = staleLabel(stale, t, requireInterfaceLanguage(i18n.language));
  return (
    <header className="flex flex-col gap-xs" data-pr-card-header="true">
      <div className="flex min-w-0 flex-wrap items-center gap-xs">
        <button type="button" className="mr-auto rounded-xs outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring" onClick={() => actions.openPullRequest(pr.url, project.device_id, true)} data-pr-card-number={pr.number}><span className="inline-flex items-center gap-xs"><PrMark state={pr.state} number={pr.number} /><span className="text-micro text-muted-foreground">{badgeWord(pr, t).label}</span></span></button>
        <Button variant="outline" size="sm" onClick={() => actions.openPullRequest(pr.url, project.device_id, true)} data-pr-card-github="true">{t("recordCard.github")}<ExternalLinkIcon aria-hidden="true" /></Button>
        <CardMenu project={project} pr={pr} entry={entry} actions={actions} />
      </div>
      <h2 className="min-w-0 whitespace-normal text-subhead font-semibold [overflow-wrap:anywhere]" data-pr-card-title="true">{pr.title}</h2>
      {author ? <span className="text-caption text-muted-foreground" data-pr-card-author={author}>{t("work.row.author", { name: author })}</span> : null}
      <p className="flex min-w-0 flex-wrap items-center gap-x-xs gap-y-xxs font-mono text-micro text-muted-foreground" data-pr-card-facts="true">
        <span className="min-w-0 truncate" data-pr-card-branch={pr.head_branch}>{pr.head_branch}</span>
        {pr.base_branch ? <><span aria-hidden="true">→</span><span className="min-w-0 truncate">{pr.base_branch}</span></> : null}
        {files ? <><span aria-hidden="true">·</span><span className="font-sans" data-pr-card-files={card?.files_total ?? files.length}>{t("work.card.files", { count: card?.files_total ?? files.length })}</span><span aria-hidden="true">·</span><span className="text-pr-mergeable">+{added}</span><span className="text-pr-failed">−{removed}</span></> : null}
        {staleText ? <Hint label={staleText}><span role="img" aria-label={staleText} className="inline-flex text-warning" data-pr-card-stale="true"><TriangleAlertIcon aria-hidden="true" className="size-(--size-icon-sm)" /></span></Hint> : null}
      </p>
      {checkout ? <Button variant="ghost" size="sm" className="self-start px-none" onClick={() => actions.openWorkspace(project.device_id, checkout.workspace_id, checkout.id)} data-pr-card-workspace={pr.number}><SquareTerminalIcon aria-hidden="true" />{t("work.card.workspace")}</Button> : null}
      <PrLinks issue={entry?.row.issue ?? null} checkout={checkout} record={panel} project={project} actions={actions} />
    </header>
  );
}

/** `⋯` of the card (D-28): hand the PR to an agent, make an issue for it, copy the branch name. */
function CardMenu({ project, pr, entry, actions }: { project: Workspace; pr: PullRequest; entry: PrEntry | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const merged = pr.state === "merged" || pr.state === "closed";
  const checkout = entry?.row.checkout_id ? project.checkouts.find((row) => row.id === entry.row.checkout_id) : null;
  const worktree = checkout?.is_worktree === true && !checkout.is_primary && checkout.exists && !checkout.worktree?.missing;
  const linkable = !merged && !entry?.row.issue && (project.tasks?.source?.kind !== "local" || worktree);
  const [linking, setLinking] = useState(false);
  return (
    <IssuePicker number={pr.number} project={project} open={linking} onOpenChange={setLinking}>
    <span className="inline-flex"><DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon-sm" className="shrink-0" aria-label={t("work.card.menu")} data-pr-card-menu={pr.number}>
          <EllipsisIcon aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" onCloseAutoFocus={(event) => { if (linking) event.preventDefault(); }}>
        {linkable ? <DropdownMenuItem onSelect={() => setLinking(true)}>{t("prList.linkIssue")}</DropdownMenuItem> : null}
        {!merged ? <DropdownMenuItem onSelect={() => useUiStore.getState().setWorkspaceDialog({ kind: "pr_delegate", workspaceId: project.id, prNumber: pr.number })}>{t("prList.delegate")}</DropdownMenuItem> : null}
        {linkable ? <DropdownMenuItem onSelect={() => useUiStore.getState().setWorkspaceDialog({ kind: "pr_new_issue", workspaceId: project.id, prNumber: pr.number })}>{t("prList.newIssue")}</DropdownMenuItem> : null}
        <DropdownMenuItem onSelect={() => void actions.copyText(pr.head_branch ?? "", "branch name")} data-pr-card-copy="true">{t("prList.copyBranch")}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu></span>
    </IssuePicker>
  );
}

// --- decision box ----------------------------------------------------------------

function factText(fact: PrFact, t: ReturnType<typeof useInterfaceTranslation>["t"]): string {
  switch (fact.kind) {
    case "checks_passing":
      return t("work.card.fact.checksPassing");
    case "checks_running":
      return t("work.card.why.ci", { count: fact.count });
    case "checks_failed":
      return t("work.card.why.failed", { count: fact.count });
    case "approved":
      return t("board.review.approved");
    case "changes":
      return t("board.review.changes");
    case "review_required":
      return t("board.review.required");
    case "conflict":
      return t("work.card.conflict", { base: fact.base });
    case "no_conflict":
      return t("work.card.fact.noConflict", { base: fact.base });
    case "behind":
      return t("work.card.fact.behind", { base: fact.base });
  }
}

/** Real CI counts beside the existing merge controls, with review facts and blockers below. */
function DecisionBox({ project, pr, entry, card, actions }: { project: Workspace; pr: PullRequest; entry: PrEntry | null; card: PrCard | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const counts = pr.work?.check_counts;
  const facts = prFacts(pr).filter((fact) => !counts || !fact.kind.startsWith("checks_"));
  const failed = (counts?.failed ?? 0) > 0 || pr.checks === "failed";
  const passing = pr.checks === "passing";
  const summary = checkSummary(pr);
  const Icon = failed ? TriangleAlertIcon : passing ? CheckCircle2Icon : LoaderCircleIcon;
  return (
    <section className="flex min-w-0 flex-col gap-xs border-t border-border pt-md" data-pr-card-reason={entry?.reason.kind} data-pr-card-group={entry?.row.group}>
      <Buttons project={project} pr={pr} entry={entry} card={card} actions={actions}>
        <div className="min-w-0 flex-1" data-pr-card-checks="true">
          <p className={cn("flex items-start gap-xs text-caption font-medium", failed ? "text-destructive" : passing ? "text-success" : "text-agent-working")}><Icon aria-hidden="true" className="mt-xxs size-(--size-icon-sm) shrink-0" /><span>{t(summary.key, summary.values)}</span></p>
          {counts && counts.total > 0 ? <p className="mt-xs flex flex-wrap gap-x-sm gap-y-xxs text-micro text-muted-foreground" data-pr-card-check-counts="true">{(["passed", "running", "waiting", "failed", "skipped", "unknown"] as const).filter((key) => counts[key] > 0).map((key) => <span key={key}>{t(`recordCard.ci.${key}`, { count: counts[key] })}</span>)}</p> : null}
          <button type="button" className="mt-xs inline-flex items-center gap-xxs rounded-xs text-micro text-muted-foreground outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring" onClick={() => actions.openPullRequest(`${pr.url}/checks`, project.device_id, true)}>{t("recordCard.checksLink")}<ExternalLinkIcon aria-hidden="true" className="size-(--size-icon-sm)" /></button>
        </div>
      </Buttons>
      {entry && ["queued", "unjudged", "checking"].includes(entry.reason.kind) ? <ReasonLine reason={entry.reason} group={entry.row.group} className="whitespace-normal [overflow-wrap:anywhere]" /> : null}
      {facts.length > 0 ? <p className="text-micro text-muted-foreground" data-pr-card-fact-line="true">{facts.map((fact) => factText(fact, t)).join(" · ")}</p> : null}
      <Blocked pr={pr} entry={entry} card={card} actions={actions} projectId={project.id} />
    </section>
  );
}

/** What blocks it (B33): a conflict, each failed check with its log, each unresolved thread. Nothing blocked, nothing drawn. */
function Blocked({ pr, entry, card, actions, projectId }: { pr: PullRequest; entry: PrEntry | null; card: PrCard | null; actions: Actions; projectId: string }) {
  const { t } = useInterfaceTranslation();
  const open = pr.state !== "merged" && pr.state !== "closed";
  const conflict = open && pr.work?.merge_state === "dirty";
  const failed = open ? pr.work?.failed_checks ?? [] : [];
  const threads = card?.threads ?? [];
  const device = entry?.project.device_id ?? "";
  // A thread read that failed says so with a retry, never "nothing blocked" (B37).
  const threadsFailed = card?.failed.includes("threads") === true;
  if (!conflict && failed.length === 0 && threads.length === 0 && !threadsFailed) return null;
  return (
    <ul className="flex flex-col gap-xxs" aria-label={t("work.card.blocked")} data-pr-card-blocked="true">
      {conflict ? <li className="text-caption text-pr-failed" data-pr-card-conflict="true">✗ {t("work.card.conflict", { base: pr.base_branch ?? "" })}</li> : null}
      {failed.map((check) => (
        <li key={check.name} className="flex min-w-0 items-center gap-xs text-caption" data-pr-card-failed={check.name}>
          <span className="min-w-0 flex-1 truncate"><span className="text-pr-failed">✗</span> <span className="font-mono">{check.name}</span></span>
          {check.url ? (
            <button type="button" className="inline-flex shrink-0 items-center gap-xxs rounded-xs text-micro text-muted-foreground outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring" onClick={() => actions.openPullRequest(check.url!, device, true)}>
              {t("work.card.checkLog")}
              <ExternalLinkIcon aria-hidden="true" className="size-(--size-icon-sm)" />
            </button>
          ) : null}
        </li>
      ))}
      {threadsFailed ? <li><PartState part="threads" card={card} actions={actions} projectId={projectId} number={pr.number} /></li> : null}
      {threads.map((thread, index) => {
        const place = thread.line == null ? thread.path : `${thread.path}:${thread.line}`;
        return <li key={`${thread.path}:${thread.line}:${index}`} className="min-w-0 truncate text-caption text-foreground" title={thread.text} data-pr-card-thread={place}>{t("work.card.thread", { reviewer: thread.reviewer ?? "?", place, text: thread.text })}</li>;
      })}
    </ul>
  );
}

type Confirm = Confirmed | null;

function confirmText(confirm: Confirmed, queue: boolean, t: (key: string, values?: Record<string, unknown>) => string, values: { number: number; base: string; method: string; head: string }): string {
  return t(confirm.action === "merge" ? (queue ? "work.card.confirm.queue" : "work.card.confirm.merge") : "work.card.confirm.update", values);
}

/**
 * The box's first line and its buttons (D-21, B33, B37): the case's word at
 * the left, the case's button at the right, and Merge, off with its real
 * reason beside it while it cannot be pressed. Merge and a branch update ask
 * once and go out on the head the card showed; what they did stands in one
 * line under them.
 */
function Buttons({ project, pr, entry, card, actions, children }: { project: Workspace; pr: PullRequest; entry: PrEntry | null; card: PrCard | null; actions: Actions; children: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const [confirm, setConfirm] = useState<Confirm>(null);
  const [picked, setPicked] = useState<PrMergeMethod | null>(null);
  const open = pr.state !== "merged" && pr.state !== "closed";
  const kind = entry?.reason.kind ?? "checking";
  const primary: PrimaryAction = primaryAction(kind);
  const mine = pr.work?.mine === true;
  const working = entry?.row.group === "working";
  const method = picked ?? defaultMethod(card);
  const queue = card?.merge?.queue === true;
  const queued = pr.work?.queue.state === "at";
  const why = mergeWhy(pr, card, working);
  const write = card?.write ?? null;
  const writing = write?.phase === "working";
  const head = card?.head ?? null;
  const github = (path = "") => actions.openPullRequest(`${pr.url}${path}`, project.device_id, true);
  const confirming = confirm !== null && confirmHolds(confirm, { head, queue, method, mergeBlocked: why !== null, behind: pr.work?.merge_state === "behind" }) ? confirm : null;
  useEffect(() => {
    if (confirm !== null && confirming === null) setConfirm(null);
  }, [confirm, confirming]);
  const line = <div className="flex min-w-0 flex-wrap items-center gap-sm">{children}</div>;
  if (!open) return line;
  const base = pr.base_branch ?? "";
  const methodLabel = method ? t(`work.card.method.${method}`) : "";
  // A write GitHub refused for the head, the merge state or the permission would only be refused again, so both buttons stay off until Look again has read the card; a transient failure (unreachable, failed) is retried with the same button (B37).
  const repeats = write?.phase === "failed" && write.reason !== "unreachable" && write.reason !== "failed";
  const canAsk = head !== null && !writing && !repeats;
  const MergeButton = (
    <span className="flex min-w-0 items-center gap-xs" data-pr-card-merge-slot="true">
      {why && !(why.key === "ci" && pr.work?.check_counts) ? <span className="min-w-0 whitespace-normal text-micro text-muted-foreground" data-pr-card-merge-why={why.key}>{t(`work.card.why.${why.key}`, { count: why.count, base: why.base })}</span> : null}
      {!queue && (card?.merge?.methods.length ?? 0) > 1 ? (
        <select aria-label={t("work.card.merge")} className="rounded-xs border border-border bg-background px-xs py-xxs text-micro outline-none focus-visible:ring-1 focus-visible:ring-ring" value={method ?? ""} onChange={(event) => setPicked(event.target.value as PrMergeMethod)} data-pr-card-method="true">
          {card!.merge!.methods.map((each) => <option key={each} value={each}>{t(`work.card.method.${each}`)}</option>)}
        </select>
      ) : null}
      <Button variant={primary === "merge" ? "default" : "secondary"} size="sm" disabled={why !== null || queued || !canAsk} onClick={() => head !== null && setConfirm({ action: "merge", head, method, queue })} data-pr-card-merge={queue ? "queue" : "merge"}>
        {queue ? t("work.card.mergeQueue") : t("work.card.merge")}
      </Button>
    </span>
  );
  const UpdateButton = (
    <Button variant={primary === "update" ? "default" : "secondary"} size="sm" disabled={!canAsk || write?.phase === "working"} onClick={() => head !== null && setConfirm({ action: "update", head, method: null, queue })} data-pr-card-update="true">{t("work.card.updateBranch")}</Button>
  );
  return (
    <div className="flex flex-col gap-sm" data-pr-card-buttons={primary}>
      <div className="flex min-w-0 flex-wrap items-center gap-sm">
        {children}
        <span className="flex min-w-0 flex-wrap items-center justify-end gap-xs">
          {primary === "update" || pr.work?.merge_state === "behind" ? UpdateButton : null}
          {primary === "review" ? <><Button size="sm" variant="outline" onClick={() => review(project, pr)} data-pr-card-review-agent="true">{t("work.card.reviewAgent")}</Button><Button size="sm" onClick={() => github("/files")} data-pr-card-review-github="true"><ExternalLinkIcon aria-hidden="true" />{t("work.card.reviewGithub")}</Button></> : null}
          {primary === "assign" ? <Button size="sm" onClick={() => useUiStore.getState().setWorkspaceDialog({ kind: "pr_delegate", workspaceId: project.id, prNumber: pr.number })} data-pr-card-assign="true">{t("work.card.assign")}</Button> : null}
          {primary === "assign_reviewer" ? <Button size="sm" variant="outline" onClick={() => github()} data-pr-card-assign-reviewer="true">{t("work.card.assignReviewer")}</Button> : null}
          {mine && primary !== "review" ? MergeButton : null}
        </span>
      </div>
      {confirming !== null ? (
        <div role="alertdialog" aria-label={confirmText(confirming, queue, t, { number: pr.number, base, method: methodLabel, head: confirming.head.slice(0, 7) })} className="flex flex-col gap-sm rounded-md border border-border bg-background p-sm" data-pr-card-confirm={confirming.action}>
          <p className="text-caption">{confirmText(confirming, queue, t, { number: pr.number, base, method: methodLabel, head: confirming.head.slice(0, 7) })}</p>
          <span className="flex gap-xs">
            <Button size="sm" onClick={() => {
              if (confirming.action === "merge") actions.mergePullRequest(project.id, pr.number, confirming.head, confirming.queue ? null : confirming.method);
              else actions.updatePullRequestBranch(project.id, pr.number, confirming.head);
              setConfirm(null);
            }} data-pr-card-confirm-yes={confirming.action}>{t(confirming.action === "merge" ? (queue ? "work.card.confirm.doQueue" : "work.card.confirm.doMerge") : "work.card.confirm.doUpdate", { number: pr.number })}</Button>
            <Button size="sm" variant="ghost" autoFocus onClick={() => setConfirm(null)}>{t("common.cancel")}</Button>
          </span>
        </div>
      ) : null}
      <WriteLine write={write} base={base} card={card} actions={actions} projectId={project.id} number={pr.number} />
    </div>
  );
}

/** What the last Merge or branch update did, in one line under the buttons (B37). */
function WriteLine({ write, base, card, actions, projectId, number }: { write: PrCard["write"]; base: string; card: PrCard | null; actions: Actions; projectId: string; number: number }) {
  const { t } = useInterfaceTranslation();
  if (!write) return null;
  if (write.phase === "working") return <p role="status" className="text-caption text-muted-foreground" data-pr-card-write="working">{t("work.card.writing")}</p>;
  if (write.phase === "done") return <p role="status" className="text-caption text-pr-mergeable" data-pr-card-write="done">{t(write.action === "merge" ? "work.card.written.merge" : "work.card.written.update")}</p>;
  const reason = write.reason ?? "failed";
  return (
    <p role="alert" className="flex flex-wrap items-center gap-sm text-caption text-warning" data-pr-card-write="failed" data-pr-card-write-reason={reason}>
      <span>{t(`work.card.fail.${reason}`, { base })}</span>
      <Button size="sm" variant="secondary" disabled={card?.reading === true} onClick={() => actions.lookAgainPrCard(projectId, number)} data-pr-card-look-again="true">{t("work.card.lookAgain")}</Button>
    </p>
  );
}

/** `Have an agent review`: the Start dialog's agent opens on the branch with the review instruction. */
function review(project: Workspace, pr: PullRequest) {
  useUiStore.getState().setWorkspaceDialog({ kind: "pr_delegate", workspaceId: project.id, prNumber: pr.number, intent: "review" });
}

// --- timeline ------------------------------------------------------------------

/**
 * 타임라인 (D-03, D-05, D-06, B3-B10; PRD record-card-readability D-01,
 * D-09): the sessions that made or worked on the pull request and its
 * milestones, oldest first, under a small `타임라인 · 세션 N`. The
 * milestones come from the list and the card's own read; the sessions from
 * the link record, whose spinner, failure, fold and empty
 * line keep their rules inside the section.
 */
function Timeline({ project, pr, card, panel, number, actions }: { project: Workspace; pr: PullRequest | null; card: PrCard | null; panel: LinkPanel | null; number: number; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const holder: LinkPanel = panel ?? { workspace_id: project.id, target: { kind: "pr", number }, loading: true, failure: null, pr: null, prs: [], sessions: [], total: 0 };
  const rows = useMemo(() => (pr ? milestones(pr, card) : []), [pr, card]);
  return (
    <section className="flex flex-col gap-xs" aria-label={t("work.card.timeline")} data-pr-card-timeline="true">
      <h3 className="flex min-h-(--size-control-sm) items-baseline gap-xs text-micro font-medium text-muted-foreground">
        <span>{t("work.card.timeline")}</span>
        {holder.loading || !holder.total ? null : <span className="font-normal" data-pr-card-sessions={holder.total}>· {t("links.sessionsCount", { count: holder.total })}</span>}
      </h3>
      <LinkTimeline panel={holder} project={project} pr={pr} milestones={rows} branchOf={() => pr?.head_branch ?? null} onRetry={() => actions.openLinks(project.id, { kind: "pr", number })} actions={actions} />
    </section>
  );
}

// --- folded files --------------------------------------------------------------------

/** A card part that has not been read, or could not be (B37). */
function PartState({ part, card, actions, projectId, number }: { part: "body" | "files" | "threads"; card: PrCard | null; actions: Actions; projectId: string; number: number }) {
  const { t } = useInterfaceTranslation();
  // The last read's failure stays drawn while a new read runs, so a re-read moves nothing under the pointer.
  const failed = card?.failed.includes(part) === true;
  if (failed) {
    return (
      <p className="flex items-center gap-sm text-caption text-muted-foreground" data-pr-card-part={part} data-pr-card-part-state="failed">
        {t("work.card.part.failed")}
        <Button variant="secondary" size="sm" disabled={card?.reading === true} onClick={() => actions.readPrCard(projectId, number)}>{t("common.retry")}</Button>
      </p>
    );
  }
  return <span className="block h-(--size-control) rounded-sm bg-muted" role="status" aria-label={t("work.card.part.pending")} data-pr-card-part={part} data-pr-card-part-state="pending" />;
}

/**
 * 바뀐 파일 N (B11, B36, B49): one folded line between the timeline and the
 * body, +/- at the right; opened, the files with generated ones folded into
 * one line. A file opens as D-27 says: the diff in this checkout's Files,
 * where the lane turns while the 보기 slot stays on the rail, or GitHub's
 * files page.
 */
function FilesRow({ pr, project, card, actions, open, onToggle }: { pr: PullRequest; project: Workspace; card: PrCard | null; actions: Actions; open: boolean; onToggle: () => void }) {
  const { t } = useInterfaceTranslation();
  const [generatedOpen, setGeneratedOpen] = useState(false);
  const front = useShellStore((state) => frontCheckout(state.rest));
  const committed = useShellStore((state) => state.changes?.committed ?? null);
  const files = card?.files ?? null;
  const split = useMemo(() => splitFiles(files), [files]);
  const added = files?.reduce((sum, file) => sum + file.additions, 0) ?? 0;
  const removed = files?.reduce((sum, file) => sum + file.deletions, 0) ?? 0;
  const openFile = (file: string) => {
    const target = fileOpen({ pr, front: front ? { branch: front.branch ?? null, path: front.path } : null, committed, file });
    if (target.kind === "diff") actions.selectChange(target.path, true, false);
    else actions.openPullRequest(target.url, project.device_id, true);
  };
  const name = files === null ? t("work.card.changedFiles") : t("work.card.filesCount", { count: card?.files_total ?? files.length });
  const summary = files === null ? (card?.failed.includes("files") ? <span className="text-warning" data-pr-card-part-summary="files">{t("work.card.part.failed")}</span> : null) : <><span className="font-mono text-pr-mergeable">+{added}</span><span className="font-mono text-pr-failed">−{removed}</span></>;
  return (
    <section className="flex flex-col border-t border-border" aria-label={name} data-pr-card-row="files" data-open={open ? "true" : undefined}>
      <button type="button" aria-expanded={open} onClick={onToggle} className="flex min-h-(--size-control) w-full min-w-0 items-center gap-xs rounded-xs py-xs text-left text-caption outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" data-pr-card-toggle="files">
        {open ? <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" /> : <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />}
        <span className="shrink-0 font-medium text-foreground">{name}</span>
        <span className="flex min-w-0 flex-1 items-center justify-end gap-xs text-micro text-muted-foreground">{summary}</span>
      </button>
      {open ? (
        <div className="flex flex-col gap-sm pb-sm pl-md">
          {files === null ? <PartState part="files" card={card} actions={actions} projectId={project.id} number={pr.number} /> : (
            <ul className="flex flex-col" role="list" data-pr-card-files-section="true">
              {split.shown.map((file) => <FileRow key={file.path} file={file} onOpen={() => openFile(file.path)} />)}
              {split.generated > 0 ? (
                <li>
                  <button type="button" aria-expanded={generatedOpen} onClick={() => setGeneratedOpen(!generatedOpen)} className="flex w-full items-center gap-xs rounded-xs py-xs text-left text-caption text-muted-foreground outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" data-pr-card-generated={split.generated}>
                    {generatedOpen ? <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
                    {t("work.card.generated", { count: split.generated })}
                  </button>
                </li>
              ) : null}
              {generatedOpen ? files.filter((file) => file.generated).map((file) => <FileRow key={file.path} file={file} onOpen={() => openFile(file.path)} />) : null}
              {card && card.files_total > files.length ? <li className="py-xs text-micro text-muted-foreground">{t("work.card.filesMore", { shown: files.length, total: card.files_total })}</li> : null}
            </ul>
          )}
        </div>
      ) : null}
    </section>
  );
}

function FileRow({ file, onOpen }: { file: NonNullable<PrCard["files"]>[number]; onOpen: () => void }) {
  return (
    <li>
      <button type="button" onClick={onOpen} className="flex w-full min-w-0 items-center gap-sm rounded-xs px-xs py-xs text-left text-caption outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" data-pr-card-file={file.path} title={file.path}>
        <span className="min-w-0 flex-1 truncate font-mono">{file.path}</span>
        <span className="shrink-0 font-mono text-micro text-pr-mergeable">+{file.additions}</span>
        <span className="shrink-0 font-mono text-micro text-pr-failed">−{file.deletions}</span>
      </button>
    </li>
  );
}

// --- body ----------------------------------------------------------------------------

/**
 * 본문 (D-02, B12, B13): never folded. Under the section head a row of the
 * body's `##` headings sticks to the top of the card while the body scrolls
 * under it; a chip scrolls to its heading, and the chip of the heading last
 * passed is pressed. A body with no heading has no row, an empty body reads
 * `본문 없음`, and a body that could not be read says so with a retry for
 * that part alone.
 */
function Body({ card, actions, projectId, number, scroller }: { card: PrCard | null; actions: Actions; projectId: string; number: number; scroller: React.RefObject<HTMLElement | null> }) {
  const { t } = useInterfaceTranslation();
  const body = card?.body ?? null;
  const content = useRef<HTMLDivElement>(null);
  const [current, setCurrent] = useState(0);
  // The `##` headings the renderer drew, in the body's order: the chips are read
  // from them, so a `##` the renderer did not draw, inside an HTML comment, gets
  // no chip, and a heading with no text (an image alone) gets none either (B12).
  const headings = () => Array.from(content.current?.querySelectorAll<HTMLElement>("[data-markdown-heading='2']") ?? []);
  const [outline, setOutline] = useState<{ at: number; text: string }[]>([]);
  useLayoutEffect(() => {
    setOutline(headings().flatMap((heading, at) => { const text = heading.textContent?.trim() ?? ""; return text ? [{ at, text }] : []; }));
  }, [body]);
  useLayoutEffect(() => {
    const root = scroller.current;
    if (!root || outline.length === 0) return;
    const chips = root.querySelector<HTMLElement>("[data-pr-card-outline]");
    const read = () => {
      const edge = (chips?.getBoundingClientRect().bottom ?? root.getBoundingClientRect().top) + 1;
      const drawn = headings();
      let passed = 0;
      outline.forEach((chip, index) => {
        if ((drawn[chip.at]?.getBoundingClientRect().top ?? Infinity) <= edge) passed = index;
      });
      setCurrent(passed);
    };
    read();
    root.addEventListener("scroll", read, { passive: true });
    return () => root.removeEventListener("scroll", read);
  }, [outline, scroller]);
  const jump = (index: number) => {
    const chip = outline[index];
    const heading = chip ? headings()[chip.at] : undefined;
    const root = scroller.current;
    if (!heading || !root) return;
    const chips = root.querySelector<HTMLElement>("[data-pr-card-outline]");
    const offset = heading.getBoundingClientRect().top - root.getBoundingClientRect().top - (chips?.getBoundingClientRect().height ?? 0);
    root.scrollTo({ top: root.scrollTop + offset });
    setCurrent(index);
  };
  return (
    <section className="flex flex-col border-t border-border" aria-label={t("work.card.body")} data-pr-card-row="body">
      <h3 className="flex min-h-(--size-control) items-center text-caption font-medium text-foreground">{t("work.card.body")}</h3>
      {outline.length > 0 ? (
        <nav aria-label={t("work.card.outline")} className="sticky top-none z-10 flex flex-wrap gap-xxs bg-background py-xs" data-pr-card-outline="true">
          {outline.map((chip, index) => (
            <button key={`${chip.at}:${chip.text}`} type="button" aria-pressed={index === current} className={cn("rounded-full border border-border px-sm text-micro outline-none focus-visible:ring-1 focus-visible:ring-ring", index === current ? "bg-secondary text-foreground" : "text-muted-foreground hover:text-foreground")} onClick={() => jump(index)} data-pr-card-outline-chip={index}>
              {chip.text}
            </button>
          ))}
        </nav>
      ) : null}
      <div ref={content} className="flex flex-col gap-sm pb-md text-caption" data-pr-card-body={body === null ? "unread" : body.trim() === "" ? "empty" : "open"}>
        {body === null ? <PartState part="body" card={card} actions={actions} projectId={projectId} number={number} /> : body.trim() === "" ? <p className="text-caption text-muted-foreground">{t("work.card.noBody")}</p> : <div className="flex flex-col gap-sm" data-pr-card-markdown="true">{renderMarkdown(body, { html: true })}</div>}
      </div>
    </section>
  );
}
