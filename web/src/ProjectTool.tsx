import { SettingsIcon, TriangleAlertIcon, XIcon } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import type { ProjectStatus, ProjectTodo } from "./agentScope";
import { agentProductName } from "./agentAdapters";
import { markWords, rowLine } from "./agentRow";
import { PrMark } from "./components/pr-mark";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { localDeviceId } from "./devices";
import { summariesEnabled } from "./hideAi";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { ageWords } from "./overviewLens";
import { agentsByPane, allAgents } from "./paneAgents";
import { ReasonLine } from "./prReason";
import { hasPullRequests, prListModel } from "./prWork";
import { projectRepository, settingsTarget } from "./projectSettings";
import { agentShares, costPrefix, dayHeights, frontProject, missedCount, projectCost, todoView, usd } from "./projectStatus";
import { machineName } from "./screenMachine";
import { sameDetailItem, type AgentRow, type DetailItem, type Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// The lane's Project tool (PRD lane-two-columns D-14..D-18, B26-B38): the
// front checkout's project and its 현황; its Memory is a tool of its own
// (operator decision 2026-10-23, replacing D-20). Every count is the core's
// (`agent_scope.status`, the usage frame's `projects`); a tile sends the
// operator where the list or the detail lives, the sidebar or the 보기 slot,
// and the rows carry no button of their own (D-17).

/** Whether the viewing column draws `item` now, so its tile or row reads `닫기 ✕` (B32). */
function useDetailOpen(item: DetailItem): boolean {
  const drawn = useUiStore((s) => s.drawnLane.view === "detail" && s.drawnLane.mode !== "closed" && s.drawnLane.mode !== "hidden");
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
      <Status project={project} actions={actions} />
    </div>
  );
}

/** The tool's head (B26): the project's name, its repository, path and machine, ⚙ and ×. */
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

function Status({ project, actions }: { project: Workspace; actions: Actions }) {
  const status = project.agent_scope.status ?? null;
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-lg overflow-y-auto px-md py-md" data-project-status="true">
      <div className="grid grid-cols-2 gap-sm" data-project-tiles="true">
        <SessionsTile project={project} actions={actions} />
        <PrsTile project={project} status={status} actions={actions} />
        <GitTile project={project} status={status} actions={actions} />
        <CostTile project={project} actions={actions} />
      </div>
      {status ? <TodoList project={project} status={status} actions={actions} /> : null}
    </div>
  );
}

/** One stacked bar of counts, each part its tone; nothing drawn while every part is 0. */
function Bar({ parts }: { parts: { key: string; count: number; tone: string }[] }) {
  if (parts.every((part) => part.count === 0)) return <span aria-hidden="true" className="block h-(--lens-bar-height) w-full rounded-xs bg-muted" />;
  return (
    <span aria-hidden="true" className="flex h-(--lens-bar-height) w-full gap-px overflow-hidden rounded-xs bg-muted">
      {parts.filter((part) => part.count > 0).map((part) => <span key={part.key} className={part.tone} style={{ flexGrow: part.count, flexBasis: 0 }} data-tile-bar={part.key} />)}
    </span>
  );
}

/**
 * One 현황 tile (B27-B33): its name and where it goes, `닫기 ✕` while the
 * viewing column shows what it opened, its number or a quiet bar before the
 * core's first answer, then its bar and its words.
 */
