// One pull request's page on the phone (PRD mobile-companion-ui B39-B45, D-12,
// D-13, doop final board F10): its state, title and branch, the agent that
// holds it, the checks, the review, the changed files with their +/- lines, a
// link to GitHub, and the two buttons: `머지`, which asks once and goes out on
// the head the page showed, and `에이전트에게 맡기기`, which opens the hand-over
// sheet. Why Merge is off, what a confirmation holds and which method it uses
// are the desktop card's own rules (`prCardModel.ts`, through `prModel.ts`);
// this draws them. The "머지 순서" line of the board is left out on purpose
// (D-12): there is no merge order on the phone.

import { ArrowLeftIcon, ChevronRightIcon, ExternalLinkIcon, GitMergeIcon, Loader2Icon, UserRoundCheckIcon } from "lucide-react";
import { StatusMark } from "../components/status-mark";
import { Button } from "../components/ui/button";
import { RowButton } from "../components/ui/row-button";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "../components/ui/sheet";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { checkSummary, splitFiles } from "../prCardModel";
import { PR_LOOK } from "../prMark";
import { approvalCount, prReview } from "../prReview";
import { agentProductName } from "../agentAdapters";
import { askMerge, cancelMerge, confirmMerge, lookAgainPr, openDelegate, openDetail } from "./connection";
import { OfflineText } from "./OfflineText";
import { PrDelegateSheet } from "./PrDelegateSheet";
import { prActions, prGroupOf, prOf, settled } from "./prModel";
import { noticeText, type AgentKey, type PhonePr } from "./protocol";
import { agentOf, closePr, usePhone } from "./store";

export function PrDetail() {
  const pr = usePhone((s) => prOf(s.prs?.items ?? null, s.prOpen));
  // A pull request that left the list while its page was open is gone with it; the store closes the page on the next frame.
  if (!pr) return null;
  return <Page pr={pr} />;
}

function Page({ pr }: { pr: PhonePr }) {
  const { t } = useInterfaceTranslation();
  const connected = usePhone((s) => s.connected);
  const unreachable = usePhone((s) => s.unreachable);
  const projects = usePhone((s) => s.prs?.projects ?? []);
  const dim = unreachable && !connected;
  const project = projects.find((candidate) => candidate.id === pr.project) ?? null;
  const base = pr.base_branch ?? "";
  return (
    <main className="flex h-full flex-col" data-phone-pr-page={pr.key}>
      <header className="phone-safe-top shrink-0 border-b border-border bg-background px-lg pb-md">
        <div className="flex items-center justify-between gap-md pt-md">
          <Button variant="text" size="inline" className="min-h-(--size-touch-target) gap-xs text-body" data-phone-back="prs" onClick={closePr}>
            <ArrowLeftIcon aria-hidden="true" className="size-(--size-icon)" />
            {t("mobile.prs.title")}
          </Button>
          <a
            href={pr.url}
            target="_blank"
            rel="noopener noreferrer"
            aria-label={t("work.card.github")}
            data-phone-pr-github="true"
            className="inline-flex min-h-(--size-touch-target) items-center gap-xs text-body text-muted-foreground"
          >
            GitHub
            <ExternalLinkIcon aria-hidden="true" className="size-(--size-icon-sm)" />
          </a>
        </div>
        <StatePill pr={pr} />
        <h1 className="mt-sm break-keep text-headline font-semibold text-foreground wrap-anywhere" data-phone-pr-title="true">
          <span className="font-mono text-subtle-foreground">#{pr.number}</span> {pr.title}
        </h1>
        <p className="mt-xs flex min-w-0 flex-wrap items-center gap-xs break-all font-mono text-body text-muted-foreground" data-phone-pr-branch="true">
          <span>{pr.head_branch ?? ""}</span>
          {base ? <><span aria-hidden="true">→</span><span>{base}</span></> : null}
        </p>
        {project && (project.device_label || projects.length > 1) ? (
          <p className="mt-xs text-body text-muted-foreground">{project.device_label ? `${project.device_label} · ${project.label}` : project.label}</p>
        ) : null}
      </header>
      {dim ? (
        <div role="status" className="flex items-center gap-sm px-lg py-sm text-body text-subtle-foreground" data-phone-unreachable="true">
          <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 animate-spin" />
          <span><OfflineText /></span>
        </div>
      ) : null}
      <div className={cn("min-h-0 flex-1 overflow-auto overscroll-contain px-lg pb-lg", dim && "opacity-(--opacity-dimmed)")} aria-busy={dim}>
        <Agents pr={pr} />
        <Checks pr={pr} />
        <Review pr={pr} />
        <Files pr={pr} />
      </div>
      <Buttons pr={pr} connected={connected} />
      <MergeSheet />
      <PrDelegateSheet />
    </main>
  );
}

