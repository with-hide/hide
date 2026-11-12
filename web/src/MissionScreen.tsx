import { CheckIcon, ChevronRightIcon, SettingsIcon, XIcon } from "lucide-react";
import { HeldMark, useDelayPill } from "./components/mission-hold";
import { useEffect, useLayoutEffect, useMemo, useRef, type KeyboardEvent, type ReactNode } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { KindHelpButton } from "./components/kind-help-dialog";
import { KindIcon, KindMark } from "./components/kind-mark";
import { GateAheadCount } from "./components/mission-gate";
import { useElapsed } from "./components/elapsed";
import { PILL_TONE, ProgressCells, STATE_PILL_TONE } from "./components/mission-tone";
import { PrMark } from "./components/pr-mark";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { RowButton, RowOverlayButton } from "./components/ui/row-button";
import { ToggleGroup, ToggleGroupItem } from "./components/ui/toggle-group";
import { Hint } from "./components/ui/tooltip";
import { deviceConnected, frontDeviceId } from "./devices";
import { useInterfaceTranslation } from "./i18n/client";
import { formatDateTime } from "./i18n/format";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { MissionMap } from "./MissionMap";
import { MissionStartGuide } from "./MissionStartGuide";
import { MissionTakeOver } from "./MissionTakeOver";
import {
  CLOSED_SHOWN,
  deviceMissions,
  groupMissions,
  listedMissions,
  missionEntry,
  missionPillKey,
  missionProjects,
  rowLine,
  rowLineText,
  STATE_PILL,
  turnCount,
  type MissionEntry,
  type MissionGroup,
  type MissionProject,
} from "./missionList";
import { mergePull, gatesAhead, heldNodes, missionCells, missionCounts, type TurnItem } from "./missions";
import { MissionNumber } from "./missionParts";
import { useWorkerEscalation } from "./missionWorkers";
import { machineName } from "./screenMachine";
import { catalogOf, type Mission, type MissionNode } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// The mission screen (PRD mission-screen D-01-D-11, B1-B23): one device's
// missions, every project of it, as a table in the Mission tool's four
// groups; a mission opened from it narrows the table to a list beside its
// detail, the map as it has always drawn. A mission pressed anywhere else
// opens as a card in the 보기 slot instead (operator decision 2026-11-09). It is the
// page's own navigation, like the Overview and Desk: where it stands is
// `MissionPlace`, and the core hears only which mission the detail shows,
// so it sends that mission's events (D-16). Every state and number is the
// core's mission record; nothing here edits a mission.

/** The device's missions, each in its group, or what stands in for them: none yet read, or a record that could not be read (B8). */
type DeviceMissions = { state: "loading" } | { state: "failed" } | { state: "ready"; entries: MissionEntry[] };

const LOADING: DeviceMissions = { state: "loading" };
const FAILED: DeviceMissions = { state: "failed" };

/** `deviceId`'s missions in their groups (D-06); one value per snapshot change of the missions or the workers, never per terminal frame. */
export function useDeviceMissions(deviceId: string): DeviceMissions {
  const missions = useShellStore((s) => s.rest?.missions);
  const escalationOf = useWorkerEscalation();
  return useMemo(() => {
    if (!missions) return LOADING;
    if (missions.failed) return FAILED;
    return { state: "ready", entries: deviceMissions(missions.items, deviceId).map((mission) => missionEntry(mission, escalationOf)) };
  }, [missions, deviceId, escalationOf]);
}

/** The sidebar row's number (B1): the front device's missions with anything in their `내 차례`. */
export function useMissionTurnCount(): number {
  const deviceId = useShellStore((s) => frontDeviceId(s.rest));
  const missions = useDeviceMissions(deviceId);
  return missions.state === "ready" ? turnCount(missions.entries) : 0;
}