function Tile({ name, kind, target, open, value, disabled, onPress, children }: {
  name: string;
  kind: string;
  target: "sidebar" | "view";
  open?: boolean;
  /** Null before the core's first answer (B33). */
  value: ReactNode | null;
  disabled?: boolean;
  onPress: () => void;
  children?: ReactNode;
}) {
  const { t } = useInterfaceTranslation();
  return (
    // The press layer covers the tile; the text sits above it (`relative`, later in order) so a hover tints behind it.
    <div className="relative flex min-w-0 flex-col gap-xs rounded-md border border-border p-sm" data-project-tile={kind} data-tile-open={open ? "true" : undefined}>
      <button type="button" disabled={disabled} aria-pressed={target === "view" ? Boolean(open) : undefined} aria-label={name} className="absolute inset-0 rounded-md outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none" onClick={onPress} data-project-tile-press={kind} />
      <span className="pointer-events-none relative flex min-w-0 items-center gap-xs text-micro text-muted-foreground">
        <span className="min-w-0 flex-1 truncate">{name}</span>
        {disabled ? null : <span className={cn("shrink-0", open && "text-foreground")} data-tile-target={open ? "close" : target}>{open ? t("projectTool.close") : t(target === "sidebar" ? "projectTool.tile.toSidebar" : "projectTool.tile.toView")}</span>}
      </span>
      <span className="pointer-events-none relative min-h-(--size-control-sm) text-subhead font-semibold tabular-nums text-foreground" data-tile-value={value === null ? "pending" : "ready"}>
        {value === null ? <span className="block h-(--size-control-sm) w-1/2 rounded-sm bg-muted" /> : value}
      </span>
      <div className="pointer-events-none relative flex min-w-0 flex-col gap-xxs text-micro text-muted-foreground">{children}</div>
    </div>
  );
}

const SESSION_TONES = { turn: "bg-warning", working: "bg-agent-working", delegating: "bg-muted-foreground", resting: "bg-border" } as const;

