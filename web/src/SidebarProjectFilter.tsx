import { ChevronDownIcon, XIcon } from "lucide-react";
import { useMemo, useState } from "react";
import type { Actions } from "./actions";
import { deviceScope } from "./agentScope";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger } from "./components/ui/dropdown-menu";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { boardProjects } from "./navigation";
import { allAgents, agentsByPane } from "./paneAgents";
import { frontProject } from "./projectStatus";
import { prListModel, type PrListModel } from "./prWork";
import { catalogWorkspaces, type AgentRow, type SidebarProject, type SnapshotRest, type Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { groupOf, taskList, type TaskList } from "./taskList";

// The project chip over the sidebar's 태스크 and PR lists: one choice the core
// keeps (`ui_state.sidebar_project`) narrows both lists to a project of the
// rail's device, and a 현황 tile sets the same choice. While it narrows them,
// the 내 차례 of the other projects stays in sight as `+N`.

/**
 * The project the sidebar's lists are narrowed to on `deviceId`, or null for
 * every project, as it is while the device's catalog does not hold the project.
 */
export function useSidebarProject(deviceId: string): SidebarProject | null {
  return useShellStore((s) => {
    const project = s.rest?.ui_state?.sidebar_project;
    if (!project || project.device_id !== deviceId) return null;
    return catalogWorkspaces(s.rest).some((row) => row.id === project.workspace_id && row.device_id === deviceId) ? project : null;
  });
}

/** What a project holds for the operator: its 내 차례 and every open task and pull request. */
export type ProjectCount = { needsYou: number; open: number };

/** Counts per project id of one device's open tasks and pull requests, 내 차례 apart, as the two lists' groups count them. */
export function projectCounts(tasks: TaskList, prs: PrListModel): Map<string, ProjectCount> {
  const counts = new Map<string, ProjectCount>();
  const add = (id: string | undefined, needsYou: boolean) => {
    if (id === undefined) return;
    const count = counts.get(id) ?? { needsYou: 0, open: 0 };
    count.open += 1;
    if (needsYou) count.needsYou += 1;
    counts.set(id, count);
  };
  for (const group of tasks.groups) for (const entry of group.entries) add(entry.project?.id, groupOf(entry.card.state) === "needs_you");
  for (const group of prs.groups) {
    if (group.group === "merged") continue;
    for (const entry of [...group.entries, ...group.team, ...group.old]) add(entry.project.id, group.group === "needs_you");
  }
  return counts;
}

function deviceCounts(rest: SnapshotRest | null, localAgents: AgentRow[], deviceId: string): Map<string, ProjectCount> {
  const byPane = agentsByPane(allAgents(localAgents, rest?.status?.remote));
  const tasks = taskList(deviceScope(rest, deviceId)?.tasks, byPane, catalogWorkspaces(rest), "status");
  return projectCounts(tasks, prListModel(boardProjects(rest, localAgents, deviceId), null, Date.now()));
}

/**
 * `모든 프로젝트 ▾`, or the chosen project's name and ✕ that lifts it, at the
 * left of the 태스크 and PR lists' count line. The menu lists the device's
 * projects with their 내 차례 and open counts, and offers the project in
 * front first.
 */
export function SidebarProjectFilter({ deviceId, actions }: { deviceId: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const filter = useSidebarProject(deviceId);
  const rest = useShellStore((s) => s.rest);
  const localAgents = useShellStore((s) => s.agents);
  const [open, setOpen] = useState(false);
  const projects = useMemo(() => catalogWorkspaces(rest).filter((row) => row.device_id === deviceId && !row.is_home), [rest, deviceId]);
  // Only the menu and `+N` read the counts, so an unnarrowed list with the menu shut builds no second list.
  const counted = open || filter !== null;
  const counts = useMemo(() => (counted ? deviceCounts(rest, localAgents, deviceId) : new Map<string, ProjectCount>()), [counted, rest, localAgents, deviceId]);
  const front = frontProject(rest);
  const here = front && front.device_id === deviceId && front.id !== filter?.workspace_id ? front : null;
  const total = [...counts.values()].reduce((sum, count) => ({ needsYou: sum.needsYou + count.needsYou, open: sum.open + count.open }), { needsYou: 0, open: 0 });
  const elsewhere = filter ? total.needsYou - (counts.get(filter.workspace_id)?.needsYou ?? 0) : 0;
  const choose = (id: string | null) => {
    if (id !== (filter?.workspace_id ?? null)) actions.setSidebarProject(id === null ? null : { device_id: deviceId, workspace_id: id });
  };
  const label = filter ? (projects.find((row) => row.id === filter.workspace_id)?.label ?? filter.workspace_id) : t("projectTool.sidebar.allProjects");
  return (
    <span className="flex min-w-0 shrink items-center gap-xs" data-sidebar-project={filter?.workspace_id ?? ""}>
      <span className={cn("flex min-w-0 items-center rounded-xs border border-border text-micro", filter ? "bg-secondary text-foreground" : "text-subtle-foreground")}>
        <DropdownMenu open={open} onOpenChange={setOpen}>
          <DropdownMenuTrigger asChild>
            <button type="button" aria-label={t("projectTool.sidebar.choose", { project: label })} data-sidebar-project-trigger="true"
              className="flex min-w-0 items-center gap-xxs rounded-xs py-px pl-xs pr-xxs outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring">
              <span className="min-w-0 truncate">{label}</span>
              {filter ? null : <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />}
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="max-w-(--size-settings-control-w)" data-sidebar-project-menu="true">
            <DropdownMenuRadioGroup value={filter?.workspace_id ?? ""} onValueChange={(id) => choose(id || null)}>
              <DropdownMenuRadioItem value="" data-sidebar-project-option="">
                <span className="min-w-0 flex-1 truncate">{t("projectTool.sidebar.allProjects")}</span>
                <Counts count={total} />
              </DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
            {here ? (
              <DropdownMenuItem inset onSelect={() => choose(here.id)} data-sidebar-project-front={here.id}>
                <span className="shrink-0">{t("projectTool.sidebar.frontProject")}</span>
                <span className="ml-auto min-w-0 truncate text-caption text-muted-foreground">{here.label}</span>
              </DropdownMenuItem>
            ) : null}
            {projects.length > 0 ? <DropdownMenuSeparator /> : null}
            <DropdownMenuRadioGroup value={filter?.workspace_id ?? ""} onValueChange={(id) => choose(id)}>
              {projects.map((project: Workspace) => (
                <DropdownMenuRadioItem key={project.id} value={project.id} data-sidebar-project-option={project.id}>
                  <span className="min-w-0 flex-1 truncate">{project.label}</span>
                  <Counts count={counts.get(project.id) ?? { needsYou: 0, open: 0 }} />
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
            <DropdownMenuLabel className="text-micro font-normal text-subtle-foreground">{t("projectTool.sidebar.applies")}</DropdownMenuLabel>
          </DropdownMenuContent>
        </DropdownMenu>
        {filter ? (
          <Hint label={t("projectTool.sidebar.clearFilter")}>
            <button type="button" aria-label={t("projectTool.sidebar.clearFilter")} onClick={() => choose(null)} data-sidebar-project-clear="true"
              className="flex shrink-0 items-center rounded-xs px-xxs py-px text-muted-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring">
              <XIcon aria-hidden="true" className="size-(--size-icon-sm)" />
            </button>
          </Hint>
        ) : null}
      </span>
      {elsewhere > 0 ? (
        <Hint label={t("projectTool.sidebar.elsewhere", { count: elsewhere })}>
          <button type="button" aria-label={t("projectTool.sidebar.elsewhere", { count: elsewhere })} onClick={() => choose(null)} data-sidebar-project-elsewhere={elsewhere}
            className="shrink-0 rounded-full bg-warning/15 px-xs text-micro text-warning tabular-nums outline-none hover:bg-warning/25 focus-visible:ring-1 focus-visible:ring-ring">
            +{elsewhere}
          </button>
        </Hint>
      ) : null}
    </span>
  );
}

/** A menu row's 내 차례 in the warning colour and its open count; a zero is not drawn. */
function Counts({ count }: { count: ProjectCount }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className="flex shrink-0 items-center gap-sm pl-md text-caption tabular-nums">
      {count.needsYou > 0 ? <span className="text-warning">{t("taskLens.group.needs_you")} {count.needsYou}</span> : null}
      {count.open > 0 ? <span className="text-muted-foreground">{count.open}</span> : null}
    </span>
  );
}
