import { AgentRecordTitle } from "./components/agent-record-title";
import { ChevronDownIcon, ChevronRightIcon, EllipsisIcon, FileTextIcon, GitBranchIcon, PlusIcon, ServerIcon, StarIcon } from "lucide-react";
import { Fragment, memo, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import type { TFunction } from "i18next";
import type { Actions } from "./actions";
import { deviceScope } from "./agentScope";
import { AgentRowMark } from "./AgentRowMark";
import { useAgentLogosShown } from "./agentLogoMode";
import { agentProductName } from "./agentAdapters";
import { backgroundTail, recordWords, forkLabel, leadWord, lineParts, markTone, markWords, rowDimmed, rowLine, rowSentence, stateLine, type LineParts } from "./agentRow";
import { AgentForkMark } from "./components/fork-mark";
import { RevisitStar, starred } from "./components/revisit-mark";
import { askParts, DescendantMark, TreeChevron, TreeRails, type TreePlace } from "./components/agent-tree";
import { childrenOf } from "./components/agent-tree-popover";
import { Elapsed } from "./components/elapsed";
import { AgentPrMark, useAgentStaleness } from "./components/pr-mark";
import { MissionChip } from "./components/mission-chip";
import { RowLineText } from "./components/row-line-text";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { Keycap } from "./components/ui/keycap";
import { EntryContextMenu, EntryDropdown, type MenuEntry } from "./components/entry-menu";
import { PartsMark } from "./components/pr-mark";
import { archivedProject, archivedRows, archiveOffer } from "./archive";
import { SidebarProjectFilter, useSidebarProject } from "./SidebarProjectFilter";
import { prMarkParts } from "./prMark";
import { Hint } from "./components/ui/tooltip";
import { deviceConnected, frontDeviceId } from "./devices";
import { hideAiWords, homeTurnLine } from "./hideAiOrigin";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { numberedPanes, numberOf } from "./numbering";
import { allAgents as everyPaneAgent, agentsByPane } from "./paneAgents";
import { familyPulls, type SessionPull } from "./prMark";
import { remoteTargetOfPane } from "./remote";
import { machineName } from "./screenMachine";
import { checkoutPathOfPane, sessionTreeLines } from "./sessionTree";
import { sessionOfAgentOn } from "./sessionWork";
import { commandLabel } from "./shortcutLabels";
import { revealRow, sidebarFocusPane } from "./sidebarFocus";
import type { SidebarTreeLine } from "./sidebarTree";
import { catalogWorkspaces, localDeviceId, SIDEBAR_LENSES, type AgentRow, type TaskView } from "./snapshot";
import { useShellStore } from "./store";
import { drawnTasks, TASK_GROUPS, taskFoldKey, taskList, taskSections, type ArchivedEntry, type TaskEntry, type TaskGroup, type TaskGroupState } from "./taskList";
import { useUiStore } from "./ui";
import type { TaskState } from "./agentScope";

const NO_FOLDS: string[] = [];
const NO_PANES: ReadonlySet<string> = new Set();
/** A row's second line by the task's state: a question and a block are the operator's turn, both in the warning colour (B8; status-marks B32). */
const LINE_TONE: Partial<Record<TaskState, string>> = { needs_you: "text-warning", blocked: "text-warning" };

/**
 * The sidebar's 프로젝트 | 태스크 switch (PRD sidebar-task-lens B1, B39):
 * under the head, right above the list. ⇧⌘B turns it.
 */
export function SidebarLensSwitch({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const lens = useShellStore((s) => s.rest?.ui_state?.sidebar_lens ?? "projects");
  const chord = useShellStore((s) => commandLabel("toggle_sidebar_lens", s.rest?.ui_state));
  return (
    <div className="shrink-0 px-sm pt-xs pb-xxs">
      <Hint label={t("taskLens.switch")} shortcut={chord || null}>
        <div role="group" aria-label={t("taskLens.switch")} className="flex min-w-0 rounded-sm bg-muted p-xxs" data-sidebar-lens={lens}>
          {SIDEBAR_LENSES.map((id) => (
            <button key={id} type="button" aria-pressed={lens === id} data-sidebar-lens-option={id}
              className={cn("flex min-w-0 flex-1 items-center justify-center rounded-xs px-sm py-xxs text-caption outline-none focus-visible:ring-1 focus-visible:ring-ring", lens === id ? "bg-background font-medium text-foreground shadow-xs" : "text-subtle-foreground hover:text-foreground")}
              onClick={() => lens !== id && actions.setSidebarLens(id)}>
              <span className="truncate">{t(`taskLens.lens.${id}`)}</span>
            </button>
          ))}
        </div>
      </Hint>
    </div>
  );
}

/**
 * The 태스크 list (B4-B22): every task of the rail's device, grouped by state
 * or by project. A row stands for a task's root agent; its descendants open
 * under it. The row of the pane with the keyboard is selected, as in the
 * 프로젝트 tree.
 */
export function TaskLens({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((s) => s.rest);
  const localAgents = useShellStore((s) => s.agents);
  const deviceId = frontDeviceId(rest);
  // The project chip, or a 현황 tile, narrows the list to a project, read from that project's own scope (PRD lane-two-columns B27).
  const filter = useSidebarProject(deviceId);
  const scope = filter ? (catalogWorkspaces(rest).find((row) => row.id === filter.workspace_id && row.device_id === deviceId)?.agent_scope ?? null) : deviceScope(rest, deviceId);
  const view: TaskView = rest?.ui_state?.task_view ?? "status";
  const folds = rest?.ui_state?.sessions_expanded_agent_pane_ids ?? NO_FOLDS;
  const connected = deviceConnected(rest, deviceId);
  const inWorkspace = useUiStore((s) => s.screen?.kind === "workspace" && !s.overviewOpen);
  const focusedPane = useShellStore((s) => s.focusedPaneId);
  // Lineage crosses devices, so a row's children are read from every device's agents.
  const agents = useMemo(() => everyPaneAgent(localAgents, rest?.status?.remote), [localAgents, rest?.status?.remote]);
  const byPane = useMemo(() => agentsByPane(agents), [agents]);
  const projects = useMemo(() => catalogWorkspaces(rest), [rest]);
  const archived = useMemo(() => {
    const rows = archivedRows(rest, deviceId);
    return filter ? rows.filter((row) => archivedProject(row, catalogWorkspaces(rest))?.id === filter.workspace_id) : rows;
  }, [rest, deviceId, filter]);
  const list = useMemo(() => taskList(scope?.tasks, byPane, projects, view, archived), [scope?.tasks, byPane, projects, view, archived]);
  const noProject = rest !== null && projects.every((project) => project.is_home || project.device_id !== deviceId);
  const unresponsive = rest !== null && !connected ? (rest.navigator?.devices?.find((device) => device.id === deviceId) ? machineName(rest.navigator?.devices, deviceId, t) : deviceId) : null;
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-task-lens={view}>
      <div className="flex shrink-0 items-center gap-xs px-md py-xs text-micro text-muted-foreground">
        <SidebarProjectFilter deviceId={deviceId} actions={actions} />
        <span className="min-w-0 flex-1 truncate" data-task-count={scope ? list.total : undefined}>{scope ? t("taskLens.count", { count: list.total }) : t("taskLens.countLoading")}</span>
        <ViewSwitch view={view} actions={actions} />
      </div>
      {unresponsive ? (
        <p role="status" className="flex shrink-0 items-center gap-xs px-md pb-xs text-micro text-muted-foreground" data-task-unresponsive={deviceId}>
          <ServerIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
          <span className="min-w-0 truncate">{t("taskLens.unresponsive", { device: unresponsive })}</span>
        </p>
      ) : null}
      {rest === null || !scope ? <TaskSkeleton /> : noProject ? <NoProject actions={actions} /> : list.total === 0 && list.groups.length === 0 && list.resting.length === 0 && list.cleanup.length === 0 && list.archived.length === 0 ? <NoTasks actions={actions} /> : (
        <TaskGroups list={list} view={view} showProject={view === "status" && !filter} byPane={byPane} agents={agents} folds={folds} focusedPane={inWorkspace ? focusedPane : null} disabled={!connected} actions={actions} />
      )}
    </div>
  );
}

function ViewSwitch({ view, actions }: { view: TaskView; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return (
    <div role="group" aria-label={t("taskLens.view")} className="flex shrink-0 rounded-xs border border-border p-px" data-task-view={view}>
      {(["status", "project"] as const).map((id) => (
        <button key={id} type="button" aria-pressed={view === id} data-task-view-option={id}
          className={cn("rounded-xs px-xs py-px text-micro outline-none focus-visible:ring-1 focus-visible:ring-ring", view === id ? "bg-secondary font-medium text-foreground" : "text-subtle-foreground hover:text-foreground")}
          onClick={() => view !== id && actions.setTaskView(id)}>
          {t(`taskLens.view.${id}`)}
        </button>
      ))}
    </div>
  );
}

/** The first read (B20): a few quiet bars, no number. */
function TaskSkeleton() {
  const { t } = useInterfaceTranslation();
  return (
    <div role="status" aria-label={t("taskLens.loading")} className="flex flex-col gap-md px-md py-sm" data-task-loading="true">
      {[0, 1, 2, 3].map((key) => (
        <span key={key} className="flex flex-col gap-xs">
          <span className="h-xs w-3/4 rounded-full bg-muted" />
          <span className="h-xs w-1/2 rounded-full bg-muted" />
        </span>
      ))}
    </div>
  );
}

function NoTasks({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex flex-col items-start gap-sm px-md py-md text-caption text-muted-foreground" data-tasks-empty="true">
      <p>{t("taskLens.empty")}</p>
      <Button variant="outline" size="sm" onClick={() => actions.openStartPanel()} data-tasks-new-agent="true"><PlusIcon />{t("taskLens.newAgent")}</Button>
    </div>
  );
}

/** No project on the device (B42): the first step, with Add project and its chord where the host has one. */
function NoProject({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const chord = commandLabel("new_workspace");
  return (
    <div className="flex flex-col items-start gap-sm px-md py-md text-caption text-muted-foreground" data-tasks-no-project="true">
      <p>{t("firstRun.home.noProject")}</p>
      <Hint label={t("taskLens.addProject")} shortcut={chord || null}>
        <Button variant="outline" size="sm" onClick={() => actions.openAddProject()} data-tasks-add-project="true"><PlusIcon />{t("taskLens.addProject")}</Button>
      </Hint>
    </div>
  );
}

const TaskGroups = memo(function TaskGroups({ list, view, showProject, byPane, agents, folds, focusedPane, disabled, actions }: {
  list: ReturnType<typeof taskList>; view: TaskView; showProject: boolean; byPane: ReadonlyMap<string, AgentRow>; agents: readonly AgentRow[]; folds: string[]; focusedPane: string | null; disabled: boolean; actions: Actions;
}) {
  const { t } = useInterfaceTranslation();
  const [shownAll, setShownAll] = useState<ReadonlySet<string>>(NO_PANES);
  // The resting and cleanup folds open in place for this page only (B5); the
  // page keeps them, so the ⌥n numbers read the same drawn order.
  const foldsOpen = useUiStore((s) => s.taskFoldsOpen);
  const toggleFold = useUiStore((s) => s.toggleTaskFold);
  const focusedRestFold = useMemo(() => {
    if (focusedPane === null) return null;
    const containsFocus = ({ card }: TaskEntry) => card.members.includes(focusedPane);
    if (view === "status") return list.resting.some(containsFocus) ? taskFoldKey("rest", "") : null;
    const group = list.groups.find((group) => group.resting.some(containsFocus));
    return group ? taskFoldKey("rest", group.key) : null;
  }, [focusedPane, list, view]);
  // Entering a resting task reveals it, including a focused descendant.
  // Read the current folds only on entry or a change of the containing fold:
  // an explicit collapse must survive snapshots while focus stays here.
  useLayoutEffect(() => {
    if (focusedRestFold !== null && !useUiStore.getState().taskFoldsOpen.includes(focusedRestFold)) toggleFold(focusedRestFold);
  }, [focusedPane, focusedRestFold, toggleFold]);
  const openFolds = useMemo(() => new Set(foldsOpen), [foldsOpen]);
  const opened = useMemo(() => new Set(folds), [folds]);
  const shown = taskSections(list, view, openFolds);
  // An ⌥ hold numbers the task rows top to bottom, nine at most, and ⌥n opens that row (electron-digit-shortcuts-hints D-02).
  const numbered = useUiStore((s) => s.hint === "agents");
  const numbers = numbered ? numberedPanes(drawnTasks(shown).map(({ agent }) => agent.pane_id)) : null;
  const treesOf = (entries: TaskEntry[]) => entries.map((entry) => ({ entry, lines: sessionTreeLines(entry.agent, byPane, opened, shownAll) }));
  const sections = shown.sections.map(({ group, resting, cleanup, archived }) => ({ group, bands: group.bands.map((band) => ({ state: band.state, trees: treesOf(band.entries) })), resting: treesOf(resting), cleanup: treesOf(cleanup), archived }));
  const restingTrees = treesOf(shown.resting);
  const cleanupTrees = treesOf(shown.cleanup);
  const drawn = new Set([...sections.flatMap(({ bands, resting, cleanup }) => [...bands.flatMap((band) => band.trees), ...resting, ...cleanup]), ...restingTrees, ...cleanupTrees]
    .flatMap(({ lines }) => lines.flatMap((line) => (line.kind === "agent" ? [line.row.agent.pane_id] : []))));
  const selected = sidebarFocusPane(focusedPane, agents, drawn);
  // The selected row comes into view when the row standing for the focus
  // changes, and never while it stays the one revealed, so a list the
  // operator scrolled stays where they left it (B13).
  const scroller = useRef<HTMLDivElement>(null);
  const revealed = useRef<string | null>(null);
  useLayoutEffect(() => {
    if (selected === null) { revealed.current = null; return; }
    const row = scroller.current?.querySelector<HTMLElement>('[data-selected="true"]');
    if (selected === revealed.current || !scroller.current || !row) return;
    revealed.current = selected;
    revealRow(scroller.current, [row], row);
  }, [selected]);
  const showAll = (parent: string) => setShownAll((before) => new Set(before).add(parent));
  const rows = (trees: ReturnType<typeof treesOf>) => trees.map(({ entry, lines }) => (
    <TaskTree key={entry.card.root} entry={entry} lines={lines} byPane={byPane} opened={opened} selected={selected} number={numbers ? numberOf(numbers, entry.agent.pane_id) : null} showProject={showProject} disabled={disabled} actions={actions} onShowAll={showAll} />
  ));
  const archivedList = (entries: ArchivedEntry[]) => entries.map((entry) => <ArchivedRow key={entry.row.id} entry={entry} showProject={showProject} disabled={disabled} actions={actions} />);
  return (
    <div ref={scroller} className={cn("min-h-0 flex-1 overflow-y-auto px-sm pb-md", disabled && "opacity-(--opacity-disabled)")} data-task-list="true" onKeyDown={moveFocus}>
      {sections.map(({ group, bands, resting, cleanup, archived }) => (
        <section key={group.key} data-task-group={group.state ?? group.project?.id ?? ""} aria-label={groupName(group, t)}>
          <GroupHead group={group} />
          {bands.map((band) => (
            <Fragment key={band.state}>
              {view === "project" ? <BandHead state={band.state} /> : null}
              <ul className="flex min-w-0 flex-col" aria-label={view === "project" ? t(`taskLens.group.${band.state}`) : undefined} data-task-band-list={view === "project" ? band.state : undefined}>{rows(band.trees)}</ul>
            </Fragment>
          ))}
          {view === "project" ? <Folds group={group} resting={resting} cleanup={cleanup} archived={archived} open={openFolds} onToggle={toggleFold} rows={rows} archivedRows={archivedList} /> : null}
        </section>
      ))}
      {view === "status" ? <Folds group={{ key: "", resting: list.resting, cleanup: list.cleanup, archived: list.archived }} resting={restingTrees} cleanup={cleanupTrees} archived={shown.archived} open={openFolds} onToggle={toggleFold} rows={rows} archivedRows={archivedList} /> : null}
    </div>
  );
}, (before, after) => before.list === after.list && before.view === after.view && before.showProject === after.showProject && before.byPane === after.byPane && before.folds === after.folds && before.focusedPane === after.focusedPane && before.disabled === after.disabled && before.actions === after.actions);

function groupName(group: TaskGroup, t: ReturnType<typeof useInterfaceTranslation>["t"]): string {
  return group.project?.label ?? (group.state ? t(`taskLens.group.${group.state}`) : "");
}

/**
 * The small state label over each band of a 프로젝트별 group, where its rows
 * change state (status-marks B32): grey like the folds, 내 차례 in the warning
 * colour, out of the arrow keys' way; the band's list carries the name.
 */
function BandHead({ state }: { state: TaskGroupState }) {
  const { t } = useInterfaceTranslation();
  return (
    <div aria-hidden="true" data-task-band={state} className={cn("px-xs pt-xs pb-xxs text-micro text-muted-foreground", state === "needs_you" && "text-warning")}>
      {t(`taskLens.group.${state}`)}
    </div>
  );
}

/** A group's head: its state and count, or a project's name with its counts by state (B4, B6). */
function GroupHead({ group }: { group: TaskGroup }) {
  const { t } = useInterfaceTranslation();
  if (group.state) {
    return (
      <h3 tabIndex={-1} data-task-focus="group" className={cn("flex items-center gap-xs rounded-xs px-xs pt-sm pb-xs text-micro font-medium text-subtle-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring", group.state === "needs_you" && "text-warning")} data-task-group-head={group.state}>
        <span>{t(`taskLens.group.${group.state}`)}</span>
        <span className="tabular-nums">{group.entries.length}</span>
      </h3>
    );
  }
  const counts = TASK_GROUPS.filter((state) => group.counts[state]);
  return (
    <h3 tabIndex={-1} data-task-focus="group" className="flex min-w-0 items-center gap-sm rounded-xs px-xs pt-sm pb-xs text-caption font-semibold text-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring" data-task-group-head="project">
      <span className="min-w-0 flex-1 truncate" title={group.project?.path}>{group.project?.label ?? ""}</span>
      {counts.map((state: TaskGroupState) => (
        <span key={state} className={cn("shrink-0 text-micro font-normal", state === "needs_you" ? "text-warning" : "text-muted-foreground")}>
          {t(`taskLens.group.${state}`)} {group.counts[state]}
        </span>
      ))}
    </h3>
  );
}

/** `쉬는 중 N ›`, `정리할 것 N ›` and, last, `보관됨 N ›` (B5; PRD session-archive B14, B15): folded, opened in place, closed with `접기`. */
function Folds({ group, resting, cleanup, archived, open, onToggle, rows, archivedRows: drawArchived }: {
  group: Pick<TaskGroup, "key" | "resting" | "cleanup" | "archived">;
  resting: { entry: TaskEntry; lines: SidebarTreeLine[] }[];
  cleanup: { entry: TaskEntry; lines: SidebarTreeLine[] }[];
  archived: ArchivedEntry[];
  open: ReadonlySet<string>;
  onToggle: (key: string) => void;
  rows: (trees: { entry: TaskEntry; lines: SidebarTreeLine[] }[]) => ReactNode;
  archivedRows: (entries: ArchivedEntry[]) => ReactNode;
}) {
  const { t } = useInterfaceTranslation();
  const fold = (kind: "rest" | "cleanup" | "archived", count: number, drawn: ReactNode) => {
    if (count === 0) return null;
    const key = taskFoldKey(kind, group.key);
    const expanded = open.has(key);
    return (
      <div data-task-fold={kind}>
        <button type="button" data-task-focus="fold" aria-expanded={expanded} onClick={() => onToggle(key)}
          className="flex w-full items-center gap-xs rounded-xs px-xs py-xs text-left text-micro text-muted-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring">
          {expanded ? <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
          <span>{t(kind === "rest" ? "taskLens.fold.resting" : kind === "cleanup" ? "taskLens.fold.cleanup" : "archive.fold", { count })}</span>
        </button>
        {expanded ? <>
          <ul className="flex min-w-0 flex-col">{drawn}</ul>
          <button type="button" data-task-focus="fold-close" onClick={() => onToggle(key)} className="ml-lg rounded-xs px-xs py-xxs text-micro text-muted-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring">{t("taskLens.fold.close")}</button>
        </> : null}
      </div>
    );
  };
  return <>{fold("rest", group.resting.length, rows(resting))}{fold("cleanup", group.cleanup.length, rows(cleanup))}{fold("archived", group.archived.length, drawArchived(archived))}</>;
}

/** A task's row and, through every opened row, its descendants (B10). */
function TaskTree({ entry, lines, byPane, opened, selected, number, showProject, disabled, actions, onShowAll }: { entry: TaskEntry; lines: SidebarTreeLine[]; byPane: ReadonlyMap<string, AgentRow>; opened: ReadonlySet<string>; selected: string | null; number: number | null; showProject: boolean; disabled: boolean; actions: Actions; onShowAll: (parent: string) => void }) {
  const { agent } = entry;
  const family = useMemo(() => familyPulls(agent, byPane), [agent, byPane]);
  const openOf = (of: AgentRow) => (childrenOf(of, byPane).length > 0 ? opened.has(of.pane_id) : null);
  return <>
    <TaskRow entry={entry} family={family} open={openOf(agent)} selected={selected === agent.pane_id} number={number} showProject={showProject} disabled={disabled} actions={actions} />
    {lines.slice(1).map((line) => {
      if (line.kind === "more") return <TreeMoreRow key={`more:${line.parent}`} line={line} onShowAll={onShowAll} />;
      const child = line.row.agent;
      const parent = (child.lineage_parent_pane_id ? byPane.get(child.lineage_parent_pane_id) : undefined) ?? agent;
      return <ChildRow key={child.pane_id} parent={parent} child={child} depth={line.row.depth} place={line.place} open={openOf(child)} selected={selected === child.pane_id} disabled={disabled} actions={actions} workspace={entry.project?.id ?? null} />;
    })}
  </>;
}

/** A task's row; `number` is the digit an ⌥ hold shows on it, floating at its top right over the time, moving nothing. */
function TaskRow({ entry, family, open, selected, number, showProject, disabled, actions }: { entry: TaskEntry; family?: SessionPull[]; open: boolean | null; selected: boolean; number: number | null; showProject: boolean; disabled: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { agent, card, project } = entry;
  const staleness = useAgentStaleness(agent, family);
  // A primitive, so a snapshot that moves nothing of this row leaves it alone.
  const localId = useShellStore((s) => localDeviceId(s.rest));
  const session = useMemo(() => sessionOfAgentOn(agent, localId), [agent, localId]);
  const go = () => { if (!disabled) actions.openListedAgent(agent.pane_id); };
  const record = session ? () => actions.openSessionRecord(session) : null;
  const toggle = () => actions.toggleSessionTree(agent.pane_id);
  // The ⋯ and a right-click on the row open the same menu (PRD session-archive B1, D-27).
  // The core refuses to resolve work that runs or waits, or an approval prompt (`sessions::resolve_refusal`), so the menu does not offer it.
  const resolvable = !agent.resolved && agent.activity !== "working" && agent.group !== "working" && agent.demand !== "approval";
  const menu = useTaskMenu(agent, record, go, resolvable, disabled, actions, true);
  // The row's own mark words, as the sidebar row says them (status-marks B3, B33); a fold's rows say the fold.
  // A starred task waiting in 다시 볼 것 wears the ★ in its mark's slot; a read ask keeps its own mark (PRD revisit-group B2, B6).
  // The small ★ after the title steps aside where the time's slot shows the ☆/★ button, so the row never draws two.
  const starMark = card.state === "revisit" && starred(agent) && !agent.revisit?.read_ask;
  const revisitWords = card.state === "revisit" ? t(starMark ? "taskLens.revisit.starred" : "taskLens.revisit.readAsk") : starred(agent) ? t("taskLens.revisit.starred") : null;
  const words = [card.state === "resting" || card.state === "cleanup" ? t(`taskLens.state.${card.state}`) : markWords(t, agent), revisitWords].filter(Boolean).join(" · ");
  const sentence = rowSentence(t, agent);
  // The visible second line follows the button and is read after it, so the name leaves the sentence to it.
  const name = [agent.identity_label, forkLabel(t, agent), agentProductName(agent.agent_kind), recordWords(t, agent), agent.record_summary, words, hideAiWords(t, agent.hide_ai)].filter(Boolean).join(", ");
  return (
    <EntryContextMenu asChild label={t("taskLens.more", { name: agent.identity_label })} title={agent.identity_label} items={menu.items} onSelect={menu.select} disabled={disabled} data-task-context={agent.pane_id}>
    <li className={cn("group/task relative flex min-w-0 items-stretch rounded-xs pr-xs", selected ? "bg-selected" : "hover:bg-accent focus-within:bg-accent", rowDimmed(agent) && "opacity-(--opacity-dimmed)")}
      data-task-row={agent.pane_id} data-task-state={card.state} data-selected={selected ? "true" : undefined} onClick={go}>
      <TreeChevron name={agent.identity_label} open={open} onToggle={toggle} hangs={false} disabled={disabled} data-task-chevron={agent.pane_id} />
      <div className="flex min-w-0 flex-1 flex-col gap-xxs py-xs pl-xs">
        <div className="flex min-h-(--size-sidebar-line) min-w-0 items-center gap-xs">
          <button type="button" data-task-focus="row" disabled={disabled} aria-current={selected ? "true" : undefined} aria-expanded={open ?? undefined} aria-label={name}
            title={[agent.identity_label, agentProductName(agent.agent_kind), recordWords(t, agent), agent.record_summary, words, sentence].filter(Boolean).join("\n")}
            onClick={(event) => { event.stopPropagation(); go(); }}
            onKeyDown={(event) => rowKey(event, open, toggle, record)}
            className="flex min-w-0 flex-1 items-center gap-xs text-left outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50">
            {starMark ? <RevisitStar slot /> : <StatusMark mark={agent.mark} className={markTone(agent)} />}
            <AgentRowMark kind={agent.agent_kind} hideAi={agent.hide_ai ? "card" : null} />
            <AgentRecordTitle agent={agent} className="text-caption font-medium" />
            <AgentForkMark agent={agent} />
            {starred(agent) && !starMark ? <RevisitStar className="group-hover/task:hidden group-focus-within/task:hidden hoverless:hidden" /> : null}
          </button>
          {open ? null : <DescendantMark agent={agent} />}
          <AgentPrMark agent={agent} family={family} staleness={staleness} disabled={disabled} onOpen={(pull) => actions.openSessionPullRequest({ workspace_id: project?.id ?? null, url: pull.url, number: pull.number })} onExternal={(pull) => actions.openLink(pull.url, true)} />
          <MissionChip agent={agent} onOpen={(id) => actions.openMission(id)} />
          <TimeSlot agent={agent} record={record} menu={menu} revisit={{ on: card.state === "revisit" || starred(agent), disabled, actions }} />
        </div>
        <SecondLine entry={entry} showProject={showProject} />
      </div>
      {number !== null ? <Keycap number={number} /> : null}
    </li>
    </EntryContextMenu>
  );
}

type ArchivedMenuId = "record" | "resume" | "check" | "delete";

/**
 * An archived session in `보관됨` (PRD session-archive B14-B18): its title,
 * the project in 상태별, its PR marks and how long ago it was archived, with
 * no status mark because nothing runs. Pressing it opens its record; ⋯ and a
 * right-click offer 기록 보기 · 이어가기 · 보관함에서 지우기…, and 상태 확인
 * while the core cannot say whether its close finished.
 */
function ArchivedRow({ entry, showProject, disabled, actions }: { entry: ArchivedEntry; showProject: boolean; disabled: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { row, project } = entry;
  const [root] = row.members;
  const [open, setOpen] = useState(false);
  const indent = detailIndent(useAgentLogosShown());
  // A row of a device that is gone keeps its record and its delete; resuming waits for the device (B13).
  const away = useShellStore((s) => row.device_id !== localDeviceId(s.rest) && !deviceConnected(s.rest, row.device_id));
  if (!root) return null;
  const record = () => actions.openSessionRecord(root.session);
  const pending = row.phase === "archiving" || row.phase === "deleting" || row.members.some((member) => member.resume === "queued" || member.resume === "starting");
  const failed = row.members.some((member) => member.resume === "failed");
  const state = row.phase === "deleting" ? t("archive.deleting") : row.phase === "archiving" ? t("archive.archiving") : row.phase === "unknown" ? t("archive.unknown")
    : row.members.some((member) => member.resume === "queued" || member.resume === "starting") ? t("archive.resuming") : failed ? t("archive.resumeFailed") : null;
  const parts = prMarkParts(row.members.flatMap((member) => member.pulls.map((pull) => ({ pull, state: pull.state }))));
  const items = (): MenuEntry<ArchivedMenuId>[] => [
    { id: "record", icon: "record", label: t("taskLens.menu.record"), unavailable: null, shortcut: "Space" },
    { id: "resume", icon: "resume", label: t("archive.resume"), unavailable: disabled || pending || away || row.phase === "unknown" ? "" : null },
    ...(row.phase === "unknown" ? [{ id: "check" as const, icon: "refresh" as const, label: t("workspace.checkStatus"), unavailable: disabled || away ? "" : null }] : []),
    { id: "delete", icon: "delete", label: t("archive.menu.delete"), unavailable: disabled || pending ? "" : null, separated: true, destructive: true },
  ];
  const select = (id: ArchivedMenuId) => {
    if (id === "record") return record();
    if (id === "resume") return actions.resumeArchived(row.id);
    if (id === "check") return actions.checkArchived(row.id);
    return actions.deleteArchived(row.id);
  };
  const label = t("taskLens.more", { name: root.title });
  return (
    <EntryContextMenu asChild label={label} title={root.title} items={items} onSelect={select} data-archived-context={row.id}>
      <li className={cn("group/task relative flex min-w-0 items-stretch rounded-xs pr-xs hover:bg-accent focus-within:bg-accent", (pending || away) && "opacity-(--opacity-dimmed)")}
        data-archived-row={row.id} data-archived-phase={row.phase} data-archived-resume={failed ? "failed" : pending ? "pending" : undefined} onClick={record}>
        <TreeChevron name={root.title} open={null} onToggle={() => {}} hangs={false} />
        <div className="flex min-w-0 flex-1 flex-col gap-xxs py-xs pl-xs">
          <div className="flex min-h-(--size-sidebar-line) min-w-0 items-center gap-xs">
            <button type="button" data-task-focus="row" aria-label={[root.title, agentProductName(root.agent_kind), t("archive.state"), row.starred ? t("taskLens.revisit.starred") : null, state].filter(Boolean).join(", ")}
              title={[root.title, agentProductName(root.agent_kind), root.last_line].filter(Boolean).join("\n")}
              onClick={(event) => { event.stopPropagation(); record(); }}
              onKeyDown={(event) => { if (event.key === " ") { event.preventDefault(); record(); } }}
              className="flex min-w-0 flex-1 items-center gap-xs text-left outline-none focus-visible:ring-1 focus-visible:ring-ring">
              <AgentRowMark kind={root.agent_kind} hideAi={null} />
              <span className="min-w-0 truncate text-caption font-medium text-muted-foreground">{root.title}</span>
              {row.starred ? <RevisitStar /> : null}
            </button>
            {parts ? <PartsMark parts={parts} /> : null}
            <MissionChip agent={{ session: root.session }} onOpen={(id) => actions.openMission(id)} />
            <span className="relative flex shrink-0 items-center" onClick={(event) => event.stopPropagation()}>
              <Elapsed since={row.archived_at_unix_ms} className="pointer-events-none text-micro text-muted-foreground tabular-nums hoverless:hidden" />
              <span data-row-actions="" className={cn(FLOATING_ACTIONS, revealed(open))}>
                <EntryDropdown label={label} title={root.title} items={items} onSelect={select} onOpenChange={setOpen} data-archived-menu={row.id}
                  trigger={
                    <button type="button" aria-label={label} data-archived-more={row.id} className="rounded-xs p-xxs text-muted-foreground outline-none hover:bg-secondary focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-secondary">
                      <EllipsisIcon className="size-(--size-icon-sm)" />
                    </button>
                  } />
              </span>
            </span>
          </div>
          {showProject || state ? (
            <div className={cn(indent, "flex min-w-0 items-baseline gap-xs text-micro")} data-archived-line="">
              {showProject ? <span className="shrink-0 text-muted-foreground" data-task-project={project?.id ?? row.project_root}>{project?.label ?? row.project_label}</span> : null}
              {state ? <span className={cn("min-w-0 truncate", failed ? "text-destructive" : "text-subtle-foreground")}>{state}</span> : null}
            </div>
          ) : null}
        </div>
      </li>
    </EntryContextMenu>
  );
}

type TaskMenuId = "record" | "pane" | "revisit" | "resolve" | "archive";
type TaskMenu = { items: () => MenuEntry<TaskMenuId>[]; select: (id: TaskMenuId) => void };

/**
 * A row's menu (B15, B18; PRD session-archive B1): 기록 보기 · pane으로 ·
 * Resolve and, on a root task, 세션 보관, disabled with why where Hide cannot
 * archive it now.
 */
function useTaskMenu(agent: AgentRow, record: (() => void) | null, onPane: () => void, resolvable: boolean, disabled: boolean, actions: Actions, root: boolean): TaskMenu {
  const { t } = useInterfaceTranslation();
  const items = (): MenuEntry<TaskMenuId>[] => {
    const entries: MenuEntry<TaskMenuId>[] = [
      { id: "record", icon: "record", label: t("taskLens.menu.record"), unavailable: record ? null : "", shortcut: "Space" },
      { id: "pane", icon: "terminal", label: t("taskLens.menu.pane"), unavailable: disabled ? "" : null, shortcut: "↩" },
    ];
    // Before 해결 and 세션 보관 on a root task (PRD revisit-group D-08, B9).
    if (root) entries.push(revisitEntry(agent, disabled, t));
    if (resolvable) entries.push({ id: "resolve", icon: "check", label: t("agentSessions.resolve"), unavailable: disabled ? "" : null });
    const archive = root ? archiveOffer(useShellStore.getState().rest, agent, t) : { kind: "hidden" as const };
    if (archive.kind === "offered") entries.push({ id: "archive", icon: "archive", label: t("archive.menu.archive"), unavailable: archive.unavailable, separated: true });
    return entries;
  };
  const select = (id: TaskMenuId) => {
    if (id === "record") return record?.();
    if (id === "pane") return onPane();
    if (id === "revisit") return revisitOn(agent) ? actions.finishRevisit(agent.pane_id) : actions.starSession(agent.pane_id);
    if (id === "resolve") return actions.resolveSession(agent.pane_id);
    return actions.archiveSession(agent.pane_id);
  };
  return { items, select };
}

/** Whether the task waits in 다시 볼 것 or wears the ★: its menu and button then say 다시 볼 것에서 빼기. */
function revisitOn(agent: AgentRow): boolean {
  return agent.revisit != null;
}

function revisitEntry(agent: AgentRow, disabled: boolean, t: TFunction<"translation">): MenuEntry<TaskMenuId> {
  const on = revisitOn(agent);
  return { id: "revisit", icon: on ? "unstar" : "star", label: t(on ? "taskLens.menu.revisitDone" : "taskLens.menu.revisit"), unavailable: disabled ? "" : null, separated: true };
}

/**
 * The ☆/★ beside the record button (PRD revisit-group D-08, B8): ☆ stars the
 * task, ★ on a starred task or one waiting in 다시 볼 것 is 다시 볼 것에서 빼기.
 */
function RevisitButton({ agent, on, disabled, actions }: { agent: AgentRow; on: boolean; disabled: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const label = t(on ? "taskLens.menu.revisitDone" : "taskLens.menu.revisit");
  return (
    <Hint label={label}>
      <button type="button" aria-label={label} aria-pressed={on} disabled={disabled} data-task-revisit={agent.pane_id}
        className={cn("rounded-xs p-xxs outline-none hover:bg-secondary focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50", on ? "text-warning" : "text-muted-foreground")}
        onClick={() => (on ? actions.finishRevisit(agent.pane_id) : actions.starSession(agent.pane_id))}>
        <StarIcon className={cn("size-(--size-icon-sm)", on && "fill-current")} />
      </button>
    </Hint>
  );
}

/**
 * A row's buttons under the pointer or focus, floating over the end of its
 * line on the row's own fill, so a row keeps only its time's width for them.
 * They stand above a PR mark's raised hover target, and a selected row's
 * translucent fill is laid on the sidebar's so nothing shows through. An
 * input with no hover always shows them, in the line in place of the time.
 */
const FLOATING_ACTIONS =
  "absolute top-1/2 right-0 z-20 flex -translate-y-1/2 items-center gap-xxs rounded-xs bg-accent pl-xs group-data-[selected=true]/task:bg-[linear-gradient(var(--color-selected),var(--color-selected)),linear-gradient(var(--color-sidebar),var(--color-sidebar))] hoverless:static hoverless:translate-y-0 hoverless:bg-none hoverless:bg-transparent hoverless:pl-0";

const revealed = (open: boolean) => (open ? "opacity-100" : "opacity-0 group-hover/task:opacity-100 group-focus-within/task:opacity-100 hoverless:opacity-100");

/**
 * The time, and floating over the end of the line under the pointer or focus
 * the record button and `⋯` (B15, B18).
 */
function TimeSlot({ agent, record, menu, revisit }: { agent: AgentRow; record: (() => void) | null; menu: TaskMenu; revisit?: { on: boolean; disabled: boolean; actions: Actions } }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  return (
    <span className="relative flex shrink-0 items-center" onClick={(event) => event.stopPropagation()}>
      <Elapsed since={agent.last_prompt_at_unix_ms} data-agent-elapsed="true" className="pointer-events-none text-micro text-muted-foreground tabular-nums hoverless:hidden" />
      <span data-row-actions="" className={cn(FLOATING_ACTIONS, revealed(open))}>
        {revisit ? <RevisitButton agent={agent} {...revisit} /> : null}
        {record ? (
          <Hint label={t("taskLens.menu.record")} shortcut="Space">
            <button type="button" aria-label={t("taskLens.menu.record")} data-task-record={agent.pane_id} className="rounded-xs p-xxs text-muted-foreground outline-none hover:bg-secondary focus-visible:ring-1 focus-visible:ring-ring" onClick={record}>
              <FileTextIcon className="size-(--size-icon-sm)" />
            </button>
          </Hint>
        ) : null}
        <EntryDropdown label={t("taskLens.more", { name: agent.identity_label })} title={agent.identity_label} items={menu.items} onSelect={menu.select} onOpenChange={setOpen} data-task-menu={agent.pane_id}
          trigger={
            <button type="button" aria-label={t("taskLens.more", { name: agent.identity_label })} data-task-more={agent.pane_id} className="rounded-xs p-xxs text-muted-foreground outline-none hover:bg-secondary focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-secondary">
              <EllipsisIcon className="size-(--size-icon-sm)" />
            </button>
          } />
      </span>
    </span>
  );
}

/**
 * B8: the project's name in 상태별, then the question, the block's cause, the
 * label line or the unfinished work; none is drawn when there is nothing to say.
 */
function SecondLine({ entry, showProject }: { entry: TaskEntry; showProject: boolean }) {
  const { t } = useInterfaceTranslation();
  const indent = detailIndent(useAgentLogosShown() || Boolean(entry.agent.hide_ai));
  const { agent, card, project } = entry;
  const ask = card.state === "needs_you" ? agent.state.ask : null;
  const turn = card.state === "needs_you" && !ask ? agent.hide_ai?.turn : undefined;
  const parts: LineParts | null = ask ? askParts(t, ask) : turn ? { lead: null, text: homeTurnLine(t, turn), tail: null } : card.state === "resting" && !agent.state.session.unfinished ? null : agentLineParts(t, agent);
  if (!parts && !(showProject && project)) return null;
  return (
    <div className={cn(indent, "flex min-w-0 items-baseline gap-xs text-micro")} data-task-line={card.state}>
      {showProject && project ? <span className="shrink-0 text-muted-foreground" data-task-project={project.id}>{project.label}</span> : null}
      {parts ? <RowLineText parts={parts} className={cn("min-w-0 flex-1", LINE_TONE[card.state] ?? "text-subtle-foreground")} title={partsTitle(parts)} /> : null}
    </div>
  );
}

/**
 * A row's line in its parts, as the sidebar's (status-marks B7, B10, B27,
 * B29, B31): a sleeping row has none, a waking, failed or unknown one says its
 * state, a waiting ring leads with 대기 and its cause, a stop leads with how
 * it ended and keeps its background tail; work in progress says its sentence.
 */
function agentLineParts(t: ReturnType<typeof useInterfaceTranslation>["t"], agent: AgentRow): LineParts | null {
  const own = stateLine(agent);
  if (own === null) return null;
  const row = own ?? rowLine(agent);
  if (own || row?.mode === "waiting") return lineParts(t, agent, row!);
  const text = agent.state.session.line;
  if (!text) return null;
  return { lead: agent.mark === "going" || agent.mark === "waiting" ? null : leadWord(t, agent), text, tail: backgroundTail(t, agent) };
}

const partsTitle = (parts: LineParts) => [parts.lead, parts.text, parts.tail].filter(Boolean).join(" · ");

/**
 * An opened row's descendant at any depth (B10, B11): its own chevron, PR
 * and, while folded, its marks; another checkout's branch on its own line,
 * another device's chip; a finished child's title is quiet.
 */
function ChildRow({ parent, child, depth, place, open, selected, disabled, actions, workspace }: { parent: AgentRow; child: AgentRow; depth: number; place: TreePlace; open: boolean | null; selected: boolean; disabled: boolean; actions: Actions; workspace: string | null }) {
  const { t } = useInterfaceTranslation();
  const staleness = useAgentStaleness(child);
  // A primitive, so a snapshot that moves nothing of this row leaves it alone.
  const localId = useShellStore((s) => localDeviceId(s.rest));
  const session = useMemo(() => sessionOfAgentOn(child, localId), [child, localId]);
  const device = useShellStore((s) => {
    const own = remoteTargetOfPane(s.rest, child.pane_id);
    return own !== remoteTargetOfPane(s.rest, parent.pane_id) && own !== null ? machineName(s.rest?.navigator?.devices, own, t) : null;
  });
  const go = () => { if (!disabled) actions.openListedAgent(child.pane_id); };
  const record = session ? () => actions.openSessionRecord(session) : null;
  const toggle = () => actions.toggleSessionTree(child.pane_id);
  const menu = useTaskMenu(child, record, go, false, disabled, actions, false);
  const quiet = !child.state.title_emphasized && !(selected && child.state.selection_emphasizes_title);
  return (
    <li className={cn("group/task flex min-w-0 items-stretch rounded-xs pr-xs", selected ? "bg-selected" : "hover:bg-accent focus-within:bg-accent", rowDimmed(child) && "opacity-(--opacity-dimmed)")}
      data-task-child={child.pane_id} data-depth={depth} data-selected={selected ? "true" : undefined} onClick={go}>
      <TreeRails place={place} />
      <TreeChevron name={child.identity_label} open={open} onToggle={toggle} hangs chain={place.chain} disabled={disabled} data-task-chevron={child.pane_id} />
      <div className="flex min-w-0 flex-1 flex-col gap-xxs py-xs pl-xs">
        <div className="flex min-h-(--size-sidebar-line) min-w-0 items-center gap-xs">
          <button type="button" data-task-focus="row" disabled={disabled} aria-current={selected ? "true" : undefined} aria-expanded={open ?? undefined}
            aria-label={[child.identity_label, forkLabel(t, child), agentProductName(child.agent_kind), recordWords(t, child), child.record_summary, markWords(t, child), child.state.branch_badge, device].filter(Boolean).join(", ")}
            title={[child.identity_label, agentProductName(child.agent_kind), recordWords(t, child), child.record_summary, markWords(t, child), rowSentence(t, child)].filter(Boolean).join("\n")}
            onClick={(event) => { event.stopPropagation(); go(); }}
            onKeyDown={(event) => rowKey(event, open, toggle, record)}
            className="flex min-w-0 flex-1 items-center gap-xs text-left outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50">
            <StatusMark mark={child.mark} className={markTone(child)} /><AgentRowMark kind={child.agent_kind} />
            <AgentRecordTitle agent={child} className={cn("text-caption", quiet ? "text-subtle-foreground" : "text-foreground")} />
            <AgentForkMark agent={child} />
          </button>
          {device ? <span className="flex shrink-0 items-center gap-xxs rounded-xs border border-border px-xxs text-micro text-muted-foreground" data-task-device={device}><ServerIcon aria-hidden="true" className="size-(--size-icon-sm)" />{device}</span> : null}
          {open ? null : <DescendantMark agent={child} />}
          <AgentPrMark agent={child} staleness={staleness} disabled={disabled} onOpen={(pull) => actions.openSessionPullRequest({ workspace_id: workspace, url: pull.url, number: pull.number })} onExternal={(pull) => actions.openLink(pull.url, true)} />
          <TimeSlot agent={child} record={record} menu={menu} />
        </div>
        <ChildLine child={child} />
        {child.state.branch_badge ? <BranchLine agent={child} branch={child.state.branch_badge} /> : null}
      </div>
    </li>
  );
}

function ChildLine({ child }: { child: AgentRow }) {
  const { t } = useInterfaceTranslation();
  const indent = detailIndent(useAgentLogosShown());
  const ask = child.state.session.group === "needs_you" ? child.state.ask : null;
  const parts: LineParts | null = ask ? askParts(t, ask) : agentLineParts(t, child);
  if (!parts) return null;
  return <div className={cn(indent, "flex min-w-0 text-micro", ask ? "text-warning" : "text-subtle-foreground")} data-task-line={ask ? "ask" : "label"}><RowLineText parts={parts} className="min-w-0 flex-1" title={partsTitle(parts)} /></div>;
}

/** Past five siblings, the rest wait behind one line at their depth (B10). */
function TreeMoreRow({ line, onShowAll }: { line: Extract<SidebarTreeLine, { kind: "more" }>; onShowAll: (parent: string) => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <li className="flex min-w-0 items-stretch rounded-xs pr-xs hover:bg-accent">
      <TreeRails place={line.place} />
      <TreeChevron name="" open={null} hangs chain={line.place.chain} />
      <button type="button" data-task-focus="tree-more" data-tree-more={line.count} onClick={() => onShowAll(line.parent)}
        className="flex min-w-0 flex-1 items-center gap-xs py-xs pl-xs text-left text-micro text-subtle-foreground outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring">
        <span className="min-w-0 truncate">{t("agentSessions.tree.more", { count: line.count })}</span>
      </button>
    </li>
  );
}

const DETAIL_INDENT = "pl-[calc(var(--size-agent-mark)+var(--spacing-xs))]";
/** A row that draws a mark before its title (the logo, or a Hide AI root's owl) indents its detail lines past it too. */
const DETAIL_INDENT_MARKED = "pl-[calc(var(--size-agent-mark)+var(--size-agent-badge-compact)+2*var(--spacing-xs))]";

function detailIndent(marked: boolean): string {
  return marked ? DETAIL_INDENT_MARKED : DETAIL_INDENT;
}

function BranchLine({ agent, branch }: { agent: AgentRow; branch: string }) {
  const path = useShellStore((state) => checkoutPathOfPane(state.rest, agent.pane_id));
  return (
    <div className={cn(detailIndent(useAgentLogosShown()), "flex min-w-0 items-center gap-xs text-micro text-muted-foreground")}>
      <GitBranchIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
      <span className="min-w-0 truncate font-mono" data-task-branch={agent.pane_id} title={[branch, path].filter(Boolean).join("\n")}>{branch}</span>
    </div>
  );
}

/** Right opens a row's children and Left folds them; Space opens its record (B12). */
function rowKey(event: KeyboardEvent<HTMLElement>, open: boolean | null, toggle: () => void, record: (() => void) | null) {
  if (event.metaKey || event.ctrlKey || event.altKey) return;
  if (event.key === " " && record) {
    event.preventDefault();
    return record();
  }
  if (open === null) return;
  if ((event.key === "ArrowRight" && !open) || (event.key === "ArrowLeft" && open)) {
    event.preventDefault();
    toggle();
  }
}

/** Up and Down move through the list's group folds and rows, Home and End to its ends (B12). */
function moveFocus(event: KeyboardEvent<HTMLElement>) {
  if (event.metaKey || event.ctrlKey || event.altKey) return;
  const step = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : null;
  if (step === null && event.key !== "Home" && event.key !== "End") return;
  const items = [...event.currentTarget.querySelectorAll<HTMLElement>("[data-task-focus]:not(:disabled)")];
  const current = (event.target as HTMLElement).closest<HTMLElement>("[data-task-focus]");
  const index = current ? items.indexOf(current) : -1;
  const next = event.key === "Home" ? items[0] : event.key === "End" ? items.at(-1) : items[Math.max(0, Math.min(items.length - 1, index + (step ?? 0)))];
  if (next) { event.preventDefault(); next.focus(); }
}