function SessionsTile({ project, actions }: { project: Workspace; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const buckets = project.agent_scope.buckets;
  const missed = useShellStore((s) => missedCount(s.linkSummaries, project.id, summariesEnabled(s.rest?.status?.background_ai)));
  const total = buckets.turn + buckets.working + buckets.delegating + buckets.resting;
  const parts = (Object.keys(SESSION_TONES) as (keyof typeof SESSION_TONES)[]).map((key) => ({ key, count: buckets[key], tone: SESSION_TONES[key] }));
  return (
    <div className="relative min-w-0">
      <Tile name={t("projectTool.tile.sessions")} kind="sessions" target="sidebar" value={total} onPress={() => actions.showProjectInSidebar("tasks", project.device_id, project.id)}>
        <Bar parts={parts} />
        <span className="truncate" data-tile-sessions={total}>{parts.filter((part) => part.count > 0).map((part) => t(`projectTool.sessions.${part.key}`, { count: part.count })).join(" · ")}</span>
      </Tile>
      {/* Above the tile's press, its own: the backlog's 놓친 일 for the project (B28). */}
      {missed > 0 ? (
        <Hint label={t("projectTool.sessions.missedHint")}>
          <button type="button" className="absolute bottom-sm right-sm z-1 rounded-xs px-xxs text-micro font-medium text-warning outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" onClick={() => actions.openProjectMissed(project.device_id, project.id)} data-tile-missed={missed}>
            {t("projectTool.sessions.missed", { count: missed })}
          </button>
        </Hint>
      ) : null}
    </div>
  );
}

const PR_TONES = { mergeable: "bg-pr-mergeable", failed: "bg-pr-failed", review: "bg-pr-pending", checking: "bg-muted-foreground" } as const;

function PrsTile({ project, status, actions }: { project: Workspace; status: ProjectStatus | null; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const agents = useShellStore((s) => s.agents);
  const reading = useMemo(() => prListModel([{ workspace: project, agents, device: null }], null, Date.now()).reading, [project, agents]);
  if (!hasPullRequests(project)) {
    return <Tile name={t("projectTool.tile.prs")} kind="prs" target="sidebar" disabled value={<span className="text-caption font-normal text-muted-foreground" data-tile-prs="no-remote">{t("prs.noRemote")}</span>} onPress={() => {}} />;
  }
  const counts = status?.prs ?? null;
  const pending = counts === null || reading.state === "reading";
  const stale = reading.state === "stale";
  const age = stale && reading.minutes !== null ? ageWords(language, reading.minutes * 60_000, t) : null;
  return (
    <Tile name={t("projectTool.tile.prs")} kind="prs" target="sidebar" onPress={() => actions.showProjectInSidebar("prs", project.device_id, project.id)}
      value={pending ? null : (
        <span className="flex items-center gap-xs" data-tile-prs={counts.open}>
          {counts.open}
          {stale ? <TriangleAlertIcon role="img" aria-label={t("projectTool.prs.lastRead", { age: age ?? "" })} className="size-(--size-icon-sm) text-warning" data-tile-prs-stale="true" /> : null}
        </span>
      )}>
      {counts && !pending ? (
        <>
          <Bar parts={(Object.keys(PR_TONES) as (keyof typeof PR_TONES)[]).map((key) => ({ key, count: counts[key], tone: PR_TONES[key] }))} />
          <span className="truncate">{(Object.keys(PR_TONES) as (keyof typeof PR_TONES)[]).filter((key) => counts[key] > 0).map((key) => t(`projectTool.prs.${key}`, { count: counts[key] })).join(" · ")}</span>
          {age ? <span className="truncate text-warning" data-tile-prs-age="true">{t("projectTool.prs.lastRead", { age })}</span> : null}
        </>
      ) : null}
    </Tile>
  );
}

const DOT_TONES = { changed: "bg-warning", unpushed: "bg-agent-working", removable: "bg-pr-merged", clean: "bg-border" } as const;

function GitTile({ project, status, actions }: { project: Workspace; status: ProjectStatus | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const item: DetailItem = { kind: "worktrees", workspace_id: project.id };
  const open = useDetailOpen(item);
  if (!project.is_git) {
    return <Tile name={t("projectTool.tile.git")} kind="git" target="view" disabled value={<span className="text-caption font-normal text-muted-foreground">{t("projectTool.git.noGit")}</span>} onPress={() => {}} />;
  }
  const git = status?.git ?? null;
  return (
    <Tile name={t("projectTool.tile.git")} kind="git" target="view" open={open} onPress={() => actions.toggleDetailItem(item)}
      value={git ? <span data-tile-git={git.worktrees}>{t("projectTool.git.worktrees", { count: git.worktrees })}</span> : null}>
      {git ? (
        <>
          <span className="flex flex-wrap gap-xxs" data-tile-git-dots={git.dots.length}>
            {git.dots.map((dot) => <span key={dot.checkout_id} aria-hidden="true" className={cn("size-(--size-status-mark) rounded-full", DOT_TONES[dot.state])} data-git-dot={dot.state} />)}
          </span>
          <span className="truncate">
            {[git.behind ? t("projectTool.git.behind", { count: git.behind }) : null, git.changed ? t("projectTool.git.changed", { count: git.changed }) : null, git.unpushed ? t("projectTool.git.unpushed", { count: git.unpushed }) : null, git.removable ? t("projectTool.git.removable", { count: git.removable }) : null].filter(Boolean).join(" · ")}
          </span>
        </>
      ) : null}
    </Tile>
  );
}

function CostTile({ project, actions }: { project: Workspace; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const item: DetailItem = { kind: "cost", workspace_id: project.id };
  const open = useDetailOpen(item);
  const local = useShellStore((s) => project.device_id === localDeviceId(s.rest));
  const node = useShellStore((s) => s.usage?.nodes[0]);
  const read = useMemo(() => projectCost(node, project.id), [node, project.id]);
  if (!local) {
    return <Tile name={t("projectTool.tile.cost")} kind="cost" target="view" disabled value={<span className="text-caption font-normal text-muted-foreground">{t("projectTool.cost.otherDevice")}</span>} onPress={() => {}} />;
  }
  const cost = read.state === "ready" ? read.cost : null;
  const heights = cost ? dayHeights(cost.days) : [];
  const value = read.state === "pending" ? null
    : read.state === "failed" ? <span role="img" aria-label={t("projectTool.cost.failed")} className="text-muted-foreground" data-tile-cost-failed="true">–</span>
    : <span data-tile-cost={cost?.micro_usd ?? 0}>{cost && cost.micro_usd > 0 ? `${costPrefix(cost)} ${usd(cost.micro_usd, language)}` : usd(0, language)}</span>;
  return (
    <Tile name={t("projectTool.tile.cost")} kind="cost" target="view" open={open} onPress={() => actions.toggleDetailItem(item)} value={value}>
      {cost ? (
        <>
          <span aria-hidden="true" className="flex h-(--size-control-sm) items-end gap-px">
            {heights.map((height, index) => <span key={cost.days[index]!.date} className="flex-1 rounded-t-xs bg-primary" style={{ height: `${Math.max(height * 100, height > 0 ? 8 : 2)}%`, opacity: height > 0 ? 1 : 0.3 }} data-cost-day={cost.days[index]!.date} />)}
          </span>
          <span className="truncate">{agentShares(cost).map((row) => `${agentProductName(row.agent)} ${Math.round(row.share * 100)}%`).join(" · ")}</span>
          {cost.unpriced_agents.length > 0 ? <span className="truncate" data-tile-cost-unpriced={cost.unpriced_agents.length}>{t("projectTool.cost.unpriced", { agents: cost.unpriced_agents.map((agent) => agentProductName(agent)).join(", ") })}</span> : null}
        </>
      ) : null}
    </Tile>
  );
}

/**
 * 할 일 (B34-B37): the agents waiting for the operator, new asks then 다시
 * 볼 것, then the operator's pull requests and reviews; five until `+N 더`.
 * A row says where it goes and does nothing else there (D-17).
 */
function TodoList({ project, status, actions }: { project: Workspace; status: ProjectStatus; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const local = useShellStore((s) => s.agents);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const byPane = useMemo(() => agentsByPane(allAgents(local, remote)), [local, remote]);
  const promoted = useShellStore((s) => {
    const memory = s.memory?.project;
    return memory?.workspace_id === project.id ? memory.rows.flatMap((row) => (row.promotion?.pr_number ? [row.promotion.pr_number] : [])).join(",") : "";
  });
  const fromMemory = useMemo(() => new Set(promoted ? promoted.split(",").map(Number) : []), [promoted]);
  const view = todoView(status, open);
  if (status.todo_total === 0) {
    return (
      <section className="flex min-w-0 flex-col gap-xs" aria-label={t("projectTool.todo")} data-project-todo="empty">
        <SectionHead title={t("projectTool.todo")} count={0} />
        <p className="px-xs text-caption text-subtle-foreground opacity-(--opacity-dimmed)">{t("projectTool.todo.empty")}</p>
      </section>
    );
  }
  return (
    <section className="flex min-w-0 flex-col gap-xs" aria-label={t("projectTool.todo")} data-project-todo={status.todo_total}>
      <SectionHead title={t("projectTool.todo")} count={status.todo_total} />
      {view.asks.length + view.revisit.length > 0 ? (
        <TodoGroup title={t("projectTool.todo.agents")} kind="agents">
          {view.asks.map((row) => <AgentTodo key={`ask:${row.kind === "ask" ? row.pane_id : ""}`} row={row} agent={row.kind === "ask" ? byPane.get(row.pane_id) : undefined} actions={actions} />)}
          {view.revisit.length > 0 ? <li className="px-xs pt-xs text-micro text-muted-foreground" data-project-todo-band="revisit">{t("projectTool.todo.revisit")}</li> : null}
          {view.revisit.map((row) => <AgentTodo key={`revisit:${row.kind === "revisit" ? row.pane_id : ""}`} row={row} agent={row.kind === "revisit" ? byPane.get(row.pane_id) : undefined} actions={actions} />)}
        </TodoGroup>
      ) : null}
      {view.prs.length > 0 ? (
        <TodoGroup title={t("projectTool.todo.prs")} kind="prs">
          {view.prs.map((row) => row.kind === "pr" ? <PrTodo key={`pr:${row.number}`} project={project} number={row.number} fromMemory={fromMemory.has(row.number)} actions={actions} /> : null)}
        </TodoGroup>
      ) : null}
      {view.more > 0 ? (
        <button type="button" className="self-start rounded-xs px-xs text-caption text-subtle-foreground outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" onClick={() => setOpen(true)} data-project-todo-more={view.more}>
          {t("projectTool.todo.more", { count: view.more })}
        </button>
      ) : view.beyond.agents + view.beyond.prs > 0 ? (
        // The rows past the core's cap, each kind to the sidebar list that holds it, narrowed to the project.
        <div className="flex flex-wrap gap-xs" data-project-todo-beyond={view.beyond.agents + view.beyond.prs}>
          {([["agents", "tasks"], ["prs", "prs"]] as const).map(([kind, lens]) => view.beyond[kind] > 0 ? (
            <button key={kind} type="button" className="rounded-xs px-xs text-caption text-subtle-foreground outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" onClick={() => actions.showProjectInSidebar(lens, project.device_id, project.id)} data-project-todo-beyond-to={lens}>
              {t(`projectTool.todo.beyond.${kind}`, { count: view.beyond[kind] })}
            </button>
          ) : null)}
        </div>
      ) : null}
    </section>
  );
}

/**
 * 할 일's head: one size above its rows and bolder, so the part reads before
 * its rows; a group inside the part keeps the small muted head, a third step.
 */
function SectionHead({ title, count }: { title: string; count: number }) {
  return (
    <div className="flex min-w-0 items-center gap-xs px-xs pb-xxs">
      <h3 className="text-body font-semibold text-foreground">{title}</h3>
      <span className="text-caption tabular-nums text-subtle-foreground" data-project-section-count={count}>{count}</span>
    </div>
  );
}

function TodoGroup({ title, kind, children }: { title: string; kind: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col" data-project-todo-group={kind}>
      <h4 className="px-xs pb-xxs text-micro font-medium text-subtle-foreground">{title}</h4>
      <ul className="flex min-w-0 flex-col">{children}</ul>
    </div>
  );
}

/** A row's right side: where it goes (B36), or `닫기 ✕` while the viewing column shows it (B32). */
function Tail({ children, open }: { children: ReactNode; open?: boolean }) {
  return <span className={cn("shrink-0 text-micro", open ? "text-foreground" : "text-muted-foreground")} data-todo-tail={open ? "close" : "go"}>{children}</span>;
}

const ROW = "flex w-full min-w-0 items-center gap-xs rounded-xs px-xs py-xs text-left outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring";

/** An ask or a 다시 볼 것 (B34): the agent's mark, name and what it asks; the press brings its pane and the keyboard. */
function AgentTodo({ row, agent, actions }: { row: ProjectTodo; agent: AgentRow | undefined; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  if (row.kind === "pr") return null;
  const words = row.kind === "revisit"
    ? t(row.record || agent?.revisit?.read_ask ? "projectTool.todo.readAsk" : "projectTool.todo.starred")
    : (agent ? (rowLine(agent)?.text ?? markWords(t, agent)) : t("projectTool.todo.ask"));
  return (
    <li className="min-w-0" data-project-todo-row={row.kind} data-todo-pane={row.pane_id}>
      <button type="button" className={ROW} onClick={() => actions.openAgent(row.pane_id)}>
        <StatusMark mark={agent?.mark ?? null} />
        <span className="min-w-0 flex-1 truncate text-caption">
          <span className="font-medium text-foreground">{agent?.identity_label ?? t("projectTool.todo.unknownAgent")}</span>
          <span className="text-muted-foreground"> · {words}</span>
        </span>
        <Tail>{t("projectTool.todo.toPane")}</Tail>
      </button>
    </li>
  );
}

/** A Needs You pull request (B35): its mark, title and reason; the press opens its card in the viewing column, or folds it. */
function PrTodo({ project, number, fromMemory, actions }: { project: Workspace; number: number; fromMemory: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const item: DetailItem = { kind: "pull_request", workspace_id: project.id, number };
  const open = useDetailOpen(item);
  const pull = project.pull_requests?.find((row) => row.number === number);
  const board = project.agent_scope.prs.rows.find((row) => row.number === number);
  return (
    <li className="min-w-0" data-project-todo-row="pr" data-todo-pr={number}>
      <button type="button" className={ROW} aria-pressed={open} onClick={() => (open ? actions.toggleDetailItem(item) : actions.openPrCard(project.id, number))}>
        {pull ? <PrMark state={pull.state} number={number} compact /> : <span className="shrink-0 text-micro text-muted-foreground">#{number}</span>}
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-caption text-foreground">{pull?.title ?? `#${number}`}</span>
          {board ? <ReasonLine reason={board.reason} group={board.group} /> : null}
        </span>
        {fromMemory ? <span className="shrink-0 text-micro text-subtle-foreground" data-todo-memory={number}>◆ {t("projectTool.todo.fromMemory")}</span> : null}
        <Tail open={open}>{open ? t("projectTool.close") : t("projectTool.todo.toView")}</Tail>
      </button>
    </li>
  );
}
