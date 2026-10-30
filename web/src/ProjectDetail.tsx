import { ChevronRightIcon, LoaderCircleIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { Actions } from "./actions";
import type { ProjectStatus } from "./agentScope";
import { agentProductName } from "./agentAdapters";
import { PrMark } from "./components/pr-mark";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { readsHere } from "./devices";
import { entranceBytes, layerLines } from "./diskCleanup";
import { useDiskWatch } from "./diskWatch";
import { LAYER_DOT, openDiskCleanup } from "./DiskCleanupSheet";
import { formatBytes } from "./i18n/format";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { allAgents } from "./paneAgents";
import { RowButton, RowOverlayButton } from "./components/ui/row-button";
import { ToggleGroup, ToggleGroupItem } from "./components/ui/toggle-group";
import type { TaskCost, TaskMission } from "./generated/hided-ws";
import { costPrefix, dayHeights, durationWords, filterCounts, projectCost, sessionName, TASK_FILTERS, taskIn, taskName, tokenWords, usd, type TaskFilter } from "./projectStatus";
import { TaskResultMark } from "./ProjectTool";
import { catalogWorkspaces, changesFor, frontCheckout, localDeviceId, type ChangedFileSnapshot, type Checkout, type Workspace } from "./snapshot";
import type { WorkspaceView } from "./workspace";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// What the Project tool opens in the 보기 slot (PRD lane-two-columns B30,
// project-panel-results B9, B27-B34): the project's worktrees with its disk,
// what one worktree left, and the seven days' cost by task. Each draws what
// the core sent; a removal goes through the disk cleanup sheet's
// confirmation as it does from the Overview.

function useProject(workspaceId: string): Workspace | null {
  return useShellStore((s) => catalogWorkspaces(s.rest).find((row) => row.id === workspaceId) ?? null);
}

function Missing() {
  const { t } = useInterfaceTranslation();
  return <p className="px-md py-sm text-caption text-muted-foreground">{t("overview.projectMissing")}</p>;
}

type DotState = ProjectStatus["git"]["dots"][number]["state"];
/** Changed first, then not pushed, then clean, as the tile's dots stand. */
const MARK_ORDER: readonly DotState[] = ["changed", "unpushed", "removable", "clean"];
const DOT_TONES: Record<DotState, string> = { changed: "bg-warning", unpushed: "bg-agent-working", removable: "bg-pr-merged", clean: "bg-border" };

/**
 * The worktree table (B30): the project's disk bar, then a row per worktree
 * marked by the core's state, the ones that can go folded under `정리할 것`
 * with the cleanup sheet one press away. Opening reads the worktrees again,
 * and while it shows (and no Overview tab measures) the project's disk is
 * measured.
 */
export function WorktreesView({ workspaceId, actions }: { workspaceId: string; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const project = useProject(workspaceId);
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const overviewOpen = useUiStore((s) => s.overviewOpen);
  const [foldOpen, setFoldOpen] = useState(false);
  useEffect(() => actions.refreshProjectOverview(workspaceId), [actions, workspaceId]);
  const measures = project !== null && readsHere(project, devices);
  // One tab's disk is watched at a time; the Overview's Projects tab takes it while it shows.
  useDiskWatch(actions, measures && !overviewOpen ? workspaceId : null);
  if (!project) return <Missing />;
  // Every worktree of the catalog by its own mark, not the tile's capped dots.
  const rows = project.checkouts.filter((checkout) => checkout.agent_scope.git_mark !== undefined);
  const kept = rows.filter((checkout) => checkout.agent_scope.git_mark !== "removable").sort((a, b) => MARK_ORDER.indexOf(a.agent_scope.git_mark!) - MARK_ORDER.indexOf(b.agent_scope.git_mark!));
  const removable = rows.filter((checkout) => checkout.agent_scope.git_mark === "removable");
  const lines = measures ? layerLines(project, t) : null;
  const size = entranceBytes(project);
  return (
    <div className="min-h-0 flex-1 space-y-md overflow-y-auto px-md py-md text-caption" data-worktrees-view={project.id}>
      {measures ? (
        <section className="space-y-xs" data-worktrees-storage="true">
          <h3 className="flex items-center gap-xs font-medium text-subtle-foreground">
            {t("projectTool.worktrees.storage")}
            {size ? <span className="font-mono text-muted-foreground">{size.partial ? "≥ " : ""}{formatBytes(language, size.bytes)}</span> : null}
            {project.disk?.measuring ? <LoaderCircleIcon role="status" aria-label={t("overviewProjects.measuring")} className="size-(--size-icon-sm) animate-spin text-muted-foreground" /> : null}
          </h3>
          {lines ? (
            <>
              <span aria-hidden="true" className="flex h-(--lens-bar-height) w-full overflow-hidden rounded-xs bg-muted">
                {lines.map((line) => <span key={line.key} className={LAYER_DOT[line.key]} style={{ flexGrow: line.bytes }} />)}
              </span>
              <ul className="flex flex-wrap gap-x-md gap-y-xxs text-micro text-muted-foreground">
                {lines.map((line) => (
                  <li key={line.key} className="inline-flex items-center gap-xs">
                    <span aria-hidden="true" className={cn("size-(--size-status-mark) rounded-full", LAYER_DOT[line.key])} />{line.label} <span className="font-mono">{formatBytes(language, line.bytes)}</span>
                  </li>
                ))}
              </ul>
            </>
          ) : <span aria-hidden="true" className="block h-(--lens-bar-height) w-full rounded-xs bg-muted" />}
        </section>
      ) : null}
      {rows.length === 0 ? <p className="text-muted-foreground">{t("projectTool.worktrees.empty")}</p> : (
        <ul className="flex min-w-0 flex-col" data-worktrees-rows={kept.length}>
          {kept.map((checkout) => <WorktreeRow key={checkout.id} checkout={checkout} state={checkout.agent_scope.git_mark!} />)}
        </ul>
      )}
      {removable.length > 0 ? (
        <section className="flex min-w-0 flex-col gap-xs" data-worktrees-cleanup={removable.length}>
          <div className="flex items-center gap-sm">
            <button type="button" aria-expanded={foldOpen} className="flex items-center gap-xxs rounded-xs px-xs text-caption text-pr-merged outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" onClick={() => setFoldOpen(!foldOpen)} data-worktrees-fold="true">
              {t("projectTool.worktrees.cleanup", { count: removable.length })}
              <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon-sm)", foldOpen && "rotate-90")} />
            </button>
            {measures ? <Button size="sm" variant="ghost" onClick={() => openDiskCleanup(project.id, "done")} data-worktrees-cleanup-open="true">{t("projectTool.worktrees.cleanupAction")}</Button> : null}
          </div>
          {foldOpen ? <ul className="flex min-w-0 flex-col">{removable.map((checkout) => <WorktreeRow key={checkout.id} checkout={checkout} state="removable" />)}</ul> : null}
        </section>
      ) : null}
    </div>
  );
}

function WorktreeRow({ checkout, state }: { checkout: Checkout; state: DotState }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const bytes = checkout.worktree?.disk?.total_bytes ?? null;
  return (
    <li className="flex min-w-0 items-center gap-xs border-b border-border py-xs" data-worktree-row={checkout.id} data-worktree-state={state}>
      <span aria-hidden="true" className={cn("size-(--size-status-mark) shrink-0 rounded-full", DOT_TONES[state])} />
      <span className="min-w-0 flex-1 truncate text-foreground" title={checkout.path}>{checkout.branch ?? checkout.label}</span>
      {checkout.pull_request ? <PrMark state={checkout.pull_request.state} number={checkout.pull_request.number} compact /> : null}
      <span className="shrink-0 text-micro text-muted-foreground">{t(`projectTool.worktrees.state.${state}`)}</span>
      {bytes !== null ? <span className="min-w-(--size-control-lg) shrink-0 whitespace-nowrap text-right font-mono text-micro text-muted-foreground">{formatBytes(language, bytes)}</span> : null}
    </li>
  );
}

/**
 * 작업 비용 (B27-B34): the seven days' total and each day, the amounts by
 * model and by agent, then the tasks under one filter, costliest first, each
 * opening onto its sessions or its mission's nodes. Under 결과 없음 a row
 * says what its agent said last and opens its conversation. Hide AI's own
 * use stands below, in no task (B24).
 */
export function ProjectCostView({ workspaceId, actions }: { workspaceId: string; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const project = useProject(workspaceId);
  const local = useShellStore((s) => project?.device_id === localDeviceId(s.rest));
  const node = useShellStore((s) => s.usage?.nodes[0]);
  const read = useMemo(() => projectCost(node, workspaceId), [node, workspaceId]);
  const table = useUiStore((s) => s.taskTable);
  if (!project) return <Missing />;
  if (!local) return <p className="px-md py-sm text-caption text-muted-foreground">{t("projectTool.cost.otherDevice")}</p>;
  if (read.state === "pending") return <div role="status" aria-label={t("projectTool.cost.reading")} className="m-md h-(--size-control-lg) rounded-sm bg-muted" data-cost-view="pending" />;
  if (read.state === "failed") return <p className="px-md py-sm text-caption text-muted-foreground" data-cost-view="failed">{t("projectTool.cost.failed")}</p>;
  const cost = read.cost;
  const hideAi = node?.hide_ai_week_micro_usd ?? 0;
  if (!cost || cost.tasks_total === 0) {
    return (
      <div className="min-h-0 flex-1 space-y-sm overflow-y-auto px-md py-md text-caption" data-cost-view="empty">
        <p className="text-muted-foreground">{t("projectTool.cost.empty")}</p>
        {hideAi > 0 ? <HideAiLine microUsd={hideAi} /> : null}
      </div>
    );
  }
  const heights = dayHeights(cost.days);
  const counts = filterCounts(cost.tasks);
  const rows = cost.tasks.filter((task) => taskIn(task, table.filter));
  return (
    <div className="min-h-0 flex-1 space-y-lg overflow-y-auto px-md py-md text-caption" data-cost-view={project.id}>
      <section className="space-y-xxs">
        <p className="text-headline font-semibold tabular-nums text-foreground" data-cost-total={cost.micro_usd}>{costPrefix(cost)} {usd(cost.micro_usd, language)}</p>
        <p className="text-micro text-muted-foreground">
          {[t("projectTool.cost.apiEquivalent"), t("projectTool.week.tokens", { tokens: tokenWords(language, cost.tokens) }), cost.partial ? t("projectTool.cost.partial") : null, cost.limited ? t("projectTool.cost.limited") : null, cost.stale ? t("usage.history.stalePrice") : null, cost.waiting ? t("usage.history.waitingPrice") : null, read.failed ? t("projectTool.cost.failedKept") : null].filter(Boolean).join(" · ")}
        </p>
        {cost.unpriced_agents.length > 0 ? <p className="text-micro text-muted-foreground" data-cost-unpriced={cost.unpriced_agents.length}>{t("projectTool.cost.unpriced", { agents: cost.unpriced_agents.map((agent) => agentProductName(agent)).join(", ") })}</p> : null}
      </section>
      <section className="space-y-xs">
        <h3 className="font-medium text-subtle-foreground">{t("projectTool.cost.days")}</h3>
        <div className="flex h-(--size-control-lg) items-end gap-xs" data-cost-days={cost.days.length}>
          {cost.days.map((day, index) => (
            <Hint key={day.date} label={`${day.date} · ${usd(day.micro_usd, language)}`}>
              <span tabIndex={0} className="flex h-full flex-1 items-end outline-none focus-visible:ring-1 focus-visible:ring-ring">
                <span className="w-full rounded-t-xs bg-primary" style={{ height: `${Math.max(heights[index]! * 100, heights[index]! > 0 ? 6 : 2)}%`, opacity: heights[index]! > 0 ? 1 : 0.3 }} data-cost-day={day.date} />
              </span>
            </Hint>
          ))}
        </div>
        <div className="flex gap-xs text-micro text-muted-foreground">{cost.days.map((day) => <span key={day.date} className="flex-1 text-center font-mono">{day.date.slice(5)}</span>)}</div>
      </section>
      <div className="grid grid-cols-2 gap-md">
        <AmountList title={t("projectTool.cost.models")} rows={cost.models.map((row) => ({ key: `${row.agent}:${row.model ?? ""}`, name: row.model ?? t("projectTool.cost.unnamedModel"), note: agentProductName(row.agent), microUsd: row.micro_usd }))} kind="models" />
        <AmountList title={t("projectTool.cost.agents")} rows={cost.agents.map((row) => ({ key: row.agent, name: agentProductName(row.agent), note: null, microUsd: row.micro_usd }))} kind="agents" />
      </div>
      <section className="@container/task-table space-y-sm" data-task-table={table.filter}>
        <ToggleGroup type="single" value={table.filter} onValueChange={(value) => { if (value) useUiStore.getState().showTaskTable(value as TaskFilter, null); }} aria-label={t("projectTool.cost.title", { project: project.label })} className="flex-wrap justify-start">
          {TASK_FILTERS.map((filter) => (
            <ToggleGroupItem key={filter} value={filter} size="sm" data-task-filter={filter}>
              {t(`projectTool.table.filter.${filter}`)} <span className="tabular-nums text-muted-foreground">{counts[filter]}</span>
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        {rows.length === 0 ? (
          <p className="flex items-center gap-sm text-muted-foreground" data-task-table-empty={table.filter}>
            {t("projectTool.table.emptyFilter")}
            <Button variant="text" size="inline" onClick={() => useUiStore.getState().showTaskTable("all", null)}>{t("projectTool.costly.all")}</Button>
          </p>
        ) : (
          <div role="table" className="flex min-w-0 flex-col" data-task-rows={rows.length}>
            <div role="row" className="flex min-w-0 items-center gap-sm border-b border-border py-xs text-micro text-muted-foreground">
              <span role="columnheader" className="min-w-0 flex-1">{t("projectTool.table.task")}</span>
              <span role="columnheader" className={RESULT_COLUMN}>{table.filter === "none" ? t("projectTool.table.lastLine") : t("projectTool.table.result")}</span>
              <span role="columnheader" className={NUMBER_COLUMN}>{t("projectTool.table.cost")}</span>
              <span role="columnheader" className={cn(NUMBER_COLUMN, WIDE_COLUMN)}>{t("projectTool.table.tokens")}</span>
              <span role="columnheader" className={NUMBER_COLUMN}>{t("projectTool.table.elapsed")}</span>
              <span role="columnheader" className={cn(NUMBER_COLUMN, WIDE_COLUMN)}>{t("projectTool.table.worked")}</span>
            </div>
            {rows.map((task) => <TaskRow key={task.id} task={task} project={project} noResult={table.filter === "none"} open={table.task === task.id} actions={actions} />)}
          </div>
        )}
        {cost.tasks_total > cost.tasks.length ? <p className="text-micro text-muted-foreground" data-task-over-cap={cost.tasks_total - cost.tasks.length}>{t("projectTool.table.overCap", { count: cost.tasks_total - cost.tasks.length })}</p> : null}
      </section>
      {hideAi > 0 ? <HideAiLine microUsd={hideAi} /> : null}
    </div>
  );
}

const RESULT_COLUMN = "w-(--size-pr-slot) shrink-0 truncate";
const NUMBER_COLUMN = "w-(--size-task-column) shrink-0 truncate text-right";
/** Tokens and working time stand as columns in a wide table and under the task's name in a narrow one (B35). */
const WIDE_COLUMN = "hidden task-table-wide:block";

function HideAiLine({ microUsd }: { microUsd: number }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  return <p className="text-micro text-muted-foreground" data-cost-hide-ai={microUsd}>{t("projectTool.cost.hideAi", { cost: usd(microUsd, language) })}</p>;
}

function AmountList({ title, rows, kind }: { title: string; rows: { key: string; name: string; note: string | null; microUsd: number }[]; kind: string }) {
  const { i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  if (rows.length === 0) return <span />;
  return (
    <section className="min-w-0 space-y-xs">
      <h3 className="font-medium text-subtle-foreground">{title}</h3>
      <ul className="flex flex-col" data-cost-amounts={kind}>
        {rows.map((row) => (
          <li key={row.key} className="flex min-w-0 items-center gap-sm border-b border-border py-xs">
            <span className="min-w-0 flex-1 truncate" title={row.note ? `${row.name} · ${row.note}` : row.name}><span className="text-foreground">{row.name}</span>{row.note ? <span className="text-muted-foreground"> · {row.note}</span> : null}</span>
            <span className="shrink-0 font-mono tabular-nums">{usd(row.microUsd, language)}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

/**
 * One task of the table (B27-B32): its name, result (or, under 결과 없음,
 * what was said last with 대화 보기), cost, tokens and times; pressed, its
 * sessions under their parents or its mission's Conductor and nodes.
 */
function TaskRow({ task, project, noResult, open: openFirst, actions }: { task: TaskCost; project: Workspace; noResult: boolean; open: boolean; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const [open, setOpen] = useState(openFirst);
  const agents = useShellStore((s) => allAgents(s.agents, s.rest?.status?.remote));
  const links = useShellStore((s) => s.linkSummaries);
  const name = taskName(task, agents, links, project.id) ?? t("projectTool.cost.session", { agent: agentProductName(task.root.agent) });
  const conversation = () => {
    const live = agents.find((row) => row.session_id === task.root.session_id);
    if (live) return actions.openAgent(live.pane_id);
    actions.openSessionRecord({ device_id: project.device_id, agent: task.root.agent, session_id: task.root.session_id });
  };
  return (
    <div role="rowgroup" className="min-w-0 border-b border-border" data-task-row={task.id} data-task-kind={task.kind}>
      <div role="row" className="relative flex min-w-0 items-center gap-sm py-xs">
        <RowOverlayButton aria-label={t("projectTool.table.expand", { name })} aria-expanded={open} onClick={() => setOpen(!open)} />
        <span role="cell" className="pointer-events-none relative flex min-w-0 flex-1 items-center gap-xxs">
          <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon-sm) shrink-0 text-muted-foreground", open && "rotate-90")} />
          <span className="flex min-w-0 flex-col">
            <span className="min-w-0 truncate text-foreground" title={name}>{name}</span>
            <span className="truncate text-micro text-muted-foreground task-table-wide:hidden">{t("projectTool.week.tokens", { tokens: tokenWords(language, task.tokens) })} · {t("projectTool.week.worked", { time: durationWords(language, task.worked_ms) })}</span>
          </span>
        </span>
        <span role="cell" className={cn(RESULT_COLUMN, "relative flex items-center gap-xs", noResult ? "pointer-events-auto" : "pointer-events-none")}>
          {noResult ? (
            <>
              <span className="min-w-0 flex-1 truncate text-micro text-muted-foreground" title={task.last_line ?? undefined}>{task.last_line ?? ""}</span>
              <Button variant="text" size="inline" className="shrink-0 text-micro" onClick={conversation} data-task-conversation={task.root.session_id}>{t("projectTool.table.conversation")}</Button>
            </>
          ) : <TaskResultMark task={task} projectId={project.id} />}
        </span>
        <span role="cell" className={cn(NUMBER_COLUMN, "pointer-events-none relative font-mono tabular-nums text-foreground")}>{task.unpriced ? "≥ " : ""}{usd(task.micro_usd, language)}</span>
        <span role="cell" className={cn(NUMBER_COLUMN, WIDE_COLUMN, "pointer-events-none relative tabular-nums text-muted-foreground")}>{tokenWords(language, task.tokens)}</span>
        <span role="cell" className={cn(NUMBER_COLUMN, "pointer-events-none relative tabular-nums text-muted-foreground")}>{durationWords(language, task.elapsed_ms)}</span>
        <span role="cell" className={cn(NUMBER_COLUMN, WIDE_COLUMN, "pointer-events-none relative tabular-nums text-muted-foreground")}>{durationWords(language, task.worked_ms)}</span>
      </div>
      {open ? (task.kind === "mission" && task.mission ? <MissionNodes mission={task.mission} /> : <TaskSessions task={task} project={project} />) : null}
    </div>
  );
}

function TaskSessions({ task, project }: { task: TaskCost; project: Workspace }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const agents = useShellStore((s) => allAgents(s.agents, s.rest?.status?.remote));
  const links = useShellStore((s) => s.linkSummaries);
  return (
    <ul className="flex min-w-0 flex-col pb-xs" data-task-sessions={task.sessions.length}>
      {task.sessions.map((session) => {
        const name = sessionName(session.session_id, agents, links, project.id) ?? t("projectTool.cost.session", { agent: agentProductName(session.agent) });
        return (
          <li key={session.session_id} className="flex min-w-0 items-center gap-sm py-xxs text-micro" style={{ paddingInlineStart: `calc(var(--spacing-md) * ${session.depth + 1})` }} data-task-session={session.session_id} data-task-depth={session.depth}>
            <span className="min-w-0 flex-1 truncate text-foreground" title={name}>{name} <span className="text-muted-foreground">· {agentProductName(session.agent)}</span></span>
            <span className={cn(NUMBER_COLUMN, "font-mono tabular-nums")}>{session.unpriced && session.micro_usd === 0 ? t("projectTool.cost.noPrice") : `${session.unpriced ? "≥ " : ""}${usd(session.micro_usd, language)}`}</span>
            <span className={cn(NUMBER_COLUMN, WIDE_COLUMN, "tabular-nums text-muted-foreground")}>{tokenWords(language, session.tokens)}</span>
            <span className={cn(NUMBER_COLUMN, "tabular-nums text-muted-foreground")}>{durationWords(language, session.worked_ms)}</span>
          </li>
        );
      })}
      {task.sessions_total > task.sessions.length ? <li className="py-xxs ps-md text-micro text-muted-foreground">{t("projectTool.table.moreSessions", { count: task.sessions_total - task.sessions.length })}</li> : null}
    </ul>
  );
}

function MissionNodes({ mission }: { mission: TaskMission }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  return (
    <ul className="flex min-w-0 flex-col pb-xs" data-mission-nodes={mission.nodes.length}>
      {mission.nodes.map((node) => (
        <li key={node.key} className="flex min-w-0 items-center gap-sm py-xxs ps-md text-micro" data-mission-node={node.key}>
          <span className="min-w-0 flex-1 truncate text-foreground" title={node.title}>{node.title}</span>
          <span className="shrink-0 text-muted-foreground">{node.pr != null ? `#${node.pr}` : t("projectTool.table.noPr")}</span>
          {node.attempts > 1 ? <span className="shrink-0 text-muted-foreground">{t("projectTool.table.attempts", { count: node.attempts - 1 })}</span> : null}
          {node.reopened > 0 ? <span className="shrink-0 text-warning">{t("projectTool.table.reopened", { count: node.reopened })}</span> : null}
          <span className={cn(NUMBER_COLUMN, "font-mono tabular-nums")}>{usd(node.micro_usd, language)}</span>
          <span className={cn(NUMBER_COLUMN, "tabular-nums text-muted-foreground")}>{durationWords(language, node.worked_ms)}</span>
        </li>
      ))}
    </ul>
  );
}

/**
 * 남은 것 (B9): what a worktree left, as its 흘린 것 row opened it: the
 * changed files not committed and the commits not pushed. The core reads the
 * worktree's changes for this view while it is on top (`changes_target`);
 * the commit list is the front checkout's header read, so another worktree
 * shows its committed files instead. A file of the front checkout opens its
 * diff.
 */
export function RemainsView({ workspaceId, checkoutId, actions }: { workspaceId: string; checkoutId: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const project = useProject(workspaceId);
  const checkout = project?.checkouts.find((row) => row.id === checkoutId) ?? null;
  const changes = useShellStore((s) => changesFor(s.changes, checkout?.path ?? null));
  const front = useShellStore((s) => frontCheckout(s.rest)?.id === checkoutId);
  const git = useShellStore((s) => (front ? (s.rest?.workspace_view as WorkspaceView | undefined)?.git ?? null : null));
  if (!project || !checkout) return <Missing />;
  if (!changes) return <div role="status" aria-label={t("projectTool.remains.reading")} className="m-md h-(--size-control-lg) rounded-sm bg-muted" data-remains-view="pending" />;
  const unpushed = git?.facts?.recent_commits?.filter((commit) => git.facts?.unpushed_commits?.includes(commit.sha)) ?? null;
  const open = (path: string, committed: boolean) => front ? actions.selectChange(path, committed, true) : undefined;
  const empty = changes.entries.length === 0 && (unpushed ? unpushed.length === 0 : changes.committed.length === 0);
  return (
    <div className="min-h-0 flex-1 space-y-md overflow-y-auto px-md py-md text-caption" data-remains-view={checkout.id}>
      {empty ? <p className="text-muted-foreground">{t("projectTool.remains.gone")}</p> : null}
      {changes.entries.length > 0 ? <FileList title={t("projectTool.remains.changes")} files={changes.entries} onOpen={front ? (path) => open(path, false) : null} kind="changes" /> : null}
      {unpushed ? (
        unpushed.length > 0 ? (
          <section className="space-y-xs" data-remains-commits={unpushed.length}>
            <h3 className="font-medium text-subtle-foreground">{t("projectTool.remains.commits")}</h3>
            <ul className="flex flex-col">
              {unpushed.map((commit) => <li key={commit.sha} className="flex min-w-0 items-center gap-sm border-b border-border py-xs"><span className="shrink-0 font-mono text-micro text-muted-foreground">{commit.sha.slice(0, 7)}</span><span className="min-w-0 flex-1 truncate text-foreground" title={commit.subject}>{commit.subject}</span></li>)}
            </ul>
          </section>
        ) : null
      ) : changes.committed.length > 0 ? <FileList title={t("projectTool.remains.commits")} files={changes.committed} onOpen={null} kind="committed" /> : null}
    </div>
  );
}

function FileList({ title, files, onOpen, kind }: { title: string; files: ChangedFileSnapshot[]; onOpen: ((path: string) => void) | null; kind: string }) {
  return (
    <section className="space-y-xs" data-remains-files={kind}>
      <h3 className="font-medium text-subtle-foreground">{title} <span className="tabular-nums text-muted-foreground">{files.length}</span></h3>
      <ul className="flex flex-col">
        {files.map((file) => {
          const content = (
            <>
              <span className="min-w-0 flex-1 truncate font-mono text-foreground" title={file.relative_path}>{file.relative_path}</span>
              {file.added_lines != null ? <span className="shrink-0 font-mono text-micro text-success">+{file.added_lines}</span> : null}
              {file.removed_lines != null ? <span className="shrink-0 font-mono text-micro text-destructive">-{file.removed_lines}</span> : null}
            </>
          );
          return (
            <li key={file.path} className="min-w-0 border-b border-border" data-remains-file={file.relative_path}>
              {onOpen ? <RowButton onClick={() => onOpen(file.path)}>{content}</RowButton> : <span className="flex min-w-0 items-center gap-xs px-xs py-xs">{content}</span>}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