/** The pull request's state under the back row: what it can do now for an open one (the phone's group), its own word once settled. */
function StatePill({ pr }: { pr: PhonePr }) {
  const { t } = useInterfaceTranslation();
  const open = !settled(pr);
  const group = open ? prGroupOf(pr.reason.kind) : null;
  const { icon: Icon, tone } = PR_LOOK[pr.state];
  return (
    <span className="mt-sm inline-flex items-center gap-xs rounded-full bg-secondary px-md py-xxs text-caption font-semibold" data-phone-pr-pill={pr.state}>
      <Icon aria-hidden="true" className={cn("size-(--size-icon-sm)", tone)} />
      <span className={tone}>{group ? t(`mobile.prs.group.${group}`) : t(PR_LOOK[pr.state].word)}</span>
    </span>
  );
}

/** The agent that holds the pull request, opening to its detail (B39). */
function Agents({ pr }: { pr: PhonePr }) {
  const { t } = useInterfaceTranslation();
  const groups = usePhone((s) => s.groups);
  const rows = pr.agents.flatMap((key) => {
    const agent = agentOf(groups, key);
    return agent ? [agent] : [];
  });
  if (rows.length === 0) return null;
  return (
    <section className="pt-lg" data-phone-pr-agents={rows.length}>
      <h2 className="pb-xs text-body text-muted-foreground">{t("mobile.prs.agents")}</h2>
      <ul className="divide-y divide-border border-b border-border">
        {rows.map((agent) => (
          <li key={`${agent.device_id}|${agent.pane_id}`}>
            <AgentLine agentKey={{ device_id: agent.device_id, pane_id: agent.pane_id }} mark={agent.mark} title={agent.title} kind={agent.agent_kind} read={agent.read === true} />
          </li>
        ))}
      </ul>
    </section>
  );
}

function AgentLine({ agentKey, mark, title, kind, read }: { agentKey: AgentKey; mark: Parameters<typeof StatusMark>[0]["mark"]; title: string; kind: string; read: boolean }) {
  return (
    <RowButton
      aria-label={`${title}, ${agentProductName(kind)}`}
      data-phone-pr-agent={`${agentKey.device_id}|${agentKey.pane_id}`}
      className="min-h-(--size-touch-target) gap-sm rounded-none px-none py-md hover:bg-transparent active:bg-accent"
      onClick={() => openDetail(agentKey)}
    >
      {read ? <StatusMark mark="turn" read /> : <StatusMark mark={mark} />}
      <span className="min-w-0 flex-1 truncate text-title font-medium text-foreground">{title}</span>
      <ChevronRightIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
    </RowButton>
  );
}

