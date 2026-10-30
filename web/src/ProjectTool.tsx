import { ArrowUpIcon, CircleXIcon, FileDiffIcon, LoaderCircleIcon, SettingsIcon, TriangleAlertIcon, XIcon } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import type { ProjectStatus, ProjectStray } from "./agentScope";
import { agentProductName } from "./agentAdapters";
import { PrMark } from "./components/pr-mark";
import { Button } from "./components/ui/button";
import { RowOverlayButton } from "./components/ui/row-button";
import { Hint } from "./components/ui/tooltip";
import { openDiskCleanup, LAYER_DOT } from "./DiskCleanupSheet";
import { readsHere } from "./devices";
import { entranceBytes, layerLines } from "./diskCleanup";
import { useDiskWatch } from "./diskWatch";
import type { TaskCost } from "./generated/hided-ws";
import { summariesEnabled } from "./hideAi";
import { formatBytes } from "./i18n/format";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { ageWords } from "./overviewLens";
import { allAgents } from "./paneAgents";
import { hasPullRequests, prListModel } from "./prWork";
import { projectRepository, settingsTarget } from "./projectSettings";
import { COSTLY_SHOWN, costPrefix, dayHeights, durationWords, frontProject, missedCount, projectCost, removableDisk, strayParts, strayView, taskName, tokenWords, usd, type TaskFilter } from "./projectStatus";
import { machineName } from "./screenMachine";
import { catalogWorkspaces, localDeviceId, sameDetailItem, type DetailItem, type Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// The lane's Project tool as the results column (PRD project-panel-results
// D-01..D-19, B1-B37): left is state, the middle is work under way, and this
// is what came of it. Under its head one line of counts, then 흘린 것, the
// disk, the last seven days, the costliest tasks and Hide AI's suggestions.
// Every row and number is the core's (`agent_scope.status`, the usage
// frame's `projects`); a press goes where the list or the detail lives.

/** Whether the viewing column draws `item` now, so its control reads pressed. */
function useDetailOpen(item: DetailItem): boolean {
  const drawn = useUiStore((s) => s.drawnLane.view === "detail" && s.drawnLane.mode !== "closed");
  const top = useShellStore((s) => {
    const detail = s.rest?.ui_state?.detail;
    return detail?.shown ? (detail.items.at(-1) ?? null) : null;
  });
  return drawn && sameDetailItem(top, item);
}

export function ProjectTool({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const project = useShellStore((s) => frontProject(s.rest));
  if (!project) return <p className="p-lg text-caption text-muted-foreground" data-project-tool="none">{t("history.openWorkspace")}</p>;
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-project-tool={project.id}>
      <Header project={project} actions={actions} />
      <Results project={project} actions={actions} />
    </div>
  );
}

/** The tool's head: the project's name, its repository, path and machine, ⚙ and ×. */
function Header({ project, actions }: { project: Workspace; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const repository = useShellStore((s) => projectRepository(s.rest, project));
  const device = useShellStore((s) => machineName(s.rest?.navigator?.devices, project.device_id, t));
  const where = [repository, project.path, device].filter(Boolean).join(" · ");
  return (
    <div className="flex shrink-0 items-start gap-xxs py-xs pl-md pr-xs" data-lane-header="true">
      <span className="flex min-w-0 flex-1 flex-col" data-lane-scope={project.label} data-lane-scope-kind="project">
        <span className="truncate text-caption font-semibold text-foreground">{project.label}</span>
        <span className="truncate text-micro text-subtle-foreground" title={where} data-project-where="true">{where}</span>
      </span>
      <Hint label={t("projectTool.settings")}>
        <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("projectTool.settings")} data-project-tool-settings={project.id} onClick={(event) => useUiStore.getState().openProjectSettings(settingsTarget(project), event.currentTarget)}>
          <SettingsIcon />
        </Button>
      </Hint>
      <Hint label={t("panes.lane.closeTool")}>
        <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("panes.lane.closeTool")} data-lane-tool-close="project" onClick={() => actions.closeTool()}>
          <XIcon />
        </Button>
      </Hint>
    </div>
  );
}

