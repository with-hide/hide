import { ChevronDownIcon, XIcon } from "lucide-react";
import { useMemo, useState } from "react";
import type { Actions } from "./actions";
import { deviceScope, type ToolCell } from "./agentScope";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger } from "./components/ui/dropdown-menu";
import { deviceMissionAsks } from "./components/sidebar-mission-row";
import { missionOwnAsks, ownAskProject } from "./ownAsks";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { localDeviceId } from "./devices";
import { boardProjects } from "./navigation";
import { allAgents, agentsByPane } from "./paneAgents";
import { frontProject } from "./projectStatus";
import { prListModel, type PrListModel } from "./prWork";
import { machineName } from "./screenMachine";
import { catalogWorkspaces, listedCatalog, type AgentRow, type SidebarProject, type SnapshotRest, type Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { groupOf, taskList, type TaskList } from "./taskList";
import { toolProjects, toolSidebarProjects, toolTaskList } from "./toolCells";

// The project chip over the sidebar's 태스크 and PR lists: one choice the core
// keeps (`ui_state.sidebar_project`) narrows both lists to a project of the
// rail's device, and a 현황 tile sets the same choice. While it narrows them,
// the 내 차례 of the other projects stays in sight as `+N`. A tool's cell draws
// the same chip over its own lists, on the same choice, reading the tool's work.

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

/**
 * Counts per project id of one device's open tasks and pull requests, 내 차례
 * apart, as the two lists' groups count them. Without `prTurn` no pull request
 * counts as a turn: a tool's turn is its tasks', since a pull request's 내 차례
 * is the operator's move.
 */
export function projectCounts(tasks: TaskList, prs: PrListModel, prTurn = true): Map<string, ProjectCount> {
  const counts = new Map<string, ProjectCount>();
  const add = (id: string | undefined, needsYou: boolean) => {
    if (id === undefined) return;
    const count = counts.get(id) ?? { needsYou: 0, open: 0 };
    count.open += 1;
    if (needsYou) count.needsYou += 1;
    counts.set(id, count);
  };
  for (const group of tasks.groups) for (const entry of group.entries) add(entry.project?.id, groupOf(entry.card.state) === "needs_you");
  for (const group of tasks.groups) for (const band of group.bands) for (const own of band.asks) add(ownAskProject(own) ?? undefined, true);
  for (const group of prs.groups) {
    if (group.group === "merged") continue;
    for (const entry of [...group.entries, ...group.team, ...group.old]) add(entry.project.id, prTurn && group.group === "needs_you");
  }
  return counts;
}

function deviceCounts(rest: SnapshotRest | null, localAgents: AgentRow[], deviceId: string): Map<string, ProjectCount> {
  const byPane = agentsByPane(allAgents(localAgents, rest?.status?.remote));
  const tasks = taskList(deviceScope(rest, deviceId)?.tasks, deviceId, byPane, listedCatalog(rest), "status", [], missionOwnAsks(deviceMissionAsks(rest, deviceId)));
  return projectCounts(tasks, prListModel(boardProjects(rest, localAgents, deviceId), null, Date.now()));
}

/**
 * `모든 프로젝트 ▾`, or the chosen project's name and ✕ that lifts it, at the
 * left of the 태스크 and PR lists' count line. The menu lists the device's
 * projects with their 내 차례 and open counts, and offers the project in
 * front first.
 */
export function SidebarProjectFilter({ deviceId, actions }: { deviceId: string; actions: Actions }) {
  const filter = useSidebarProject(deviceId);
  const rest = useShellStore((s) => s.rest);
  const localAgents = useShellStore((s) => s.agents);
  const [open, setOpen] = useState(false);
  const projects = useMemo(() => listedCatalog(rest).filter((row) => row.device_id === deviceId), [rest, deviceId]);
  // Only the menu and `+N` read the counts, so an unnarrowed list with the menu shut builds no second list.
  const counted = open || filter !== null;
  const counts = useMemo(() => (counted ? deviceCounts(rest, localAgents, deviceId) : NO_COUNTS), [counted, rest, localAgents, deviceId]);
  const front = frontProject(rest);
  return (
    <ProjectChip open={open} onOpenChange={setOpen} projects={projects} chosen={filter?.workspace_id ?? null} counts={counts}
      front={front && front.device_id === deviceId ? front : null} tool={false}
      choose={(id) => actions.setSidebarProject(id === null ? null : { device_id: deviceId, workspace_id: id })} />
  );
}

/**
 * The same chip over a tool's cell's 태스크 and PR lists (operator decision
 * 2026-11-15): it reads the tool's work, never the operator's. The menu lists
 * the projects the tool works in on every device it reaches, each counted
 * from the tool's task list and its PR cut, and its turn and `+N` are the
 * tool's (`도구가 답할 차례`), drawn muted since they call no one (D-46). It
 * keeps the one stored choice the operator's chip keeps; `chosen` is that
 * choice while it is one of the tool's projects (`toolProjectChoice`).
 */
export function ToolProjectFilter({ tool, chosen, actions }: { tool: ToolCell; chosen: SidebarProject | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((s) => s.rest);
  const localAgents = useShellStore((s) => s.agents);
  const [open, setOpen] = useState(false);
  const projects = useMemo(() => toolProjects(boardProjects(rest, localAgents), tool).map((project) => project.workspace), [rest, localAgents, tool]);
  const counted = open || chosen !== null;
  const counts = useMemo(() => (counted ? toolCounts(rest, localAgents, tool) : NO_COUNTS), [counted, rest, localAgents, tool]);
  const front = frontProject(rest);
  const local = localDeviceId(rest);
  return (
    <ProjectChip open={open} onOpenChange={setOpen} projects={projects} chosen={chosen?.workspace_id ?? null} counts={counts}
      front={front && projects.some((row) => row.id === front.id && row.device_id === front.device_id) ? front : null} tool={true}
      machine={(row) => (row.device_id === local ? null : machineName(rest?.navigator?.devices, row.device_id, t))}
      choose={(id) => {
        const project = id === null ? undefined : projects.find((row) => row.id === id);
        actions.setSidebarProject(project ? { device_id: project.device_id, workspace_id: project.id } : null);
      }} />
  );
}

const NO_COUNTS = new Map<string, ProjectCount>();

/** A tool's counts per project: its tasks, its turn among them, and its PR cut's open pull requests, none of which is the tool's turn. */
function toolCounts(rest: SnapshotRest | null, localAgents: AgentRow[], tool: ToolCell): Map<string, ProjectCount> {
  const byPane = agentsByPane(allAgents(localAgents, rest?.status?.remote));
  const tasks = toolTaskList(rest, tool, byPane, (id) => id, "status");
  return projectCounts(tasks, prListModel(toolSidebarProjects(boardProjects(rest, localAgents), tool), null, Date.now()), false);
}

function ProjectChip({ open, onOpenChange, projects, chosen, counts, front, tool, machine, choose }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  projects: readonly Workspace[];
  chosen: string | null;
  counts: ReadonlyMap<string, ProjectCount>;
  front: Workspace | null;
  /** A tool's cell: the turn counted is the tool's, drawn muted. */
  tool: boolean;
  machine?: (project: Workspace) => string | null;
  choose: (id: string | null) => void;
}) {
  const { t } = useInterfaceTranslation();
  const here = front && front.id !== chosen ? front : null;
  const total = [...counts.values()].reduce((sum, count) => ({ needsYou: sum.needsYou + count.needsYou, open: sum.open + count.open }), { needsYou: 0, open: 0 });
  const elsewhere = chosen ? total.needsYou - (counts.get(chosen)?.needsYou ?? 0) : 0;
  const pick = (id: string | null) => {
    if (id !== chosen) choose(id);
  };
  const label = chosen ? (projects.find((row) => row.id === chosen)?.label ?? chosen) : t("projectTool.sidebar.allProjects");
  const elsewhereLabel = t(tool ? "projectTool.sidebar.toolElsewhere" : "projectTool.sidebar.elsewhere", { count: elsewhere });
  return (
    <span className="flex min-w-0 shrink items-center gap-xs" data-sidebar-project={chosen ?? ""}>
      <span className={cn("flex min-w-0 items-center rounded-xs border border-border text-micro", chosen ? "bg-secondary text-foreground" : "text-subtle-foreground")}>
        <DropdownMenu open={open} onOpenChange={onOpenChange}>
          <DropdownMenuTrigger asChild>
            <button type="button" aria-label={t("projectTool.sidebar.choose", { project: label })} data-sidebar-project-trigger="true"
              className="flex min-w-0 items-center gap-xxs rounded-xs py-px pl-xs pr-xxs outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring">
              <span className="min-w-0 truncate">{label}</span>
              {chosen ? null : <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />}
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="max-w-(--size-settings-control-w)" data-sidebar-project-menu="true">
            <DropdownMenuRadioGroup value={chosen ?? ""} onValueChange={(id) => pick(id || null)}>
              <DropdownMenuRadioItem value="" data-sidebar-project-option="">
                <span className="min-w-0 flex-1 truncate">{t("projectTool.sidebar.allProjects")}</span>
                <Counts count={total} tool={tool} />
              </DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
            {here ? (
              <DropdownMenuItem inset onSelect={() => pick(here.id)} data-sidebar-project-front={here.id}>
                <span className="shrink-0">{t("projectTool.sidebar.frontProject")}</span>
                <span className="ml-auto min-w-0 truncate text-caption text-muted-foreground">{here.label}</span>
              </DropdownMenuItem>
            ) : null}
            {projects.length > 0 ? <DropdownMenuSeparator /> : null}
            <DropdownMenuRadioGroup value={chosen ?? ""} onValueChange={(id) => pick(id)}>
              {projects.map((project) => {
                const where = machine?.(project) ?? null;
                return (
                  <DropdownMenuRadioItem key={`${project.device_id}:${project.id}`} value={project.id} data-sidebar-project-option={project.id}>
                    <span className="min-w-0 flex-1 truncate">{project.label}</span>
                    {where ? <span className="shrink-0 truncate text-caption text-subtle-foreground" data-sidebar-project-machine={project.device_id}>{where}</span> : null}
                    <Counts count={counts.get(project.id) ?? { needsYou: 0, open: 0 }} tool={tool} />
                  </DropdownMenuRadioItem>
                );
              })}
            </DropdownMenuRadioGroup>
            <DropdownMenuLabel className="text-micro font-normal text-subtle-foreground">{t("projectTool.sidebar.applies")}</DropdownMenuLabel>
          </DropdownMenuContent>
        </DropdownMenu>
        {chosen ? (
          <Hint label={t("projectTool.sidebar.clearFilter")}>
            <button type="button" aria-label={t("projectTool.sidebar.clearFilter")} onClick={() => pick(null)} data-sidebar-project-clear="true"
              className="flex shrink-0 items-center rounded-xs px-xxs py-px text-muted-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring">
              <XIcon aria-hidden="true" className="size-(--size-icon-sm)" />
            </button>
          </Hint>
        ) : null}
      </span>
      {elsewhere > 0 ? (
        <Hint label={elsewhereLabel}>
          <button type="button" aria-label={elsewhereLabel} onClick={() => pick(null)} data-sidebar-project-elsewhere={elsewhere} data-tone={tool ? "muted" : "warning"}
            className={cn("shrink-0 rounded-full px-xs text-micro tabular-nums outline-none focus-visible:ring-1 focus-visible:ring-ring", tool ? "bg-secondary text-muted-foreground hover:text-foreground" : "bg-warning/15 text-warning hover:bg-warning/25")}>
            +{elsewhere}
          </button>
        </Hint>
      ) : null}
    </span>
  );
}

/** A menu row's turn and its open count; a zero is not drawn. The operator's 내 차례 is in the warning colour, a tool's turn muted (D-46). */
function Counts({ count, tool }: { count: ProjectCount; tool: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className="flex shrink-0 items-center gap-sm pl-md text-caption tabular-nums">
      {count.needsYou > 0 ? <span className={tool ? "text-muted-foreground" : "text-warning"} data-sidebar-project-turn={tool ? "tool" : "operator"}>{t(tool ? "taskLens.tool.group.needs_you" : "taskLens.group.needs_you")} {count.needsYou}</span> : null}
      {count.open > 0 ? <span className="text-muted-foreground">{count.open}</span> : null}
    </span>
  );
}