/** The checks: the summary the desktop card says, and each failed one by name (B39). */
function Checks({ pr }: { pr: PhonePr }) {
  const { t } = useInterfaceTranslation();
  if (settled(pr)) return null;
  const summary = checkSummary(pr);
  const failed = pr.work?.failed_checks ?? [];
  return (
    <section className="pt-lg" data-phone-pr-checks={pr.checks ?? "unknown"}>
      <h2 className="flex items-baseline justify-between gap-md pb-xs text-body text-muted-foreground">
        <span>{t("mobile.prs.checks")}</span>
        <span className="text-subtle-foreground">{t(summary.key, summary.values)}</span>
      </h2>
      {failed.length > 0 ? (
        <ul className="divide-y divide-border border-b border-border">
          {failed.map((check, index) => (
            <li key={`${check.name ?? ""}:${index}`} className="flex min-h-(--size-touch-target) items-center gap-sm py-sm text-title text-pr-failed" data-phone-pr-failed-check={check.name ?? ""}>
              <span className="min-w-0 break-all">{check.name ?? t("githubReason.unnamedCheck")}</span>
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}

/** The review: the approvals out of what the base requires when GitHub said so, and each reviewer's state (B39). */
function Review({ pr }: { pr: PhonePr }) {
  const { t } = useInterfaceTranslation();
  const review = prReview(pr);
  if (!review) return null;
  const count = approvalCount(review);
  return (
    <section className="pt-lg" data-phone-pr-review={review.decision ?? "none"}>
      <h2 className="flex items-baseline justify-between gap-md pb-xs text-body text-muted-foreground">
        <span>{t("work.review.title")}</span>
        {count ? <span className={cn("font-mono", count.approved ? "text-pr-mergeable" : "text-pr-pending")}>{t("work.review.count", count)}</span> : null}
      </h2>
      {review.reviewers.length > 0 ? (
        <ul className="divide-y divide-border border-b border-border">
          {review.reviewers.map((reviewer) => (
            <li key={`${reviewer.team}:${reviewer.login}`} className="flex min-h-(--size-touch-target) items-center gap-sm py-sm text-title" data-phone-pr-reviewer={reviewer.login} data-phone-pr-reviewer-state={reviewer.state}>
              <UserRoundCheckIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
              <span className="min-w-0 flex-1 truncate text-foreground">{reviewer.team ? t("work.review.team", { name: reviewer.login }) : reviewer.login}</span>
              <span className="shrink-0 text-body text-subtle-foreground">{t(`work.review.state.${reviewer.state}`)}</span>
            </li>
          ))}
        </ul>
      ) : review.nobodyAsked ? (
        <p className="py-sm text-body text-subtle-foreground">{t("work.review.nobodyAsked")}</p>
      ) : null}
    </section>
  );
}

/** The changed files with their +/- lines, generated ones folded into a line; what the card read in part says so (B39). */
function Files({ pr }: { pr: PhonePr }) {
  const { t } = useInterfaceTranslation();
  const card = pr.card;
  if (!card) return null;
  if (card.files === null) {
    return card.reading || card.failed.includes("files") ? (
      <section className="pt-lg" data-phone-pr-files="pending">
        <h2 className="pb-xs text-body text-muted-foreground">{t("work.card.changedFiles")}</h2>
        <p className="py-sm text-body text-subtle-foreground">{t(card.reading ? "work.card.part.pending" : "work.card.part.failed")}</p>
      </section>
    ) : null;
  }
  const { shown, generated } = splitFiles(card.files);
  // The total of +/- is told only when every file is here; a card cut at its cap says how many are left.
  const complete = card.files.length === card.files_total;
  const additions = card.files.reduce((sum, file) => sum + file.additions, 0);
  const deletions = card.files.reduce((sum, file) => sum + file.deletions, 0);
  return (
    <section className="pt-lg" data-phone-pr-files={card.files_total}>
      <h2 className="flex items-baseline justify-between gap-md pb-xs text-body text-muted-foreground">
        <span>{t("work.card.filesCount", { count: card.files_total })}</span>
        {complete ? <Lines additions={additions} deletions={deletions} /> : null}
      </h2>
      <ul className="divide-y divide-border border-b border-border">
        {shown.map((file) => (
          <li key={file.path} className="flex min-h-(--size-touch-target) items-center justify-between gap-md py-sm text-body" data-phone-pr-file={file.path}>
            <span className="min-w-0 break-all font-mono text-foreground">{file.path}</span>
            <Lines additions={file.additions} deletions={file.deletions} />
          </li>
        ))}
      </ul>
      {generated > 0 ? <p className="pt-sm text-body text-subtle-foreground">{t("work.card.generated", { count: generated })}</p> : null}
      {!complete ? <p className="pt-sm text-body text-subtle-foreground" data-phone-pr-files-more="true">{t("work.card.filesMore", { total: card.files_total, shown: card.files.length })}</p> : null}
    </section>
  );
}

function Lines({ additions, deletions }: { additions: number; deletions: number }) {
  return (
    <span className="flex shrink-0 gap-sm font-mono text-body" data-phone-pr-lines={`+${additions} -${deletions}`}>
      <span className="text-pr-mergeable">+{additions}</span>
      <span className="text-pr-failed">−{deletions}</span>
    </span>
  );
}

/**
 * Merge and hand-over (B40-B44): Merge is off with its reason beside it while
 * the desktop card would not allow it, and while the phone cannot reach the
 * Mac; what the last merge did stands under the buttons.
 */
function Buttons({ pr, connected }: { pr: PhonePr; connected: boolean }) {
  const { t } = useInterfaceTranslation();
  const pending = usePhone((s) => s.prPending?.key === pr.key);
  const notice = usePhone((s) => (s.prNotice?.key === pr.key ? s.prNotice.notice : null));
  const delegated = usePhone((s) => s.prDelegated === pr.key);
  const view = prActions(pr);
  if (!view.open) return null;
  const why = view.why;
  const canAsk = connected && view.head !== null && !view.working && !view.repeats && !pending;
  const showWhy = why !== null && !(why.key === "ci" && pr.work?.check_counts);
  return (
    <footer className="phone-safe-bottom shrink-0 border-t border-border bg-background px-lg pt-md pb-md">
      <div className="flex flex-col gap-sm">
        {showWhy ? (
          <p className="text-body text-muted-foreground" data-phone-pr-merge-why={why.key}>{t(`work.card.why.${why.key}`, { count: why.count, base: why.base })}</p>
        ) : null}
        <WriteLine pr={pr} connected={connected} />
        {notice ? <p role="alert" className="text-body text-warning" data-phone-pr-notice="true">{noticeText(t, notice)}</p> : null}
        {delegated ? <p role="status" className="text-body text-pr-mergeable" data-phone-pr-delegated="true">{t("mobile.prs.delegated")}</p> : null}
        <div className="flex gap-sm">
          {view.mergeShown ? (
            <Button
              disabled={!canAsk || why !== null}
              onClick={askMerge}
              className="h-(--size-touch-target) flex-1 rounded-lg text-title font-semibold"
              data-phone-pr-merge={view.queue ? "queue" : "merge"}
            >
              {pending || view.working ? <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) animate-spin" /> : <GitMergeIcon aria-hidden="true" className="size-(--size-icon-lg)" />}
              {view.queue ? t("work.card.mergeQueue") : t("work.card.merge")}
            </Button>
          ) : null}
          <Button
            variant="secondary"
            disabled={!connected}
            onClick={() => openDelegate(pr.key)}
            className="h-(--size-touch-target) flex-1 rounded-lg text-title"
            data-phone-pr-delegate="true"
          >
            {t("work.card.assign")}
          </Button>
        </div>
      </div>
    </footer>
  );
}

/** What the last merge did, in one line under the reason, as the desktop card says it (B42, B43). */
function WriteLine({ pr, connected }: { pr: PhonePr; connected: boolean }) {
  const { t } = useInterfaceTranslation();
  const write = pr.card?.write ?? null;
  if (!write) return null;
  if (write.phase === "working") return <p role="status" className="text-body text-muted-foreground" data-phone-pr-write="working">{t("work.card.writing")}</p>;
  if (write.phase === "done") {
    const queued = write.action === "merge" && write.method === null && pr.card?.merge?.queue === true;
    return <p role="status" className="text-body text-pr-mergeable" data-phone-pr-write="done">{t(queued ? "mobile.prs.queued" : write.action === "merge" ? "work.card.written.merge" : "work.card.written.update")}</p>;
  }
  const reason = write.reason ?? "failed";
  return (
    <p role="alert" className="flex flex-wrap items-center gap-sm text-body text-warning" data-phone-pr-write="failed" data-phone-pr-write-reason={reason}>
      <span className="min-w-0 break-keep wrap-anywhere">{t(`work.card.fail.${reason}`, { base: pr.base_branch ?? "" })}</span>
      <Button size="sm" variant="secondary" className="min-h-(--size-touch-target)" disabled={!connected || pr.card?.reading === true} onClick={lookAgainPr} data-phone-pr-look-again="true">
        {t("work.card.lookAgain")}
      </Button>
    </p>
  );
}

/**
 * The one confirmation (B41): which pull request, into which branch, by which
 * method and for which head; nothing in it is preselected and neither button
 * has the keyboard, so nothing goes out on a stray key.
 */
function MergeSheet() {
  const { t } = useInterfaceTranslation();
  const confirm = usePhone((s) => s.prConfirm);
  const pr = usePhone((s) => (s.prConfirm ? prOf(s.prs?.items ?? null, s.prConfirm.key) : null));
  const connected = usePhone((s) => s.connected);
  const open = confirm !== null && pr !== null;
  const base = pr?.base_branch ?? "";
  const number = pr?.number ?? 0;
  const detail = confirm ? [confirm.queue || !confirm.method ? null : t(`work.card.method.${confirm.method}`), confirm.head.slice(0, 7)].filter(Boolean).join(" · ") : "";
  return (
    <Sheet open={open} onOpenChange={(next) => !next && cancelMerge()}>
      <SheetContent
        side="bottom"
        className="phone-safe-bottom gap-md rounded-t-xl px-lg pt-lg pb-lg"
        data-phone-pr-confirm={confirm?.queue ? "queue" : "merge"}
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <SheetTitle className="break-keep text-headline wrap-anywhere">{t(confirm?.queue ? "mobile.prs.confirm.queue" : "mobile.prs.confirm.merge", { number, base })}</SheetTitle>
        <SheetDescription className="font-mono text-body text-muted-foreground">{detail}</SheetDescription>
        <div className="flex flex-col gap-sm">
          <Button disabled={!connected} onClick={confirmMerge} className="h-(--size-touch-target) w-full rounded-lg text-title font-semibold" data-phone-pr-confirm-yes="true">
            {t(confirm?.queue ? "work.card.confirm.doQueue" : "work.card.confirm.doMerge", { number })}
          </Button>
          <Button variant="secondary" onClick={cancelMerge} className="h-(--size-touch-target) w-full rounded-lg text-title" data-phone-pr-confirm-no="true">
            {t("common.cancel")}
          </Button>
        </div>
      </SheetContent>
    </Sheet>
  );
}
