import { CoinsIcon, FileDiffIcon, FileTextIcon, FolderGit2Icon, GitBranchIcon, GitPullRequestIcon, ListChecksIcon, SettingsIcon, WaypointsIcon } from "lucide-react";
import { cn } from "./lib/utils";
import { PR_LOOK } from "./prMark";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { MissionMap } from "./MissionMap";
import { PlanMap } from "./PlanMap";
import { ProjectCostView, RemainsView, WorktreesView } from "./ProjectDetail";
import { BranchCardView } from "./BranchCard";
import { PrCardView } from "./PrCard";
import { SlotSessionRecord } from "./SessionRecord";
import { cardPath, cardTitle } from "./cardTitle";
import { OpenerMark } from "./ViewComments";
import { useDeskFront } from "./deskFront";
import { workKey } from "./sessionWork";
import { catalogWorkspaces, type DetailItem, type ViewOpener } from "./snapshot";
import { useShellStore } from "./store";

// A card on a desk (PRD lane-desk-pair-view D-13): one pull request's card,
// one session's record, a mission's map, a session's plan, a project's
// worktrees or cost, a checkout's remains, or a Hide AI branch's card, drawn
// as a View tab's body. The core keeps which desk holds it; this draws its
// name, its mark and what it shows.

/** A card's name, kept current as the shell's state moves. */
export function useCardTitle(item: DetailItem | null, bare = false): string {
  const { t } = useInterfaceTranslation();
  return useShellStore((s) => cardTitle(s, t, item, bare));
}

/**
 * A card's mark on its tab (B46): a card's pull request
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
  if (item.kind === "remains") return <FileDiffIcon aria-hidden="true" className={className} data-detail-mark="remains" />;
  if (item.kind === "branch") return <GitBranchIcon aria-hidden="true" className={className} data-detail-mark="branch" />;
  if (item.kind !== "pull_request") return <FileTextIcon aria-hidden="true" className={className} />;
  const look = state ? PR_LOOK[state] : null;
  const Icon = look?.icon ?? GitPullRequestIcon;
  return <Icon aria-hidden="true" className={cn(className, look?.tone)} data-detail-mark={state ?? undefined} />;
}

/**
 * A card tab's body: its path line, as a file's path toolbar stands over a
 * file (PRD lane-desk-pair-view D-30), with what the card points at
 * (`cardPath`), a mission's way to every mission and its settings, and the
 * logo of the agent the card belongs to where it has one, as a View's opener
 * ends a file's (`ownerPane`, the core's owner of the card's desk tab); then
 * what the card shows. A pull request's GitHub page is
 * its card's own button, so the path line carries none.
 */
export function CardBody({ item, ownerPane = null, actions }: { item: DetailItem; ownerPane?: string | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const name = useShellStore((s) => cardPath(s, t, item));
  const front = useDeskFront(item);
  const ownerRow = useShellStore((s) => (ownerPane ? s.agents.find((row) => row.pane_id === ownerPane) ?? null : null));
  const owner: ViewOpener | null = ownerRow ? { name: ownerRow.identity_label, provider: ownerRow.agent_kind, device_id: ownerRow.device_id, pane_id: ownerRow.pane_id, resumable: false } : null;
  const key = item.kind === "pull_request" ? `pr:${item.workspace_id}:${item.number}` : item.kind === "mission" ? `mission:${item.mission}` : item.kind === "record" || item.kind === "plan" ? `${item.kind}:${workKey(item.session)}` : item.kind === "branch" ? `branch:${item.workspace_id}:${item.row}:${item.path ?? ""}` : `${item.kind}:${item.workspace_id}`;
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-background text-foreground" data-card={item.kind}>
      <div className="flex h-[var(--size-tab-strip)] shrink-0 items-center gap-xxs border-b border-border bg-card px-xs text-caption" data-card-head="true">
        <span className="min-w-0 flex-1 truncate text-muted-foreground" data-card-name="true">{name}</span>
        {item.kind === "mission" ? (
          <>
            {/* The sidebar's 미션 row's own mark, so the button reads as the way to the same screen. */}
            <Button variant="secondary" size="sm" className="mr-xxs" onClick={() => actions.openMissions()} data-detail-all-missions="true">
              <WaypointsIcon aria-hidden="true" />
              {t("missionScreen.allMissions")}
            </Button>
            <Hint label={t("missionScreen.settings")}>
              <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("missionScreen.settings")} onClick={() => actions.openMissionSettings()} data-detail-conductor-settings="true">
                <SettingsIcon />
              </Button>
            </Hint>
          </>
        ) : null}
        {owner ? <OpenerMark opener={owner} role="owner" actions={actions} /> : null}
      </div>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col" key={key}>
        {item.kind === "pull_request"
          ? <PrCardView workspaceId={item.workspace_id} number={item.number} actions={actions} front={front} />
          : item.kind === "mission"
            ? <MissionMap missionId={item.mission} actions={actions} />
            : item.kind === "plan"
              ? <PlanMap session={item.session} actions={actions} />
              : item.kind === "worktrees"
                ? <WorktreesView workspaceId={item.workspace_id} actions={actions} />
                : item.kind === "cost"
                  ? <ProjectCostView workspaceId={item.workspace_id} actions={actions} />
                  : item.kind === "remains"
                    ? <RemainsView workspaceId={item.workspace_id} checkoutId={item.checkout_id} actions={actions} />
                    : item.kind === "branch"
                    ? <BranchCardView workspaceId={item.workspace_id} rowId={item.row} path={item.path ?? null} actions={actions} />
                    : <SlotSessionRecord actions={actions} target={item.session} />}
      </div>
    </div>
  );
}
