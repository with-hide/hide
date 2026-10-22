import { ArrowLeftIcon, CoinsIcon, ExternalLinkIcon, FileTextIcon, FolderGit2Icon, GitPullRequestIcon, ListChecksIcon, WaypointsIcon, XIcon } from "lucide-react";
import { cn } from "./lib/utils";
import { PR_LOOK } from "./prMark";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { MissionMap } from "./MissionMap";
import { PlanMap } from "./PlanMap";
import { ProjectCostView, WorktreesView } from "./ProjectDetail";
import { missionOf } from "./missions";
import { PrCardView } from "./PrCard";
import { SlotSessionRecord } from "./SessionRecord";
import { recordTitle, workKey } from "./sessionWork";
import { summariesEnabled } from "./hideAi";
import { allAgents } from "./paneAgents";
import { catalogWorkspaces, type DetailItem, type DetailSlot, type WorkSession } from "./snapshot";
import { useShellStore } from "./store";

// The lane's 보기 slot (PRD sidebar-task-lens D-50, B46-B49): one pull
// request's card, one session's record, a mission's map (PRD
// software-factory-conductor D-14), a session's plan (PRD hide-plan B11),
// or a project's worktrees or cost (PRD lane-two-columns B30, B31), a
// window's and no checkout's, so
// it stays when the Agent tab or the checkout changes. The core keeps what
// it holds and its history; this draws the item in front under the slot's
// head of ←, its name, GitHub for a card, and ✕.

/** The item the slot shows, or null when it holds none. */
export function detailTop(detail: DetailSlot | null | undefined): DetailItem | null {
  return detail?.items.at(-1) ?? null;
}

/** What a record is called in the head: its agent's name while the agent is listed, else the record's own title once read. */
function sessionTitle(s: ReturnType<typeof useShellStore.getState>, session: WorkSession): string | null {
  const listed = allAgents(s.agents, s.rest?.status?.remote).find((row) => row.session_id === session.session_id)?.identity_label;
  if (listed) return listed;
  const read = s.sessionRecord;
  return read?.session && read.record && workKey(read.session) === workKey(session) ? recordTitle(read.record, summariesEnabled(s.rest?.status?.background_ai)) : null;
}

function useItemTitle(item: DetailItem | null): string {
  const { t } = useInterfaceTranslation();
  return useShellStore((s) => {
    if (!item) return "";
    if (item.kind === "pull_request") {
      const project = catalogWorkspaces(s.rest).find((row) => row.id === item.workspace_id);
      return t("detail.pr", { number: item.number, project: project?.label ?? "" });
    }
    if (item.kind === "mission") return t("detail.mission", { title: missionOf(s.rest, item.mission)?.title ?? item.mission });
    if (item.kind === "worktrees" || item.kind === "cost") {
      const project = catalogWorkspaces(s.rest).find((row) => row.id === item.workspace_id)?.label ?? "";
      return t(item.kind === "worktrees" ? "projectTool.worktrees.title" : "projectTool.cost.title", { project });
    }
    const title = sessionTitle(s, item.session);
    if (item.kind === "plan") return title ? t("detail.plan", { title }) : t("detail.planUntitled");
    return title ? t("detail.record", { title }) : t("detail.recordUntitled");
  });
}

/**
 * The slot's mark in its head and on the rail (B46): a card's pull request
 * drawn as its PR mark, in its state's icon and colour, so it is told from
 * the PRs tool beside it; the record's page glyph. A pull request the list
 * no longer holds keeps the plain glyph in the caller's colour.
 */
