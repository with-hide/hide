import { CheckIcon, ChevronRightIcon, FolderIcon, PlusIcon, SettingsIcon, WaypointsIcon, XIcon } from "lucide-react";
import { useMemo, type ReactNode } from "react";
import { ProjectLanePin } from "./ProjectLanePin";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { useElapsed } from "./components/elapsed";
import { ProgressCells } from "./components/mission-tone";
import { PrMark } from "./components/pr-mark";
import { Button } from "./components/ui/button";
import { RowButton, RowOverlayButton } from "./components/ui/row-button";
import { Hint } from "./components/ui/tooltip";
import { DEFAULT_CONDUCTOR } from "./conductorSettings";
import { useInterfaceTranslation } from "./i18n/client";
import { formatDateTime } from "./i18n/format";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { PILL } from "./MissionMap";
import { MissionStartGuide } from "./MissionStartGuide";
import { MissionTakeOver } from "./MissionTakeOver";
import { groupMissions, CLOSED_SHOWN, missionEntry, projectMissions, shownMission, turnCount, type MissionEntry, type MissionGroup } from "./missionList";
import { liveNodes, missionCells, missionCounts, type TurnItem } from "./missions";
import { allAgents, agentsByPane } from "./paneAgents";
import { frontProject } from "./projectStatus";
import { CandidateAdd, CandidateRows, CapControl, ConductorError, OtherVendorControl, ScopeControl, StrengthControl, useConductorEdit } from "./settings/ConductorControls";
import type { Mission, MissionNode, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// The rail's Mission tool (PRD mission-tool D-01..D-03, D-08..D-10, B1-B11):
// the front project's missions in groups, each opening its map in the 보기
// slot; with none, how a mission starts; and the Conductor defaults, edited
// in place with Settings › Conductor's own controls. Every state and number
// is the core's mission record; what waits on the operator is the map's own
// `내 차례` rule (`turnItems`).

/** The front project's missions, each in its group, with the worker rows the `내 차례` rule reads; null while the record cannot be read. */
export function useProjectMissions(project: Workspace | null): MissionEntry[] | null {
  const missions = useShellStore((s) => s.rest?.missions);
  const local = useShellStore((s) => s.agents);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  return useMemo(() => {
    if (missions?.failed) return null;
    if (!project) return [];
    const rows = agentsByPane(allAgents(local, remote));
    const workerRow = (node: MissionNode) => (node.pane_id ? (rows.get(node.pane_id) ?? null) : null);
    return projectMissions(missions?.items ?? [], project).map((mission) => missionEntry(mission, workerRow));
  }, [missions, project, local, remote]);
}

/** The rail icon's number (D-08, B2): open missions of the front project with anything in their `내 차례`. */
export function useMissionTurnCount(): number {
  const project = useShellStore((s) => frontProject(s.rest));
  const entries = useProjectMissions(project);
  return entries ? turnCount(entries) : 0;
}

export function MissionTool({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const project = useShellStore((s) => frontProject(s.rest));
  const page = useUiStore((s) => s.missionToolPage);
  if (!project) return <p className="p-lg text-caption text-muted-foreground" data-mission-tool="none">{t("history.openWorkspace")}</p>;
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-mission-tool={project.id} data-mission-tool-page={page}>
      <Head project={project} page={page} actions={actions} />
      {page === "settings" ? <Defaults actions={actions} /> : <Missions project={project} actions={actions} />}
    </div>
  );
}

/** The head (B8, B9): the tool and its project, or its defaults; `+` for the start guide over a list, ⚙ for the defaults, ✕. */
function Head({ project, page, actions }: { project: Workspace; page: "list" | "settings"; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const guide = useUiStore((s) => s.missionGuideOpen);
  const any = useShellStore((s) => !s.rest?.missions?.failed && projectMissions(s.rest?.missions?.items ?? [], project).length > 0);
  const settings = page === "settings";
  const scope = settings ? t("missionTool.defaults") : project.label;
  return (
    <div className="flex h-[var(--size-tab-strip)] shrink-0 items-center gap-xxs border-b border-border pl-md pr-xs text-caption" data-lane-header="true">
      <span className="flex min-w-0 flex-1 items-center truncate" data-lane-scope={scope} data-lane-scope-kind="project">
        <WaypointsIcon aria-hidden="true" className="mr-xs size-(--size-icon-sm) shrink-0 text-muted-foreground" />
        <span className="font-semibold text-foreground">{t("missionTool.title")}</span>
        <span className="sr-only"> · </span>
        <span className="ml-xs min-w-0 truncate text-subtle-foreground">
          {settings ? null : <FolderIcon aria-hidden="true" className="mr-xs inline size-(--size-icon-sm) align-middle" />}
          {scope}
        </span>
      </span>
      {!settings && any ? (
        <Hint label={guide ? t("missionTool.startClose") : t("missionTool.start")}>
          <Button variant="ghost" size="icon-sm" className={cn(guide ? "bg-secondary text-foreground" : "text-muted-foreground")} aria-pressed={guide} aria-label={t("missionTool.start")} data-mission-tool-start="true" onClick={() => useUiStore.getState().setMissionGuideOpen(!guide)}>
            <PlusIcon />
          </Button>
        </Hint>
      ) : null}
      <Hint label={t("missionTool.settings")}>
        <Button variant="ghost" size="icon-sm" className={cn(settings ? "bg-secondary text-foreground" : "text-muted-foreground")} aria-pressed={settings} aria-label={t("missionTool.settings")} data-mission-tool-settings="true" onClick={() => (settings ? useUiStore.getState().setMissionToolPage("list") : actions.openMissionSettings())}>
          <SettingsIcon />
        </Button>
      </Hint>
      <ProjectLanePin column="tool" actions={actions} />
      <Hint label={t("panes.lane.closeTool")}>
        <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("panes.lane.closeTool")} data-lane-tool-close="mission" onClick={() => actions.closeTool()}>
          <XIcon />
        </Button>
      </Hint>
    </div>
  );
}

/** The list (B5-B7, B10): one quiet line while the record cannot be read, the start guide while the project has no mission, else the groups. */
function Missions({ project, actions }: { project: Workspace; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const entries = useProjectMissions(project);
  const guide = useUiStore((s) => s.missionGuideOpen);
  const selected = useShellStore((s) => shownMission(s.rest?.ui_state?.detail));
  // The reason is the core's diagnostic; the operator can do nothing with it here (design principle 13).
  if (entries === null) return <p className="px-md py-md text-caption text-muted-foreground" data-mission-tool-state="unreadable">{t("missionTool.unreadable")}</p>;
  if (entries.length === 0) {
    return (
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto" data-mission-tool-state="empty">
        <div className="px-md py-md"><MissionStartGuide /></div>
        <DefaultsSummary actions={actions} />
      </div>
    );
  }
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-md overflow-y-auto px-md py-md" data-mission-tool-state="list">
      {guide ? (
        <div className="relative rounded-md border border-border p-sm" data-mission-tool-guide="true">
          <Hint label={t("missionTool.startClose")}>
            <Button variant="ghost" size="icon-sm" className="absolute right-xxs top-xxs text-muted-foreground" aria-label={t("missionTool.startClose")} data-mission-tool-guide-close="true" onClick={() => useUiStore.getState().setMissionGuideOpen(false)}>
              <XIcon />
            </Button>
          </Hint>
          <MissionStartGuide />
        </div>
      ) : null}
      {groupMissions(entries).map(({ group, entries: rows }) => (
        <Group key={group} group={group} entries={rows} selected={selected} actions={actions} />
      ))}
    </div>
  );
}

/** One group (B5, B7): its name and count, its rows; 끝남 shows its last three until `모두 ›`. */
function Group({ group, entries, selected, actions }: { group: MissionGroup; entries: MissionEntry[]; selected: string | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const all = useUiStore((s) => s.missionClosedAll);
  const closed = group === "closed";
  const shown = closed && !all ? entries.slice(0, CLOSED_SHOWN) : entries;
  return (
    <section className="flex min-w-0 flex-col gap-xs" aria-label={`${t(`missionTool.group.${group}`)} ${entries.length}`} data-mission-group={group} data-mission-group-count={entries.length}>
      <h3 className={cn("flex items-center gap-xs text-caption font-semibold", group === "turn" ? "text-mission-hold" : "text-muted-foreground")}>
        <span>{t(`missionTool.group.${group}`)}</span>
        <span className="font-normal tabular-nums text-subtle-foreground">{entries.length}</span>
        {closed && entries.length > CLOSED_SHOWN ? (
          <Button variant="text" size="inline" className="ml-auto gap-0" aria-expanded={all} data-mission-closed-all={all ? "open" : "folded"} onClick={() => useUiStore.getState().setMissionClosedAll(!all)}>
            {all ? t("missionTool.fewer") : t("missionTool.all")}
            {all ? null : <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
          </Button>
        ) : null}
      </h3>
      {closed ? (
        <div className="flex min-w-0 flex-col">
          {shown.map((entry) => <ClosedRow key={entry.mission.id} mission={entry.mission} selected={selected === entry.mission.id} actions={actions} />)}
        </div>
      ) : (
        shown.map((entry) => <OpenRow key={entry.mission.id} entry={entry} selected={selected === entry.mission.id} actions={actions} />)
      )}
    </section>
  );
}

/**
 * An open mission's row (B6, B11): its state, title, one line of what waits,
 * works or merges, a cell per node with `n/m`, and its time; a mission whose
 * device does not answer keeps its last values dimmed and counts no time.
 * Pressing it opens the map in the 보기 slot, where it then reads selected,
 * whichever place opened the map (B23).
 */
function OpenRow({ entry, selected, actions }: { entry: MissionEntry; selected: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { mission, group, turn } = entry;
  const { done, total } = missionCounts(mission);
  const elapsed = useElapsed(mission.stale ? null : (mission.started_at_unix_ms ?? mission.created_at_unix_ms));
  // A row the operator has to take over carries its 이어받기 beside the line, so the row is an overlay button.
  return (
    <div
      className={cn(
        "relative flex min-w-0 flex-col gap-xxs rounded-md border bg-card px-sm py-xs",
        selected ? "border-primary ring-1 ring-primary" : group === "turn" ? "border-mission-hold/40" : "border-border",
        mission.stale && "opacity-(--opacity-dimmed)",
      )}
      data-mission-row={mission.id}
      data-mission-row-group={group}
      data-mission-row-stale={mission.stale || undefined}
      data-selected={selected ? "true" : undefined}
    >
      <RowOverlayButton className="rounded-md" aria-label={t("missionTool.open", { title: mission.title })} aria-current={selected ? "true" : undefined} onClick={() => actions.openMission(mission.id)} />
      <span className="pointer-events-none relative flex min-w-0 items-center gap-xs">
        <Pill mission={mission} turn={turn[0] ?? null} />
        <span className="min-w-0 truncate text-body font-semibold text-foreground" title={mission.title}>{mission.title}</span>
      </span>
      <span className="pointer-events-none relative flex min-w-0 items-center gap-xs">
        <RowLine mission={mission} turn={turn[0] ?? null} />
        {turn[0]?.kind === "takeover" ? <span className="pointer-events-auto ml-auto shrink-0"><MissionTakeOver mission={mission} actions={actions} /></span> : null}
      </span>
      {mission.nodes.length > 0 || elapsed ? (
        <span className="pointer-events-none relative flex min-w-0 items-center gap-xs text-caption text-muted-foreground">
          {mission.nodes.length > 0 ? (
            <>
              <ProgressCells tones={missionCells(mission)} size="chip" />
              <span className="shrink-0 font-mono tabular-nums" data-mission-row-counts={`${done}/${total}`}>{done}/{total}</span>
            </>
          ) : null}
          {elapsed ? <span className="shrink-0" data-mission-row-elapsed="true">· {elapsed}</span> : null}
        </span>
      ) : null}
    </div>
  );
}

const TURN_PILL: Record<TurnItem["kind"], "missionTool.pill.decision" | "missionTool.pill.takeover" | "missionTool.pill.blocked" | "missionTool.pill.closedPr" | "missionTool.pill.merge"> = {
  decision: "missionTool.pill.decision",
  takeover: "missionTool.pill.takeover",
  worker_blocked: "missionTool.pill.blocked",
  outside_closed: "missionTool.pill.closedPr",
  landing: "missionTool.pill.merge",
};

/** The state pill: what waits on the operator, `!` in the decision colour, else the map head's own pill. */
function Pill({ mission, turn }: { mission: Mission; turn: TurnItem | null }) {
  const { t } = useInterfaceTranslation();
  if (turn) {
    const key = turn.kind === "landing" && turn.git ? "missionTool.pill.join" : TURN_PILL[turn.kind];
    return <span className="flex shrink-0 items-center gap-xxs rounded-full bg-mission-hold/15 px-sm text-caption font-semibold text-mission-hold" data-mission-row-pill={turn.kind}><span aria-hidden="true">!</span>{t(key)}</span>;
  }
  const pill = PILL[mission.status];
  return (
    <span className={cn("flex shrink-0 items-center gap-xxs rounded-full px-sm text-caption font-semibold", pill.tone)} data-mission-row-pill={mission.status}>
      {mission.status === "done" ? <CheckIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : null}
      {t(pill.key)}
    </span>
  );
}

/** The row's one line (B6): what waits on the operator, else the node at work, else the plan being made. */
function RowLine({ mission, turn }: { mission: Mission; turn: TurnItem | null }) {
  const { t } = useInterfaceTranslation();
  if (turn) {
    switch (turn.kind) {
      case "decision":
        return <Line data="decision">{mission.decision}</Line>;
      case "takeover": {
        // The pill already says no Conductor holds it; the line says what is left for the one who takes it.
        const { done, total } = missionCounts(mission);
        const left = total - done;
        return <Line data="takeover">{left > 0 ? t("missionTool.takeoverLeft", { count: left }) : t("missionTool.takeoverReport")}</Line>;
      }
      case "worker_blocked":
        return <Line data="worker_blocked">{turn.node.title} · {turn.line ?? t(`mission.turn.cause.${turn.cause}`)}</Line>;
      case "outside_closed":
        return <Line data="outside_closed">{turn.node.title} · {t("mission.outside.closed")}</Line>;
      case "landing":
        return <LandingLine nodes={turn.nodes} join={turn.git} />;
    }
  }
  if (mission.status === "planning" && mission.nodes.length === 0) return <Line data="planning">{t("mission.planning")}</Line>;
  const live = liveNodes(mission);
  const first = live[0];
  if (!first) return null;
  const title = live.length > 1 ? t("mission.line.others", { title: first.title, count: live.length - 1 }) : first.title;
  return (
    <span className="flex min-w-0 items-center gap-xs text-caption text-muted-foreground" data-mission-row-line="now">
      {first.worker ? <AgentMark kind={first.worker.agent} /> : null}
      <span className="min-w-0 truncate">{t("missionTool.now", { title })}</span>
    </span>
  );
}

function Line({ data, children }: { data: string; children: ReactNode }) {
  return <span className="min-w-0 truncate text-caption text-muted-foreground" data-mission-row-line={data}>{children}</span>;
}

/** `머지 순서 #1 → #2` (B6, B18): each change by its PR mark in its state, or its title where it has no pull request. */
function LandingLine({ nodes, join }: { nodes: MissionNode[]; join: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className="flex min-w-0 items-center gap-xxs overflow-hidden text-caption text-muted-foreground" data-mission-row-line={join ? "join" : "merge"}>
      <span className="shrink-0">{t(join ? "mission.turn.join" : "mission.turn.merge")}</span>
      {nodes.map((node, index) => (
        <span key={node.key} className="flex min-w-0 shrink-0 items-center gap-xxs">
          {index > 0 ? <span aria-hidden="true">→</span> : null}
          {node.pr != null && node.pr_state ? <PrMark state={node.pr_state} number={node.pr} /> : node.pr != null ? <span className="font-mono">#{node.pr}</span> : <span className="max-w-(--size-flow-title) truncate">{node.title}</span>}
        </span>
      ))}
    </span>
  );
}

/** A finished or cancelled mission (B7): ✓ or a grey ✕, its title and the day it closed. */
function ClosedRow({ mission, selected, actions }: { mission: Mission; selected: boolean; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const closed = mission.closed_at_unix_ms;
  const done = mission.status === "done";
  return (
    <RowButton
      aria-label={t("missionTool.open", { title: mission.title })}
      aria-current={selected ? "true" : undefined}
      className={cn("rounded-sm py-xxs text-caption", selected && "bg-secondary")}
      data-mission-row={mission.id}
      data-mission-row-group="closed"
      data-selected={selected ? "true" : undefined}
      onClick={() => actions.openMission(mission.id)}
    >
      {done ? <CheckIcon aria-label={t("mission.pill.done")} className="size-(--size-icon-sm) shrink-0 text-mission-done" /> : <XIcon aria-label={t("mission.pill.cancelled")} className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />}
      <span className="min-w-0 flex-1 truncate text-foreground" title={mission.title}>{mission.title}</span>
      {closed != null ? <span className="shrink-0 tabular-nums text-subtle-foreground">{formatDateTime(language, closed, { month: "numeric", day: "numeric" })}</span> : null}
    </RowButton>
  );
}

/** Under the empty tool (B3): the defaults a new mission starts with, in three lines, and `바꾸기 ›`. */
function DefaultsSummary({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const settings = useShellStore((s) => s.rest?.ui_state?.conductor_settings) ?? DEFAULT_CONDUCTOR;
  const verify = [t(`mission.strength.${settings.default_strength}`), settings.other_vendor ? t("missionTool.otherVendor") : null].filter(Boolean).join(" · ");
  return (
    <section className="mt-auto flex flex-col gap-xs border-t border-border px-md py-sm text-caption" aria-label={t("missionTool.defaults")} data-mission-defaults-summary="true">
      <div className="flex items-center justify-between text-muted-foreground">
        <span>{t("missionTool.defaults")}</span>
        <Button variant="ghost" size="sm" className="text-muted-foreground" data-mission-defaults-change="true" onClick={() => actions.openMissionSettings()}>
          {t("missionTool.change")}
          <ChevronRightIcon aria-hidden="true" />
        </Button>
      </div>
      <Fact label={t("missionTool.cap")} value={t("missionTool.capValue", { count: settings.concurrency })} />
      <Fact label={t("missionTool.scope")} value={t(`mission.scope.${settings.default_scope}`)} />
      <Fact label={t("missionTool.verify")} value={verify} />
    </section>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex min-w-0 items-center justify-between gap-md">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span className="min-w-0 truncate text-foreground">{value}</span>
    </div>
  );
}

/**
 * The defaults, edited in place (B9): the same values, edit events and
 * refusal words as Settings › Conductor, one line saying they apply from the
 * next new mission, and `완료` back to the list.
 */
function Defaults({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { settings, send, error } = useConductorEdit(actions);
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-md overflow-y-auto px-md py-md text-caption" data-mission-defaults="true">
      <p className="text-muted-foreground" data-mission-defaults-applies="true">{t("missionTool.applies")}</p>
      <Setting label={t("conductor.cap")}><CapControl settings={settings} send={send} /></Setting>
      <Setting label={t("conductor.scope")} stacked><ScopeControl settings={settings} send={send} /></Setting>
      <Setting label={t("conductor.strength")} stacked><StrengthControl settings={settings} send={send} /></Setting>
      <Setting label={t("conductor.otherVendor")}><OtherVendorControl settings={settings} send={send} /></Setting>
      <ConductorError error={error} />
      <section className="flex min-w-0 flex-col" aria-label={t("conductor.candidates")} data-conductor-candidates={String(settings.candidates.length)}>
        <div className="flex items-center justify-between gap-xs">
          <span className="text-subhead text-foreground">{t("conductor.candidates")} <span className="tabular-nums text-subtle-foreground">{settings.candidates.length}</span></span>
          <CandidateAdd settings={settings} send={send} />
        </div>
        <CandidateRows settings={settings} send={send} layout="stack" />
      </section>
      <div className="mt-auto flex justify-end">
        <Button size="sm" variant="secondary" data-mission-defaults-done="true" onClick={() => useUiStore.getState().setMissionToolPage("list")}>{t("missionTool.done")}</Button>
      </div>
    </div>
  );
}

function Setting({ label, stacked = false, children }: { label: string; stacked?: boolean; children: ReactNode }) {
  return (
    <div className={cn("flex min-w-0 gap-xs", stacked ? "flex-col" : "items-center justify-between")}>
      <span className="text-subhead text-foreground">{label}</span>
      {children}
    </div>
  );
}