function Results({ project, actions }: { project: Workspace; actions: Actions }) {
  const status = project.agent_scope.status ?? null;
  const measures = useShellStore((s) => project.is_git && readsHere(project, s.rest?.navigator?.devices));
  const overviewOpen = useUiStore((s) => s.overviewOpen);
  // While the tool shows, its one project's disk is measured; the Overview's Projects tab takes the watch while it is open (D-15, B20).
  useDiskWatch(actions, measures && !overviewOpen ? project.id : null);
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-lg overflow-y-auto px-md pb-md pt-xs" data-project-status="true">
      <Counts project={project} status={status} actions={actions} />
      {status ? <Strays project={project} status={status} actions={actions} /> : null}
      {measures ? <DiskLine project={project} /> : null}
      <Week project={project} actions={actions} />
      <Suggestions project={project} actions={actions} />
    </div>
  );
}

/** A quiet bar where a number goes before the core's first answer (B5). */
function Pending({ className }: { className?: string }) {
  return <span aria-hidden="true" className={cn("inline-block h-(--size-icon-sm) w-(--size-control) rounded-xs bg-muted align-middle", className)} data-count-pending="true" />;
}

/**
 * The count line (B1-B5): 세션, PR and 워크트리, each opening where its list
 * lives, and 놓친 일 while there is some. A 0 is drawn as 0.
 */
function Counts({ project, status, actions }: { project: Workspace; status: ProjectStatus | null; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const buckets = project.agent_scope.buckets;
  const sessions = buckets.turn + buckets.working + buckets.delegating + buckets.resting;
  const missed = useShellStore((s) => missedCount(s.linkSummaries, project.id, summariesEnabled(s.rest?.status?.background_ai)));
  const agents = useShellStore((s) => s.agents);
  const reading = useMemo(() => prListModel([{ workspace: project, agents, device: null }], null, Date.now()).reading, [project, agents]);
  const worktrees: DetailItem = { kind: "worktrees", workspace_id: project.id };
  const worktreesOpen = useDetailOpen(worktrees);
  const prs = status?.prs.open ?? null;
  const prPending = prs === null || reading.state === "reading";
  const prAge = reading.state === "stale" && reading.minutes !== null ? ageWords(language, reading.minutes * 60_000, t) : null;
  const items: ReactNode[] = [
    <Button key="sessions" variant="text" size="inline" className="text-caption" onClick={() => actions.showProjectInSidebar("tasks", project.device_id, project.id)} data-project-count="sessions">
      {t("projectTool.count.sessions")} <span className="tabular-nums text-foreground" data-count-value={sessions}>{sessions}</span>
    </Button>,
  ];
  if (hasPullRequests(project)) {
    items.push(
      <Button key="prs" variant="text" size="inline" className="text-caption" onClick={() => actions.showProjectInSidebar("prs", project.device_id, project.id)} data-project-count="prs">
        {t("projectTool.count.prs")} {prPending ? <Pending /> : <span className="tabular-nums text-foreground" data-count-value={prs}>{prs}</span>}
        {reading.state === "stale" ? (
          <span className="inline-flex items-center gap-xxs text-warning" data-count-stale="true">
            <TriangleAlertIcon aria-hidden="true" className="size-(--size-icon-sm)" />
            {prAge ? t("projectTool.prs.lastRead", { age: prAge }) : null}
          </span>
        ) : null}
      </Button>,
    );
  }
  if (project.is_git) {
    const count = status?.git.worktrees ?? null;
    items.push(
      <Button key="worktrees" variant="text" size="inline" className={cn("text-caption", worktreesOpen && "text-foreground")} aria-pressed={worktreesOpen} onClick={() => actions.toggleDetailItem(worktrees)} data-project-count="worktrees">
        {t("projectTool.count.worktrees")} {count === null ? <Pending /> : <span className="tabular-nums text-foreground" data-count-value={count}>{count}</span>}
      </Button>,
    );
  }
  if (missed > 0) {
    items.push(
      <Hint key="missed" label={t("projectTool.sessions.missedHint")}>
        <Button variant="text" size="inline" className="text-caption text-warning hover:text-warning" onClick={() => actions.openProjectMissed(project.device_id, project.id)} data-project-count="missed" data-count-value={missed}>
          {t("projectTool.sessions.missed", { count: missed })}
        </Button>
      </Hint>,
    );
  }
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-xs gap-y-xxs px-xs" data-project-counts="true">
      {items.map((item, index) => (
        <span key={index} className="inline-flex items-center gap-xs">
          {index > 0 ? <span aria-hidden="true" className="text-caption text-muted-foreground">·</span> : null}
          {item}
        </span>
      ))}
    </div>
  );
}