export function DetailIcon({ item, className }: { item: DetailItem; className?: string }) {
  const state = useShellStore((s) => item.kind === "pull_request"
    ? catalogWorkspaces(s.rest).find((row) => row.id === item.workspace_id)?.pull_requests?.find((row) => row.number === item.number)?.state ?? null
    : null);
  if (item.kind === "mission") return <WaypointsIcon aria-hidden="true" className={className} data-detail-mark="mission" />;
  if (item.kind === "plan") return <ListChecksIcon aria-hidden="true" className={className} data-detail-mark="plan" />;
  if (item.kind === "worktrees") return <FolderGit2Icon aria-hidden="true" className={className} data-detail-mark="worktrees" />;
  if (item.kind === "cost") return <CoinsIcon aria-hidden="true" className={className} data-detail-mark="cost" />;
  if (item.kind !== "pull_request") return <FileTextIcon aria-hidden="true" className={className} />;
  const look = state ? PR_LOOK[state] : null;
  const Icon = look?.icon ?? GitPullRequestIcon;
  return <Icon aria-hidden="true" className={cn(className, look?.tone)} data-detail-mark={state ?? undefined} />;
}

export function useDetailTitle(): string {
  const item = useShellStore((s) => detailTop(s.rest?.ui_state?.detail));
  return useItemTitle(item);
}

export function DetailPanel({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const items = useShellStore((s) => s.rest?.ui_state?.detail?.items);
  const item = items?.at(-1) ?? null;
  const before = items && items.length > 1 ? items[items.length - 2]! : null;
  const title = useItemTitle(item);
  const back = useItemTitle(before?.kind === "record" && item?.kind === "record" ? before : null);
  const url = useShellStore((s) => {
    if (item?.kind !== "pull_request") return null;
    return catalogWorkspaces(s.rest).find((row) => row.id === item.workspace_id)?.pull_requests?.find((row) => row.number === item.number)?.url ?? null;
  });
  if (!item) return null;
  const key = item.kind === "pull_request" ? `pr:${item.workspace_id}:${item.number}` : item.kind === "mission" ? `mission:${item.mission}` : item.kind === "record" || item.kind === "plan" ? `${item.kind}:${workKey(item.session)}` : `${item.kind}:${item.workspace_id}`;
  return (
    // The slot reads as a page on the lane: its head keeps the lane's surface
    // and what it holds, a card or a record alike, sits on the page's.
    <aside className="flex h-full min-h-0 min-w-0 flex-1 flex-col bg-background text-foreground" aria-label={title} data-lane="detail" data-detail={item.kind}>
      <div className="flex h-[var(--size-tab-strip)] shrink-0 items-center gap-xxs border-b border-border bg-card px-xs text-caption" data-detail-head="true">
        <Hint label={t("detail.back")}>
          <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("detail.back")} disabled={!before} onClick={() => actions.detailBack()} data-detail-back="true">
            <ArrowLeftIcon />
          </Button>
        </Hint>
        <span className="flex min-w-0 flex-1 items-center gap-xs font-semibold" data-detail-title={title}>
          <DetailIcon item={item} className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
          <span className="min-w-0 truncate" title={title}>{title}</span>
        </span>
        {url ? (
          <Hint label={t("detail.github")}>
            <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("detail.github")} onClick={() => actions.openLink(url, true)} data-detail-github="true">
              <ExternalLinkIcon />
            </Button>
          </Hint>
        ) : null}
        <Hint label={t("detail.close")}>
          <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("detail.close")} onClick={() => actions.detailClose()} data-detail-close="true">
            <XIcon />
          </Button>
        </Hint>
      </div>
      {before?.kind === "record" && item.kind === "record" && back ? (
        <Button size="sm" variant="outline" className="mx-md mt-sm min-w-0 self-start" onClick={() => actions.detailBack()} data-detail-parent="true">
          <ArrowLeftIcon /><span className="truncate">{back.replace(/^[^·]*·\s*/, "")}</span>
        </Button>
      ) : null}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col" key={key}>
        {item.kind === "pull_request"
          ? <PrCardView workspaceId={item.workspace_id} number={item.number} actions={actions} />
          : item.kind === "mission"
            ? <MissionMap missionId={item.mission} actions={actions} />
            : item.kind === "plan"
              ? <PlanMap session={item.session} actions={actions} />
              : item.kind === "worktrees"
                ? <WorktreesView workspaceId={item.workspace_id} actions={actions} />
                : item.kind === "cost"
                  ? <ProjectCostView workspaceId={item.workspace_id} actions={actions} />
                  : <SlotSessionRecord actions={actions} target={item.session} />}
      </div>
    </aside>
  );
}
