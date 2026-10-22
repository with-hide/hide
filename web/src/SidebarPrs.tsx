import { TriangleAlertIcon, XIcon } from "lucide-react";
import { useMemo } from "react";
import type { Actions } from "./actions";
import { Hint } from "./components/ui/tooltip";
import { frontDeviceId } from "./devices";
import { useInterfaceTranslation } from "./i18n/client";
import { boardProjects } from "./navigation";
import { PrList } from "./PrList";
import { prListModel, readingWarning } from "./prWork";
import { useRequestViewDemand } from "./requestViewDemand";
import { catalogWorkspaces } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore, type SidebarProject } from "./ui";

// The sidebar's PR list (PRD lane-two-columns D-20, B41; PRD
// sidebar-task-lens D-17, D-45): every pull request of the rail's device in
// the core's groups, rows, order and folds, narrowed to one project by the
// chip a 현황 tile sets. A row opens its card in the lane's 보기 slot.

/** The project the sidebar's lists are narrowed to on `deviceId`, or null for every project. */
export function useSidebarProject(deviceId: string): SidebarProject | null {
  return useUiStore((s) => (s.sidebarProject?.deviceId === deviceId ? s.sidebarProject : null));
}

/** The narrowing chip (B27, B29): the project's name, and ✕ that shows every project again. */
export function SidebarProjectChip({ filter }: { filter: SidebarProject }) {
  const { t } = useInterfaceTranslation();
  const label = useShellStore((s) => catalogWorkspaces(s.rest).find((row) => row.id === filter.projectId)?.label ?? filter.projectId);
  return (
    <span className="flex min-w-0 items-center gap-xxs rounded-sm bg-secondary py-px pl-xs text-micro text-foreground" data-sidebar-project={filter.projectId}>
      <span className="min-w-0 truncate">{t("projectTool.sidebar.filter", { project: label })}</span>
      <Hint label={t("projectTool.sidebar.clearFilter")}>
        <button type="button" aria-label={t("projectTool.sidebar.clearFilter")} className="flex shrink-0 items-center rounded-xs px-xxs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring" onClick={() => useUiStore.getState().setSidebarProject(null)} data-sidebar-project-clear="true">
          <XIcon aria-hidden="true" className="size-(--size-icon-sm)" />
        </button>
      </Hint>
    </span>
  );
}

export function SidebarPrs({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((state) => state.rest);
  const agents = useShellStore((state) => state.agents);
  const live = useShellStore((state) => state.connection === "live");
  const deviceId = frontDeviceId(rest);
  const filter = useSidebarProject(deviceId);
  const model = useMemo(() => prListModel(boardProjects(rest, agents, deviceId).filter((item) => !filter || item.workspace.id === filter.projectId), null, Date.now()), [rest, agents, deviceId, filter]);
  // The board's GitHub reads follow what is on screen.
  useRequestViewDemand(actions, live);
  // ⚠ while the last read failed or left a question beside the list unanswered, its hint saying which (work-tab-pr-lens B43).
  const staleText = readingWarning(model.reading, t);
  const reading = model.reading.state === "reading" && model.groups.length === 0;
  const open = model.groups.filter((group) => group.group !== "merged").reduce((sum, group) => sum + group.entries.length + group.team.length + group.old.length, 0);
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-sidebar-prs="true">
      <div className="flex shrink-0 items-center gap-xs px-md py-xs text-micro text-muted-foreground" data-prs-header="true">
        <span className="shrink-0" data-prs-total={reading ? undefined : open}>{reading ? t("projectTool.sidebar.prsLoading") : t("projectTool.sidebar.prsCount", { count: open })}</span>
        {staleText ? (
          <Hint label={staleText}>
            <span role="img" aria-label={staleText} className="inline-flex shrink-0 text-warning" data-prs-stale="true"><TriangleAlertIcon aria-hidden="true" className="size-(--size-icon-sm)" /></span>
          </Hint>
        ) : null}
        <span className="flex-1" />
        {filter ? <SidebarProjectChip filter={filter} /> : null}
      </div>
      {model.projects === 0 && model.reading.state !== "blocked"
        ? <p className="px-md py-sm text-caption text-muted-foreground" data-prs-view="no-remote">{t("prs.noRemote")}</p>
        : <PrList model={model} actions={actions} onlyCheckout={false} showProject={filter === null} onAllCheckouts={() => useUiStore.getState().setSidebarProject(null)} />}
    </div>
  );
}