/** A part's head: one size above its rows and bolder, with its count. */
function SectionHead({ title, count, children }: { title: string; count?: number; children?: ReactNode }) {
  return (
    <div className="flex min-w-0 items-baseline gap-xs px-xs pb-xxs">
      <h3 className="text-body font-semibold text-foreground">{title}</h3>
      {count !== undefined ? <span className="text-caption tabular-nums text-warning" data-project-section-count={count}>{count}</span> : null}
      <span className="min-w-0 flex-1" />
      {children}
    </div>
  );
}

/** How long a 놔두기's 되돌리기 stays (B15). */
const UNDO_MS = 6_000;

/**
 * 흘린 것 (B6-B16): a row per worktree where work ended unfinished. Its
 * press brings the session that last worked there forward and opens the
 * evidence; hovered or focused, its age gives way to 마무리 맡기기 and
 * 놔두기. With no row the part is not drawn (B8).
 */
function Strays({ project, status, actions }: { project: Workspace; status: ProjectStatus; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const [undo, setUndo] = useState<string | null>(null);
  useEffect(() => {
    if (!undo) return;
    const timer = window.setTimeout(() => setUndo(null), UNDO_MS);
    return () => window.clearTimeout(timer);
  }, [undo]);
  const strays = status.strays ?? [];
  const total = status.strays_total ?? 0;
  if (total === 0 && !undo) return null;
  const view = strayView(strays, total, open);
  return (
    <section className="flex min-w-0 flex-col" aria-label={t("projectTool.strays")} data-project-strays={total}>
      <SectionHead title={t("projectTool.strays")} count={total}>
        <span className="truncate text-micro text-muted-foreground">{t("projectTool.strays.hint")}</span>
      </SectionHead>
      <ul className="flex min-w-0 flex-col">
        {view.shown.map((stray) => <StrayRow key={stray.checkout_id} project={project} stray={stray} actions={actions} onDismiss={() => setUndo(stray.checkout_id)} />)}
      </ul>
      {undo ? (
        <p className="flex items-center gap-xs px-xs py-xxs text-caption text-muted-foreground" data-stray-undo={undo}>
          {t("projectTool.strays.dismissed")}
          <Button variant="text-link" size="inline" onClick={() => { actions.restoreStray(project.id, undo); setUndo(null); }} data-stray-restore={undo}>{t("projectTool.strays.undo")}</Button>
        </p>
      ) : null}
      {view.more > 0 && !open ? (
        <Button variant="text" size="inline" className="self-start px-xs text-caption" onClick={() => setOpen(true)} data-project-strays-more={view.more}>
          {t("projectTool.strays.more", { count: view.more })}
        </Button>
      ) : view.more > 0 ? (
        // Past the core's cap the rest are counted, not listed (B8).
        <span className="px-xs text-caption text-muted-foreground" data-project-strays-more={view.more}>{t("projectTool.strays.more", { count: view.more })}</span>
      ) : null}
    </section>
  );
}

const STRAY_ICON = { changes: FileDiffIcon, unpushed: ArrowUpIcon, ci: CircleXIcon } as const;
const STRAY_TONE = { changes: "text-warning", unpushed: "text-agent-working", ci: "text-pr-failed" } as const;

function StrayRow({ project, stray, actions, onDismiss }: { project: Workspace; stray: ProjectStray; actions: Actions; onDismiss: () => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const [request, setRequest] = useState<string | null>(null);
  const receipt = useShellStore((s) => (request ? s.rest?.status?.stray_hand_offs?.find((row) => row.request_id === request) ?? null : null));
  // Once sent, the words go once (B12): the button stays off until the row
  // leaves as the agent starts, or the hand-off fails. A receipt the core let
  // go after `sent` still reads sent.
  const [sent, setSent] = useState(false);
  useEffect(() => {
    if (receipt?.state === "sent") setSent(true);
    if (receipt?.state === "failed") {
      setSent(false);
      setRequest(null);
    }
  }, [receipt?.state]);
  const sending = request !== null && !sent && (receipt === null || receipt.state === "sending");
  const handedOff = request !== null && (sending || sent);
  const parts = strayParts(stray);
  const first = parts[0]!;
  const Icon = STRAY_ICON[first.kind];
  const name = stray.name ?? stray.branch ?? stray.checkout_id;
  const words = parts.map((part) => part.kind === "ci" ? t("projectTool.strays.ci", { number: String(part.number) }) : t(`projectTool.strays.${part.kind}`, { count: part.count })).join(" · ");
  const age = stray.since_unix_ms ? ageWords(language, Date.now() - stray.since_unix_ms, t) : null;
  const press = () => {
    if (stray.pane_id) actions.openAgent(stray.pane_id);
    if (stray.failed_pr != null && stray.changed_files === 0 && stray.unpushed === 0) actions.openPrCard(project.id, stray.failed_pr, stray.checkout_id);
    else actions.openDetail({ kind: "remains", workspace_id: project.id, checkout_id: stray.checkout_id });
  };
  return (
    <li className="group relative min-w-0 rounded-xs" data-stray-row={stray.checkout_id}>
      <RowOverlayButton aria-label={t("projectTool.strays.open", { name })} onClick={press} data-stray-open={stray.checkout_id} />
      <span className="pointer-events-none relative flex min-h-(--size-control) min-w-0 items-center gap-xs px-xs py-xs">
        <Icon aria-hidden="true" className={cn("size-(--size-icon-sm) shrink-0", STRAY_TONE[first.kind])} />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-caption text-foreground" title={name}>{name}</span>
          <span className="truncate text-micro text-muted-foreground" title={words}>{words}</span>
        </span>
        <span className={cn("shrink-0 text-micro text-muted-foreground", !handedOff && "group-focus-within:hidden group-hover:hidden")} data-stray-age={age ?? ""}>{handedOff ? null : age}</span>
        <span className={cn("pointer-events-auto shrink-0 items-center gap-xxs", handedOff ? "flex" : "hidden group-focus-within:flex group-hover:flex")}>
          <Button size="sm" variant="secondary" disabled={handedOff} onClick={() => { const id = `stray-${crypto.randomUUID()}`; setRequest(id); actions.handOffStray(project.id, stray.checkout_id, id); }} data-stray-hand-off={stray.checkout_id}>
            {sending ? <LoaderCircleIcon role="status" aria-label={t("projectTool.strays.sending")} className="animate-spin" /> : null}
            {t("projectTool.strays.handOff")}
          </Button>
          {sending ? null : (
            <Button size="sm" variant="ghost" onClick={() => { actions.dismissStray(project.id, stray.checkout_id); onDismiss(); }} data-stray-dismiss={stray.checkout_id}>
              {t("projectTool.strays.dismiss")}
            </Button>
          )}
        </span>
      </span>
    </li>
  );
}

/**
 * The disk line (B18, B19): the project's layers in one bar, its size, what
 * the merged worktrees would free and `정리`, which opens the cleanup sheet.
 * While measured it is a skeleton; a project measured only in part is dim
 * and has no size.
 */
function DiskLine({ project }: { project: Workspace }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const lines = layerLines(project, t);
  const size = entranceBytes(project);
  const removable = removableDisk(project);
  const measuring = project.disk?.measuring === true || (!project.disk?.layers && !project.disk?.unavailable_reason);
  if (measuring && !lines) {
    return <div role="status" aria-label={t("overviewProjects.measuring")} className="mx-xs h-(--size-control) rounded-sm bg-muted" data-project-disk="measuring" />;
  }
  const partial = !size || size.partial;
  return (
    <section className={cn("flex min-w-0 items-center gap-sm rounded-sm bg-card px-sm py-xs", partial && "opacity-(--opacity-dimmed)")} aria-label={t("projectTool.disk")} data-project-disk={partial ? "partial" : "measured"}>
      <span className="flex min-w-0 flex-1 flex-col gap-xxs">
        <span aria-hidden="true" className="flex h-(--lens-bar-height) w-full overflow-hidden rounded-xs bg-muted">
          {(lines ?? []).map((line) => <span key={line.key} className={LAYER_DOT[line.key]} style={{ flexGrow: line.bytes, flexBasis: 0 }} />)}
        </span>
        <span className="truncate text-micro text-muted-foreground">
          {[t("projectTool.disk"), size && !size.partial ? formatBytes(language, size.bytes) : null, removable.count > 0 ? t("projectTool.disk.reclaimable", { count: removable.count, size: formatBytes(language, removable.bytes) }) : null].filter(Boolean).join(" · ")}
        </span>
      </span>
      {removable.count > 0 ? <Button size="sm" variant="secondary" onClick={() => openDiskCleanup(project.id, "done")} data-project-disk-cleanup="true">{t("projectTool.disk.cleanup")}</Button> : null}
    </section>
  );
}

/**
 * 최근 7일 (B21-B25): merged pull requests, what one cost and took, the
 * missions, the week's cost with each day's merges, the tasks that left
 * nothing, and the three costliest tasks. A project on a connected device
 * is counted on that machine only (B23).
 */
function Week({ project, actions }: { project: Workspace; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const local = useShellStore((s) => project.device_id === localDeviceId(s.rest));
  const node = useShellStore((s) => s.usage?.nodes[0]);
  const read = useMemo(() => projectCost(node, project.id), [node, project.id]);
  const agents = useShellStore((s) => allAgents(s.agents, s.rest?.status?.remote));
  const links = useShellStore((s) => s.linkSummaries);
  const openTable = (filter: TaskFilter, task?: string) => {
    useUiStore.getState().showTaskTable(filter, task ?? null);
    actions.openDetail({ kind: "cost", workspace_id: project.id });
  };
  if (!local) {
    return (
      <section className="flex min-w-0 flex-col" data-project-week="other-device">
        <SectionHead title={t("projectTool.week")} />
        <p className="px-xs text-caption text-muted-foreground">{t("projectTool.cost.otherDevice")}</p>
      </section>
    );
  }
  if (read.state === "pending") return <div role="status" aria-label={t("projectTool.cost.reading")} className="mx-xs h-(--size-control-lg) rounded-sm bg-muted" data-project-week="pending" />;
  if (read.state === "failed") {
    return (
      <section className="flex min-w-0 flex-col" data-project-week="failed">
        <SectionHead title={t("projectTool.week")} />
        <p className="px-xs text-caption text-muted-foreground"><span role="img" aria-label={t("projectTool.cost.failed")}>–</span></p>
      </section>
    );
  }
  const cost = read.cost;
  const week = cost?.week;
  const total = cost ? (cost.micro_usd > 0 ? `${costPrefix(cost)} ${usd(cost.micro_usd, language)}` : usd(0, language)) : usd(0, language);
  const heights = cost ? dayHeights(cost.days) : [];
  const costly = (cost?.tasks ?? []).slice(0, COSTLY_SHOWN);
  return (
    <>
      <section className="flex min-w-0 flex-col gap-xs" aria-label={t("projectTool.week")} data-project-week={project.id}>
        <SectionHead title={t("projectTool.week")} />
        <div className="grid grid-cols-2 gap-xs px-xs">
          <Figure label={t("projectTool.week.merged")} value={String(week?.merged_prs ?? 0)} note={t("projectTool.week.open", { count: week?.open_prs ?? 0 })} kind="merged" />
          <Figure label={t("projectTool.week.perPrCost")} value={week?.per_pr ? `${week.per_pr.unpriced ? "≥ " : ""}${usd(week.per_pr.micro_usd, language)}` : "–"} note={week?.per_pr ? t("projectTool.week.tokens", { tokens: tokenWords(language, week.per_pr.tokens) }) : null} kind="per-pr-cost" />
          <Figure label={t("projectTool.week.perPrTime")} value={week?.per_pr ? durationWords(language, week.per_pr.elapsed_ms) : "–"} note={week?.per_pr ? t("projectTool.week.worked", { time: durationWords(language, week.per_pr.worked_ms) }) : null} kind="per-pr-time" />
          <Figure label={t("projectTool.week.missions")} value={String(week?.missions ?? 0)} note={week && week.mission_reopened > 0 ? t("projectTool.week.reopened", { count: week.mission_reopened }) : null} kind="missions" />
        </div>
        <div className="flex min-w-0 flex-col gap-xxs px-xs">
          <span className="flex items-baseline gap-xs text-caption text-muted-foreground">
            {t("projectTool.week.cost")} <span className="font-semibold tabular-nums text-foreground" data-week-cost={cost?.micro_usd ?? 0}>{total}</span>
            {read.failed ? <span className="text-micro">{t("projectTool.cost.failedKept")}</span> : null}
          </span>
          {cost ? (
            <span aria-hidden="true" className="flex h-(--size-control-sm) items-end gap-px">
              {heights.map((height, index) => {
                const merged = week?.merged_by_day[index] ?? 0;
                return (
                  <span key={cost.days[index]!.date} className="relative flex h-full flex-1 items-end" data-cost-day={cost.days[index]!.date} data-day-merged={merged}>
                    <span className="w-full rounded-t-xs bg-primary" style={{ height: `${Math.max(height * 100, height > 0 ? 8 : 2)}%`, opacity: height > 0 ? 1 : 0.3 }} />
                    {merged > 0 ? <span className="absolute inset-x-0 bottom-0 h-(--lens-bar-height) rounded-t-xs bg-pr-merged" /> : null}
                  </span>
                );
              })}
            </span>
          ) : null}
          {cost && cost.unpriced_agents.length > 0 ? <span className="truncate text-micro text-muted-foreground" data-week-unpriced={cost.unpriced_agents.length}>{t("projectTool.cost.unpriced", { agents: cost.unpriced_agents.map((agent) => agentProductName(agent)).join(", ") })}</span> : null}
        </div>
        {week && week.no_result_tasks > 0 ? (
          <Button variant="text" size="inline" className="mx-xs self-start text-caption text-warning hover:text-warning" onClick={() => openTable("none")} data-week-no-result={week.no_result_tasks}>
            {t("projectTool.week.noResult", { count: week.no_result_tasks, cost: `${week.no_result_unpriced ? "≥ " : ""}${usd(week.no_result_micro_usd, language)}` })}
          </Button>
        ) : null}
      </section>
      {costly.length > 0 ? (
        <section className="flex min-w-0 flex-col" aria-label={t("projectTool.costly")} data-project-costly={costly.length}>
          <SectionHead title={t("projectTool.costly")}>
            <Button variant="text" size="inline" className="text-caption" onClick={() => openTable("all")} data-costly-all="true">{t("projectTool.costly.all")}</Button>
          </SectionHead>
          <ul className="flex min-w-0 flex-col">
            {costly.map((task) => <CostlyRow key={task.id} task={task} name={taskName(task, agents, links, project.id) ?? t("projectTool.cost.session", { agent: agentProductName(task.root.agent) })} projectId={project.id} onPress={() => openTable("all", task.id)} />)}
          </ul>
        </section>
      ) : null}
    </>
  );
}

function Figure({ label, value, note, kind }: { label: string; value: string; note: string | null; kind: string }) {
  return (
    <span className="flex min-w-0 flex-col rounded-sm bg-card px-sm py-xs" data-week-figure={kind}>
      <span className="truncate text-micro text-muted-foreground">{label}</span>
      <span className="truncate text-subhead font-semibold tabular-nums text-foreground">{value}</span>
      {note ? <span className="truncate text-micro text-muted-foreground">{note}</span> : null}
    </span>
  );
}

/** A costly task's row (B25): its result, name, children, cost and how long it took; the press opens the table on it. */
function CostlyRow({ task, name, projectId, onPress }: { task: TaskCost; name: string; projectId: string; onPress: () => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const children = Math.max(0, task.sessions_total - 1);
  return (
    <li className="relative min-w-0 rounded-xs" data-costly-task={task.id}>
      <RowOverlayButton aria-label={name} onClick={onPress} />
      <span className="pointer-events-none relative flex min-w-0 items-center gap-xs px-xs py-xs">
        <TaskResultMark task={task} projectId={projectId} />
        <span className="min-w-0 flex-1 truncate text-caption text-foreground" title={name}>{name}</span>
        {children > 0 ? <span className="shrink-0 text-micro text-muted-foreground">{t("projectTool.task.children", { count: children })}</span> : null}
        <span className="shrink-0 text-right font-mono text-micro tabular-nums text-foreground">{task.unpriced ? "≥ " : ""}{usd(task.micro_usd, language)}</span>
        <span className="w-(--size-control) shrink-0 text-right text-micro tabular-nums text-muted-foreground">{durationWords(language, task.elapsed_ms)}</span>
      </span>
    </li>
  );
}

/** A task's result as its mark (B30): its pull request's mark, or the kind's word. */
export function TaskResultMark({ task, projectId }: { task: TaskCost; projectId: string }) {
  const { t } = useInterfaceTranslation();
  const result = task.result;
  // An open pull request draws the state the PR list gives it.
  const openState = useShellStore((s) => result.kind === "pr" && result.state === "open"
    ? catalogWorkspaces(s.rest).find((row) => row.id === projectId)?.pull_requests?.find((row) => row.number === result.number)?.state ?? null
    : null);
  if (task.kind === "mission" && task.mission) return <span className="shrink-0 text-micro text-muted-foreground" data-task-result="mission">{t("projectTool.task.missionPrs", { count: task.mission.prs, merged: task.mission.merged })}</span>;
  if (result.kind === "pr" && result.number != null) return <PrMark state={result.state === "merged" ? "merged" : result.state === "closed" ? "closed" : (openState ?? "pending")} number={result.number} compact />;
  return <span className={cn("shrink-0 text-micro", result.kind === "working" ? "text-agent-working" : result.kind === "none" ? "text-warning" : "text-muted-foreground")} data-task-result={result.kind}>{t(`projectTool.task.result.${result.kind}`)}</span>;
}

/** Hide AI 제안 N (B26): the Hide AI tool's 제안 tab; none while Hide AI is off or has nothing. */
function Suggestions({ project, actions }: { project: Workspace; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const count = useShellStore((s) => {
    const opened = s.suggestions?.project;
    return opened && opened.workspace_id === project.id ? opened.counts.suggestions : 0;
  });
  if (count === 0) return null;
  return (
    <Button variant="text" size="inline" className="mx-xs self-start text-caption" onClick={() => actions.openSuggestions(project.device_id, project.id)} data-project-suggestions={count}>
      {t("projectTool.suggestions", { count })}
    </Button>
  );
}