export function MissionScreen({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const place = useUiStore((s) => s.missions);
  const front = useShellStore((s) => frontDeviceId(s.rest));
  const deviceId = place.deviceId ?? front;
  const missions = useDeviceMissions(deviceId);
  const connected = useShellStore((s) => deviceConnected(s.rest, deviceId));
  const device = useShellStore((s) => machineName(s.rest?.navigator?.devices, deviceId, t));
  const workspaces = useShellStore((s) => s.rest?.navigator?.workspaces);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const catalog = useMemo(() => catalogOf(workspaces, remote), [workspaces, remote]);
  const entries = missions.state === "ready" ? missions.entries : null;
  const projects = useMemo(() => (entries ? missionProjects(entries.map((entry) => entry.mission), catalog) : []), [entries, catalog]);
  // A chip whose project has no mission on this device any more reads as every project.
  const projectId = place.projectId !== null && projects.some((project) => project.id === place.projectId) ? place.projectId : null;
  const shown = useMemo(() => (entries && projectId ? entries.filter((entry) => entry.mission.workspace_id === projectId) : entries), [entries, projectId]);
  const groups = useMemo(() => (shown ? groupMissions(shown) : []), [shown]);
  const selected = entries?.find((entry) => entry.mission.id === place.selected) ?? null;
  const detail = place.selected !== null && missions.state !== "failed" ? place.selected : null;

  // A mission removed or no longer this device's while its detail shows returns to the table, without a word (B22).
  useEffect(() => {
    if (place.selected !== null && entries && !selected) useUiStore.getState().setMissions({ selected: null });
  }, [place.selected, entries, selected]);
  // The core sends the events of the mission the detail shows, and none once the screen goes (D-16).
  // A dispatch made while the connection is down is dropped and a new core starts with none, so the
  // value is sent again each time the connection comes back. A page that never named a mission sends
  // no none either, so opening the table leaves another window's detail its events (last word wins).
  const live = useShellStore((s) => s.connection === "live");
  const named = useRef(false);
  useEffect(() => {
    if (!live || (detail === null && !named.current)) return;
    actions.missionShown(detail);
    named.current = detail !== null;
  }, [actions, detail, live]);
  useEffect(() => () => {
    if (named.current) actions.missionShown(null);
  }, [actions]);

  const projectLabel = (id: string) => projects.find((project) => project.id === id)?.label ?? null;
  return (
    <section
      className="@container/missions flex min-h-0 min-w-0 flex-1 flex-col bg-background text-foreground"
      aria-label={t("missionScreen.list", { device })}
      data-mission-screen={missions.state}
      data-mission-screen-view={detail ? "detail" : "table"}
      data-mission-screen-device={deviceId}
    >
      <Head
        device={device}
        count={shown?.length ?? null}
        connected={connected}
        projects={projects}
        projectId={projectId}
        selectedProject={selected?.mission.workspace_id ?? null}
      />
      {missions.state === "loading" ? (
        <Skeleton />
      ) : missions.state === "failed" ? (
        // The reason is the core's diagnostic; the operator can do nothing with it here (design principle 13).
        <p className="px-lg py-md text-caption text-muted-foreground" data-mission-screen-state="failed">{t("missionScreen.failed")}</p>
      ) : detail && selected ? (
        <div className="flex min-h-0 min-w-0 flex-1 flex-col missions-split:flex-row" data-mission-split="true">
          <List groups={groups} selected={selected.mission.id} connected={connected} actions={actions} />
          <Detail mission={selected.mission} project={projectLabel(selected.mission.workspace_id)} actions={actions} />
        </div>
      ) : (
        <Table groups={groups} empty={entries?.length === 0} filtered={projectId !== null} connected={connected} projectLabel={projectLabel} actions={actions} />
      )}
    </section>
  );
}

/**
 * The head (B3, B9): `미션 · <device> · N개`, the device's `연결 안 됨` while it does not
 * answer, and the project chips, `모든 프로젝트` first.
 */
function Head({ device, count, connected, projects, projectId, selectedProject }: {
  device: string;
  count: number | null;
  connected: boolean;
  projects: readonly MissionProject[];
  projectId: string | null;
  /** The project of the mission the detail shows, if any. */
  selectedProject: string | null;
}) {
  const { t } = useInterfaceTranslation();
  return (
    <header className="flex shrink-0 flex-col gap-xs border-b border-border px-lg py-sm" data-mission-screen-head="true">
      <div className="flex min-w-0 flex-wrap items-center gap-x-md gap-y-xs">
        <h1 className="flex min-w-0 items-baseline gap-xs text-subhead">
          <span className="shrink-0 font-semibold text-foreground">{t("missionScreen.title")}</span>
          <span className="min-w-0 truncate text-body text-muted-foreground" data-mission-screen-device-name="true">· {device}</span>
          {count !== null ? <span className="shrink-0 text-body tabular-nums text-muted-foreground" data-mission-screen-count={count}>· {t("missionScreen.count", { count })}</span> : null}
        </h1>
        {connected ? null : <Badge variant="outline" className="text-muted-foreground" data-mission-screen-offline="true">{t("missionScreen.offline")}</Badge>}
        {projects.length > 1 ? (
          <ToggleGroup
            type="single"
            variant="pill"
            className="min-w-0"
            aria-label={t("missionScreen.projects")}
            value={projectId ?? ""}
            onValueChange={(value) => {
              const next = value === "" ? null : value;
              // A chip that leaves out the mission the detail shows returns to its table, so the list never loses the mission it shows (B10).
              const keeps = selectedProject === null || next === null || next === selectedProject;
              useUiStore.getState().setMissions({ projectId: next, scroll: 0, ...(keeps ? {} : { selected: null }) });
            }}
          >
            <ToggleGroupItem variant="pill" size="sm" value="" data-mission-project-chip="all">{t("missionScreen.allProjects")}</ToggleGroupItem>
            {projects.map((project) => (
              <ToggleGroupItem key={project.id} variant="pill" size="sm" value={project.id} className="max-w-(--size-mission-table-project)" data-mission-project-chip={project.id}>
                <span className="min-w-0 truncate" title={project.label}>{project.label}</span>
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
        ) : null}
      </div>
    </header>
  );
}

/** Quiet rows before the core's missions first arrive (B8): no count, no words. */
function Skeleton() {
  return (
    <div className="flex flex-col gap-sm px-lg py-md" aria-hidden="true" data-mission-screen-state="loading">
      {[0, 1, 2].map((row) => (
        <div key={row} className="flex items-center gap-md">
          <span className="h-(--size-badge-height) flex-1 rounded-xs bg-secondary" />
          <span className="h-(--size-badge-height) w-(--size-mission-table-project) rounded-xs bg-secondary" />
          <span className="h-(--size-badge-height) w-(--size-mission-table-time) rounded-xs bg-secondary" />
        </div>
      ))}
    </div>
  );
}

/** The table's columns; a screen too narrow for the list beside the detail leaves out the project, which the chips still name (B23). */
const TABLE_COLUMNS = "grid grid-cols-[minmax(0,1fr)_var(--size-mission-table-steps)_var(--size-mission-table-time)] missions-split:grid-cols-[minmax(0,1fr)_var(--size-mission-table-project)_var(--size-mission-table-steps)_var(--size-mission-table-time)] items-center gap-x-md";

/**
 * The table (B2-B7): the groups with their counts, an empty group not drawn,
 * 끝남 cut to its newest three until `모두 ›`; with no mission the start
 * guide, with none in the chosen project a line and `모든 프로젝트`. Its
 * scroll is kept, so ✕ comes back to it (B11).
 */
function Table({ groups, empty, filtered, connected, projectLabel, actions }: {
  groups: { group: MissionGroup; entries: MissionEntry[] }[];
  empty: boolean;
  filtered: boolean;
  connected: boolean;
  projectLabel: (id: string) => string | null;
  actions: Actions;
}) {
  const { t } = useInterfaceTranslation();
  const scroller = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (scroller.current) scroller.current.scrollTop = useUiStore.getState().missions.scroll;
  }, []);
  if (empty) {
    return (
      <div className="flex min-h-0 flex-1 flex-col items-center overflow-y-auto px-lg py-xl" data-mission-screen-state="empty">
        <p className="pb-md text-body text-muted-foreground">{t("missionScreen.empty")}</p>
        <div className="w-full max-w-(--size-mission-popover)"><MissionStartGuide /></div>
      </div>
    );
  }
  if (groups.length === 0 && filtered) {
    return (
      <div className="flex flex-col items-start gap-xs px-lg py-md text-caption text-muted-foreground" data-mission-screen-state="no-match">
        <p>{t("missionScreen.noMatch")}</p>
        <Button variant="text" size="inline" onClick={() => useUiStore.getState().setMissions({ projectId: null })}>{t("missionScreen.allProjects")}</Button>
      </div>
    );
  }
  return (
    <div
      ref={scroller}
      className="min-h-0 flex-1 overflow-y-auto pb-lg"
      onScroll={(event) => useUiStore.getState().setMissions({ scroll: event.currentTarget.scrollTop })}
      data-mission-screen-state="table"
    >
      <div className={cn(TABLE_COLUMNS, "sticky top-0 z-10 border-b border-border/60 bg-background px-[calc(var(--spacing-lg)+var(--spacing-xs))] py-xs text-caption text-muted-foreground")} aria-hidden="true">
        <span>{t("missionScreen.col.mission")}</span>
        <span className="hidden missions-split:block">{t("missionScreen.col.project")}</span>
        <span>{t("missionScreen.col.steps")}</span>
        <span className="text-right">{t("missionScreen.col.time")}</span>
      </div>
      {groups.map(({ group, entries }) => (
        <TableGroup key={group} group={group} entries={entries} connected={connected} projectLabel={projectLabel} actions={actions} />
      ))}
    </div>
  );
}

/** A group's head: its name, its count, and for 끝남 `모두 ›` once it holds more than three. */
function GroupHead({ group, count, className }: { group: MissionGroup; count: number; className?: string }) {
  const { t } = useInterfaceTranslation();
  const all = useUiStore((s) => s.missions.closedAll);
  return (
    <h2 className={cn("flex items-center gap-xs text-caption font-semibold", group === "turn" ? "text-mission-hold" : "text-muted-foreground", className)}>
      <span>{t(`missionScreen.group.${group}`)}</span>
      <span className="font-normal tabular-nums text-subtle-foreground">{count}</span>
      {group === "closed" && count > CLOSED_SHOWN ? (
        <Button variant="text" size="inline" className="ml-auto gap-0" aria-expanded={all} data-mission-closed-all={all ? "open" : "folded"} onClick={() => useUiStore.getState().setMissions({ closedAll: !all })}>
          {all ? t("missionScreen.fewer") : t("missionScreen.all")}
          {all ? null : <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
        </Button>
      ) : null}
    </h2>
  );
}

function TableGroup({ group, entries, connected, projectLabel, actions }: { group: MissionGroup; entries: MissionEntry[]; connected: boolean; projectLabel: (id: string) => string | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const all = useUiStore((s) => s.missions.closedAll);
  const rows = group === "closed" && !all ? entries.slice(0, CLOSED_SHOWN) : entries;
  return (
    <section className="flex min-w-0 flex-col border-t border-border/60 px-lg pt-md pb-sm first-of-type:border-t-0" aria-label={`${t(`missionScreen.group.${group}`)} ${entries.length}`} data-mission-group={group} data-mission-group-count={entries.length}>
      <GroupHead group={group} count={entries.length} className="px-xs pb-xs" />
      {rows.map((entry) => <TableRow key={entry.mission.id} entry={entry} connected={connected} project={projectLabel(entry.mission.workspace_id)} actions={actions} />)}
    </section>
  );
}

/**
 * One mission (B5, B6, B9): its number, state, title and one line; its
 * project; a cell per step with `n/m`; how long it has run, or the day it
 * closed. `이어받기` takes a mission no Conductor holds without opening it;
 * the rest of the row opens it. A device that does not answer dims the row
 * and holds its actions.
 */
/** A row's action keeps the line's height so it never outgrows the line it sits in, and keeps a button's square corners and outline so it never reads as the row's state pill. */
const TABLE_ACTION = "-my-px h-auto rounded-xs px-xs text-caption leading-normal";

function TableRow({ entry, connected, project, actions }: { entry: MissionEntry; connected: boolean; project: string | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { mission, group, turn } = entry;
  const closed = group === "closed";
  const stale = mission.stale || !connected;
  return (
    <div className={cn("relative min-w-0 rounded-xs px-xs py-xs", TABLE_COLUMNS, stale && "opacity-(--opacity-dimmed)")} data-mission-row={mission.id} data-mission-row-group={group} data-mission-row-stale={stale || undefined}>
      <RowOverlayButton aria-label={t("missionScreen.open", { title: mission.title })} onClick={() => actions.showMissionOnScreen(mission.id)} />
      <span className="pointer-events-none relative flex min-w-0 flex-col gap-xxs">
        <span className="flex min-w-0 items-center gap-xs">
          <MissionNumber mission={mission} className="text-caption" />
          {closed ? <ClosedMark mission={mission} /> : <Pill mission={mission} turn={turn[0] ?? null} />}
          <span className={cn("min-w-0 truncate text-body", closed ? "text-subtle-foreground" : "font-semibold text-foreground")} title={mission.title}>{mission.title}</span>
        </span>
        {closed ? null : (
          <span className="flex min-w-0 items-center gap-xs">
            <RowLine mission={mission} turn={turn[0] ?? null} />
            {turn[0]?.kind === "takeover" ? <span className={cn("shrink-0", stale ? "pointer-events-none" : "pointer-events-auto")}><MissionTakeOver mission={mission} actions={actions} disabled={stale} className={TABLE_ACTION} /></span> : null}
          </span>
        )}
      </span>
      <span className="pointer-events-none relative hidden min-w-0 truncate text-caption text-subtle-foreground missions-split:block" title={project ?? undefined} data-mission-row-project={mission.workspace_id}>{project}</span>
      <Steps mission={mission} onOpen={() => actions.showMissionOnScreen(mission.id)} />
      <span className="pointer-events-none relative text-right text-caption text-muted-foreground"><Time mission={mission} /></span>
    </div>
  );
}

/**
 * How many places the plan still has the operator look, a quiet grey eye and
 * number at the row's end with `내가 볼 곳 N` on hover (PRD mission-human-gate
 * D-18, B18); none when none is ahead, and never in 내 차례's colour or count.
 * Over the table row's overlay it takes the pointer for its hover, so its press
 * opens the mission as the row does (`onOpen`).
 */
function GateCount({ mission, onOpen }: { mission: Mission; onOpen?: () => void }) {
  const { t } = useInterfaceTranslation();
  const ahead = gatesAhead(mission).length;
  if (ahead === 0) return null;
  return (
    <Hint label={t("mission.gate.planTitle", { count: ahead })}>
      <span className={cn("flex shrink-0", onOpen && "pointer-events-auto")} onClick={onOpen}><GateAheadCount count={ahead} /></span>
    </Hint>
  );
}

function Steps({ mission, onOpen }: { mission: Mission; onOpen: () => void }) {
  const { done, total } = missionCounts(mission);
  if (mission.nodes.length === 0) return <span />;
  return (
    <span className="pointer-events-none relative flex min-w-0 items-center gap-xs text-caption text-muted-foreground">
      <ProgressCells tones={missionCells(mission)} size="chip" className="min-w-0 overflow-hidden" />
      <span className="shrink-0 font-mono tabular-nums" data-mission-row-counts={`${done}/${total}`}>{done}/{total}</span>
      <GateCount mission={mission} onOpen={onOpen} />
      <HoldCount mission={mission} />
    </span>
  );
}

/**
 * How many nodes its Conductor holds back, in quiet words at the row's end
 * (`보류 1`, named `보류한 노드 1개`); never in 내 차례's colour or count.
 * A mission's delay is its pill's to say. None when no node is held.
 */
function HoldCount({ mission }: { mission: Mission }) {
  const { t } = useInterfaceTranslation();
  const held = heldNodes(mission);
  if (held === 0) return null;
  return (
    <span className="shrink-0 whitespace-nowrap" title={t("mission.holdCount", { count: held })} data-mission-row-held={held}>
      {t("mission.holdShort", { count: held })}
    </span>
  );
}

/** How long an open mission has run, none while its device does not answer; a closed one's day. */
function Time({ mission }: { mission: Mission }) {
  const { i18n } = useInterfaceTranslation();
  const open = mission.closed_at_unix_ms == null;
  const elapsed = useElapsed(open && !mission.stale ? (mission.started_at_unix_ms ?? mission.created_at_unix_ms) : null);
  if (!open) return <span className="tabular-nums" data-mission-row-closed="true">{formatDateTime(requireInterfaceLanguage(i18n.language), mission.closed_at_unix_ms!, { month: "numeric", day: "numeric" })}</span>;
  return elapsed ? <span data-mission-row-elapsed="true">{elapsed}</span> : null;
}

/** A finished or cancelled mission's mark: ✓ or a grey ✕. */
function ClosedMark({ mission }: { mission: Mission }) {
  const { t } = useInterfaceTranslation();
  return mission.status === "done"
    ? <CheckIcon aria-label={t("mission.pill.done")} className="size-(--size-icon-sm) shrink-0 text-mission-done" />
    : <XIcon aria-label={t("mission.pill.cancelled")} className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />;
}

/** The state pill: what the operator does on their turn, in the decision colour, else the map head's own pill; a takeover row draws none, since its 이어받기 button is the action. */
function Pill({ mission, turn }: { mission: Mission; turn: TurnItem | null }) {
  const { t } = useInterfaceTranslation();
  const held = useDelayPill(mission);
  if (turn?.kind === "takeover") return null;
  if (turn) {
    return <span className={cn("flex shrink-0 items-center rounded-full px-sm text-caption font-semibold", PILL_TONE.hold)} data-mission-row-pill={turn.kind}>{t(missionPillKey(mission, turn))}</span>;
  }
  // A delay holds every new start: the pill says so, with how long, in place of 진행 중.
  if (held) {
    return (
      <span className={cn("flex shrink-0 items-center gap-xxs rounded-full px-sm text-caption font-semibold", PILL_TONE.quiet)} data-mission-row-pill="held">
        <HeldMark />
        {held}
      </span>
    );
  }
  return (
    <span className={cn("flex shrink-0 items-center gap-xxs rounded-full px-sm text-caption font-semibold", STATE_PILL_TONE[mission.status])} data-mission-row-pill={mission.status}>
      {mission.status === "done" ? <CheckIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : null}
      {t(STATE_PILL[mission.status])}
    </span>
  );
}

/** The row's one line (B5): `rowLine` decides it and `rowLineText` words it, as on the phone; this adds the worker's mark and the PR marks. */
function RowLine({ mission, turn }: { mission: Mission; turn: TurnItem | null }) {
  const { t } = useInterfaceTranslation();
  const line = rowLine(mission, turn);
  if (!line) return null;
  switch (line.kind) {
    case "landing":
      return <LandingLine nodes={line.nodes} join={line.join} />;
    case "now":
      return (
        <span className="flex min-w-0 items-center gap-xs text-caption text-muted-foreground" data-mission-row-line="now">
          {line.first.worker ? <AgentMark kind={line.first.worker.agent} /> : null}
          <span className="min-w-0 truncate">{rowLineText(line, t)}</span>
        </span>
      );
    default:
      return <Line data={line.kind}>{rowLineText(line, t)}</Line>;
  }
}

function Line({ data, children }: { data: string; children: ReactNode }) {
  return <span className="min-w-0 truncate text-caption text-muted-foreground" data-mission-row-line={data}>{children}</span>;
}

/** `머지 순서 #1 → #2`: each change by its PR mark in its state, or its title where it has no pull request. */
function LandingLine({ nodes, join }: { nodes: MissionNode[]; join: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className="flex min-w-0 items-center gap-xxs overflow-hidden text-caption text-muted-foreground" data-mission-row-line={join ? "join" : "merge"}>
      <span className="shrink-0">{t(join ? "mission.turn.join" : "mission.turn.merge")}</span>
      {nodes.map((node, index) => {
        const pr = join ? { number: node.pr, state: node.pr_state, more: 0 } : mergePull(node);
        return (
          <span key={node.key} className="flex min-w-0 shrink-0 items-center gap-xxs">
            {index > 0 ? <span aria-hidden="true">→</span> : null}
            {pr.number != null && pr.state ? <PrMark state={pr.state} number={pr.number} /> : pr.number != null ? <span className="font-mono">#{pr.number}</span> : <span className="max-w-(--size-flow-title) truncate">{node.title}</span>}
            {pr.more > 0 ? <span>+{pr.more}</span> : null}
          </span>
        );
      })}
    </span>
  );
}

/**
 * The list beside the detail (B10, B11): the table's groups and order, the
 * mission shown selected; ↑ and ↓ show the previous or next one. In a window
 * too narrow for it beside the detail it stands above (B23).
 */
function List({ groups, selected, connected, actions }: { groups: { group: MissionGroup; entries: MissionEntry[] }[]; selected: string; connected: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const all = useUiStore((s) => s.missions.closedAll);
  const order = useMemo(() => listedMissions(groups, all), [groups, all]);
  const box = useRef<HTMLDivElement>(null);
  // Opened from the table, whose row went with it, or from another screen, the keys land on the mission shown, so ↑ and ↓ work at once (B11).
  useEffect(() => {
    const active = document.activeElement;
    if (!active || active === document.body) box.current?.querySelector<HTMLElement>('[aria-current="true"]')?.focus();
  }, []);
  const step = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    const index = order.findIndex((entry) => entry.mission.id === selected);
    const next = order[index + (event.key === "ArrowDown" ? 1 : -1)];
    event.preventDefault();
    if (!next) return;
    actions.showMissionOnScreen(next.mission.id);
    requestAnimationFrame(() => box.current?.querySelector<HTMLElement>(`[data-mission-list-row="${CSS.escape(next.mission.id)}"]`)?.focus());
  };
  return (
    <div
      ref={box}
      className="flex max-h-(--size-mission-popover) min-w-0 shrink-0 flex-col overflow-y-auto border-b border-border px-xs pb-sm missions-split:max-h-[none] missions-split:w-(--size-mission-list) missions-split:border-r missions-split:border-b-0"
      onKeyDown={step}
      data-mission-list="true"
    >
      {groups.map(({ group, entries }) => (
        <section key={group} className="flex min-w-0 flex-col pt-sm" aria-label={`${t(`missionScreen.group.${group}`)} ${entries.length}`} data-mission-group={group}>
          <GroupHead group={group} count={entries.length} className="px-sm pb-xxs" />
          {(group === "closed" && !all ? entries.slice(0, CLOSED_SHOWN) : entries).map((entry) => (
            <ListRow key={entry.mission.id} entry={entry} selected={entry.mission.id === selected} connected={connected} actions={actions} />
          ))}
        </section>
      ))}
    </div>
  );
}

function ListRow({ entry, selected, connected, actions }: { entry: MissionEntry; selected: boolean; connected: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { mission, group, turn } = entry;
  const stale = mission.stale || !connected;
  return (
    <RowButton
      aria-label={t("missionScreen.open", { title: mission.title })}
      aria-current={selected ? "true" : undefined}
      className={cn("flex-col items-stretch gap-xxs px-sm py-sm", selected && "bg-selected hover:bg-selected", stale && "opacity-(--opacity-dimmed)")}
      onClick={() => actions.showMissionOnScreen(mission.id)}
      data-mission-list-row={mission.id}
      data-selected={selected ? "true" : undefined}
    >
      <span className="flex min-w-0 items-center gap-xs">
        {group === "closed" ? <ClosedMark mission={mission} /> : <KindIcon kind="mission" className="size-(--size-icon-sm)" />}
        <span className={cn("min-w-0 flex-1 truncate text-body", group === "closed" ? "text-subtle-foreground" : "text-foreground")} title={mission.title}>{mission.title}</span>
        <span className="shrink-0 text-caption text-muted-foreground"><Time mission={mission} /></span>
      </span>
      {group !== "closed" && ((group === "turn" && turn[0]) || mission.nodes.length > 0) ? (
        <span className="flex min-w-0 items-center gap-xs pl-[calc(var(--size-icon-sm)+var(--spacing-xs))]">
          {group === "turn" && turn[0] ? (
            <span className="min-w-0 flex-1 truncate text-caption text-mission-hold">{t(missionPillKey(mission, turn[0]))}</span>
          ) : (
            <span className="min-w-0 flex-1"><ProgressCells tones={missionCells(mission)} size="chip" /></span>
          )}
          <GateCount mission={mission} />
        </span>
      ) : null}
    </RowButton>
  );
}

/**
 * A mission's detail (B10): the kind and title with its project, ⚙ for
 * Settings › Mission and ✕ back to the table (B11); under it the map as it
 * has always drawn, its head, Conductor row, 내 차례, the map and its summary.
 */
function Detail({ mission, project, actions }: { mission: Mission; project: string | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col" aria-label={mission.title} data-mission-screen-detail={mission.id}>
      <div className="flex h-(--size-tab-strip) shrink-0 items-center gap-xs border-b border-border px-md text-caption" data-mission-screen-detail-head="true">
        <KindMark kind="mission" />
        <KindHelpButton from="mission" />
        <span className="min-w-0 truncate text-body font-semibold" title={mission.title}>{mission.title}</span>
        {project ? <span className="min-w-0 shrink truncate text-subtle-foreground" title={project}>{project}</span> : null}
        <span className="ml-auto flex shrink-0 items-center gap-xxs">
          <Hint label={t("missionScreen.settings")}>
            <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("missionScreen.settings")} onClick={() => actions.openMissionSettings()} data-mission-screen-settings="true">
              <SettingsIcon />
            </Button>
          </Hint>
          <Hint label={t("missionScreen.close")}>
            <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("missionScreen.close")} onClick={() => useUiStore.getState().setMissions({ selected: null })} data-mission-screen-close="true">
              <XIcon />
            </Button>
          </Hint>
        </span>
      </div>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col" key={mission.id}>
        <MissionMap missionId={mission.id} actions={actions} />
      </div>
    </section>
  );
}
