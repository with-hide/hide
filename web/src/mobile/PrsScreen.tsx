// The phone's PR tab (PRD mobile-companion-ui B28, B37, B38, B45, D-12, doop
// final board F9): the open pull requests of the registered projects in the
// four groups of `prModel.ts`, each row with its state mark, number, title and
// the one line the desktop list says for it (the core's reason and the
// approvals). While GitHub cannot be read the last list stays, dimmed, under
// one line; while the Mac cannot be reached it stays under the unreachable
// line. What the groups, the line and the reading are is `prModel.ts` and the
// desktop's own rules; this draws them.

import { ChevronRightIcon, Loader2Icon } from "lucide-react";
import { useElapsed } from "../components/elapsed";
import { RowButton } from "../components/ui/row-button";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { PR_LOOK } from "../prMark";
import { GROUP_TONE, reasonText, splitReason } from "../prReasonText";
import { approvalCount, prReview } from "../prReview";
import { OfflineText } from "./OfflineText";
import { DeviceTag } from "./parts";
import { openCount, groupPrs, projectTag, prsReading, type PrEntry, type PrGroup } from "./prModel";
import { openPrPage } from "./connection";
import { TabBar } from "./TabBar";
import { usePhone } from "./store";

export function PrsScreen() {
  const { t } = useInterfaceTranslation();
  const prs = usePhone((s) => s.prs);
  const connected = usePhone((s) => s.connected);
  const unreachable = usePhone((s) => s.unreachable);
  // The last list stays, dimmed, under the unreachable line (B45).
  const offline = unreachable && !connected;
  const groups = prs ? groupPrs(prs.items, prs.projects) : [];
  const reading = prs ? prsReading(prs.projects, Date.now()) : null;
  const count = openCount(groups);
  const blocked = reading?.state === "blocked" ? reading.category : null;
  // A failed read keeps the last list; the line says it is read again (B38).
  const stale = reading?.state === "stale";
  const dim = offline || stale;
  const names = prs && prs.projects.length === 1 ? projectTag(prs.projects[0]!) : null;
  return (
    <main className="flex min-h-full flex-col" data-phone-prs="true">
      <header className="phone-safe-top px-xl pb-sm">
        <h1 className="pt-lg text-headline font-semibold text-foreground">{t("mobile.prs.title")}</h1>
        {prs && count > 0 ? (
          <p className="mt-xs break-keep text-subhead text-muted-foreground wrap-anywhere" data-phone-prs-open={count}>
            {[names, t("mobile.prs.count", { count })].filter(Boolean).join(" · ")}
          </p>
        ) : null}
      </header>
      {offline ? (
        <div role="status" className="flex items-center gap-sm px-xl py-sm text-body text-subtle-foreground" data-phone-unreachable="true">
          <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 animate-spin" />
          <span><OfflineText /></span>
        </div>
      ) : stale ? (
        <p role="status" className="px-xl py-sm text-body text-subtle-foreground" data-phone-prs-state="stale">{t("mobile.prs.readFailed")}</p>
      ) : null}
      <div className={cn("flex-1", dim && "opacity-(--opacity-dimmed)")} aria-busy={dim}>
        {blocked ? (
          <p className="px-xl py-lg text-body text-muted-foreground" data-phone-prs-state="blocked" data-phone-prs-gh={blocked}>{t(blocked === "not_installed" ? "issueSettings.ghNotInstalled" : "issueSettings.ghNotLoggedIn")}</p>
        ) : prs === null || (count === 0 && reading?.state === "reading") ? (
          <p role="status" className="px-xl py-lg text-body text-muted-foreground" data-phone-prs-state="reading">{t("work.reading")}</p>
        ) : count === 0 ? (
          <p className="px-xl py-xxl text-center text-title text-muted-foreground" data-phone-prs-state="empty">{t("mobile.prs.empty")}</p>
        ) : (
          groups.map(({ group, entries }) => <Group key={group} group={group} entries={entries} showProject={prs.projects.length > 1} />)
        )}
        {prs && prs.omitted > 0 ? (
          <p className="px-xl py-md text-body text-muted-foreground" data-phone-prs-omitted={prs.omitted}>{t("mobile.prs.omitted", { count: prs.omitted })}</p>
        ) : null}
      </div>
      <TabBar active="prs" />
    </main>
  );
}

/** One group (B37): its name and count, then its rows in the order the core listed them. */
function Group({ group, entries, showProject }: { group: PrGroup; entries: PrEntry[]; showProject: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <section className="px-xl pt-lg" aria-label={`${t(`mobile.prs.group.${group}`)} ${entries.length}`} data-phone-pr-group={group} data-phone-pr-group-count={entries.length}>
      <h2 className="flex items-center gap-sm pb-xs text-body text-muted-foreground">
        <span>{t(`mobile.prs.group.${group}`)}</span>
        <span className="font-mono font-normal">{entries.length}</span>
      </h2>
      <ul className="divide-y divide-border border-b border-border">
        {entries.map((entry) => (
          <li key={entry.pr.key}>
            <Row entry={entry} showProject={showProject} />
          </li>
        ))}
      </ul>
    </section>
  );
}

/** A pull request's row: its mark and number, its title, then the core's reason and the approvals; a project away from the Mac names its device (B37). */
function Row({ entry, showProject }: { entry: PrEntry; showProject: boolean }) {
  const { t } = useInterfaceTranslation();
  const { pr, project } = entry;
  const { icon: Icon, tone } = PR_LOOK[pr.state];
  const elapsed = useElapsed(pr.reason.since_unix_ms);
  const [lead, tail] = splitReason(reasonText(t, pr.reason, elapsed));
  const review = prReview(pr);
  const approvals = review ? approvalCount(review) : null;
  const toneClass = pr.reason.kind === "checking" || pr.reason.kind === "unjudged" ? "text-muted-foreground" : GROUP_TONE[pr.group];
  return (
    <RowButton
      aria-label={t("work.row.open", { number: String(pr.number), title: pr.title })}
      data-phone-pr={pr.key}
      data-phone-pr-reason={pr.reason.kind}
      className="min-h-(--size-touch-target) flex-col items-stretch gap-xs rounded-none px-none py-md hover:bg-transparent active:bg-accent"
      onClick={() => openPrPage(pr.key)}
    >
      <span className="flex w-full min-w-0 items-center gap-sm">
        <Icon aria-hidden="true" className={cn("size-(--size-pr-icon) shrink-0", tone)} />
        <span className="shrink-0 font-mono text-body text-subtle-foreground">#{pr.number}</span>
        <span className="min-w-0 flex-1 truncate text-title font-medium text-foreground">{pr.title}</span>
        <ChevronRightIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
      </span>
      <span className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs pl-[calc(var(--size-pr-icon)+var(--spacing-sm))] text-body">
        <span className="min-w-0 break-keep wrap-anywhere" data-phone-pr-line="true">
          {lead ? <span className="font-medium text-foreground">{lead}</span> : null}
          <span className={toneClass}>{tail}</span>
          {approvals ? <span className={approvals.approved ? "text-pr-mergeable" : "text-pr-pending"}> · {t("work.review.count", approvals)}</span> : null}
        </span>
        {showProject ? <span className="min-w-0 truncate text-muted-foreground">{project.label}</span> : null}
        {project.device_label ? <DeviceTag label={project.device_label} /> : null}
      </span>
    </RowButton>
  );
}
