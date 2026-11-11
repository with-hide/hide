import { CogIcon, KeyRoundIcon, PauseIcon, TriangleAlertIcon } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useState } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { DependencyGraphView, type EdgeLook, type OverlayEdge } from "./DependencyGraph";
import type { GraphEdge } from "./dependencyLayout";
import { formatElapsed, useElapsed, useMinuteClock, useRemaining } from "./components/elapsed";
import { ConductorRow } from "./components/mission-conductor-row";
import { MapPage, MapSplit } from "./components/map-page";
import { MissionEnd } from "./components/mission-end";
import { GatePlanList, HumanStrip, mergeWords } from "./components/mission-gate";
import { MissionNodeHover } from "./components/mission-node-hover";
import { MissionDestinationTag, MissionOptionsLine } from "./components/mission-options";
import { StationLine } from "./components/mission-stations";
import { MissionNodeMark } from "./components/mission-node-mark";
import { PullList, ResultTiles } from "./components/mission-results";
import { satelliteLines, Satellites } from "./components/mission-satellites";
import { MissionSummary } from "./components/mission-summary";
import { HeldMark, useDelayPill } from "./components/mission-hold";
import { HEAD_FILL, PILL_TONE, ProgressCells, STATE_PILL_TONE, STEP_FILL, WorkPill } from "./components/mission-tone";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { MarkdownText } from "./MarkdownText";
import { markdownPlainText } from "./markdownPlain";
import { useMapPick } from "./mapPick";
import { MissionMapCanvas } from "./MissionMapCanvas";
import { MissionNodePanel } from "./MissionNodePanel";
import { MissionTurn } from "./MissionTurn";
import { markWord, MissionNumber, ResultTail, useMissionDeviceName, useNodeRun, useNodeTimes } from "./missionParts";
import { mergeOverridden, nodeStations, stationNode, stationWords } from "./missionStations";
import { landsInDestination, missionOptions } from "./missionOptions";
import { DEFAULT_CONDUCTOR } from "./conductorSettings";
import { STATE_PILL } from "./missionList";
import { planCount } from "./plans";
import { humanTurnText } from "./humanTurn";
import { useWorkerEscalation } from "./missionWorkers";
import {
  attentionWords,
  defectEdges,
  gatesAhead,
  gateWaitedLong,
  openAttention,
  hideWords,
  missionHeadCells,
  missionCounts,
  missionEventsOf,
  missionOf,
  missionOpen,
  neighbours,
  nodeHold,
  nodeLook,
  nodeSteps,
  nodeWork,
  nodeWorkWords,
  stopCauseWords,
  nodeResult,
  reportLine,
  stackWaiting,
  warningWords,
  type NodeLook,
  type NodeResult,
} from "./missions";
import { layerDependencies } from "./projectBoard";
import { machineName } from "./screenMachine";
import type { Mission, MissionEvent, MissionMapFlow, MissionNode, MissionNodeHold, MissionNodeWork } from "./snapshot";
import { useShellStore } from "./store";
import type { TFunction } from "i18next";

// A mission's page in the lane's 보기 slot (PRD mission-map-git-criteria
// D-01-D-06, B24-B35; PRD mission-tool B12-B25): the head (state, time, a
// cell per node, `n / m`); a finished mission's results, its pull requests
// and its report's line; the Conductor row; what only the operator can do
// (내 차례, B25); the DAG; and beside it the summary, or the panel of the
// node the operator picked, beside the map when the page is wide and under
// it when narrow (B34). Picking a card keeps it and the cards one edge away
// lit and dims the rest (B30). Every value is the core's (`MissionView`, its
// events); this page words and draws it and edits nothing.

/** The card that stands for the report every node ends in; a node key is letters, digits, `-`, `_` and `.`, so none is this. */
const REPORT = "#report";

/**
 * `onNode` hears a node card picked, so a page that holds the map beside a
 * thread can show the node's own (Desk, PRD session-docs-and-assistant B34).
 */
export function MissionMap({ missionId, actions, onNode }: { missionId: string; actions: Actions; onNode?: (node: MissionNode) => void }) {
  const { t } = useInterfaceTranslation();
  const mission = useShellStore((s) => missionOf(s.rest, missionId));
  const failed = useShellStore((s) => s.rest?.missions?.failed ?? null);
  if (failed) return <MapState text={t("mission.unreadable")} tone="text-destructive" state="unreadable" />;
  if (!mission) return <MapState text={t("mission.gone")} state="gone" />;
  // Another mission is another page: it opens at 맞춤 with nothing picked.
  return <MissionBody key={mission.id} mission={mission} actions={actions} onNode={onNode} />;
}

function MapState({ text, tone, state }: { text: string; tone?: string; state: string }) {
  return <p className={cn("px-md py-md text-body text-muted-foreground", tone)} data-mission-state={state}>{text}</p>;
}

function MissionBody({ mission, actions, onNode }: { mission: Mission; actions: Actions; onNode?: (node: MissionNode) => void }) {
  const { t } = useInterfaceTranslation();
  const [root, rootRef] = useState<HTMLDivElement | null>(null);
  const [page, pageRef] = useState<HTMLDivElement | null>(null);
  const wide = useWide(root);
  const device = useShellStore((s) => (mission.stale ? machineName(s.rest?.navigator?.devices, mission.device_id, t) : null));
  const log = useShellStore((s) => missionEventsOf(s.rest, mission.id));
  // The map names each item's model as the start panel's list does (PRD agent-profiles-phase2 B12): those lists are read while a screen
  // reads them, so the map observes them while it is drawn, again after a reconnect drops hided's side of it.
  const live = useShellStore((s) => s.connection === "live");
  useEffect(() => (live ? actions.observeCatalog() : undefined), [actions, live]);
  // Settings › Mission keeps which way every mission's map runs.
  const flow = useShellStore((s) => s.rest?.ui_state?.conductor_settings?.map_flow) ?? "down";
  const open = missionOpen(mission);
  // The page's clock ticks by the minute; a closed mission's times end where it closed.
  const minute = useMinuteClock();
  const end = mission.closed_at_unix_ms ?? minute;
  // A pick whose node a replan took away is no pick.
  const { selected, select, panel, onKeyDown, onMapClick } = useMapPick(
    mission.nodes.map((one) => one.key),
    "[data-mission-node], [data-node-satellites]",
  );
  const node = selected ? mission.nodes.find((candidate) => candidate.key === selected)! : null;
  return (
    <MapPage
      pageRef={rootRef}
      scrollRef={pageRef}
      data={{ "data-mission": mission.id, "data-mission-status": mission.status, "data-mission-stale": mission.stale || undefined }}
      scrollData={{ "data-mission-scroll": "true" }}
      onKeyDown={onKeyDown}
    >
      <Head mission={mission} actions={actions} />
      {/* A finished mission leads with what it left (B20); its report's line follows, then its Conductor. */}
      {!open ? <ResultTiles mission={mission} /> : null}
      {!open ? <PullList mission={mission} onSelect={select} /> : null}
      {mission.report && !open ? <ReportLine mission={mission} /> : null}
      <ConductorRow mission={mission} actions={actions} />
      <MissionTurn mission={mission} device={device} actions={actions} onSelect={select} />
      <GatePlan mission={mission} onSelect={select} />
      {mission.delay && open ? <DelayLine line={mission.delay} until={mission.delay_until_unix_ms} /> : null}
      {device ? <p className="px-md pb-xs text-caption text-muted-foreground" data-mission-stale-note="true">{t("mission.stale", { device })}</p> : null}
      {mission.nodes.length === 0 ? (
        <p className="px-md py-sm text-body text-muted-foreground" data-mission-planning="true">{t("mission.planning")}</p>
      ) : (
        <MapSplit
          dimmed={mission.stale}
          data={{ "data-mission-values": mission.stale ? "stale" : "live" }}
          onMapClick={onMapClick}
          mapData={{ "data-mission-map": "true" }}
          map={
            <MissionMapCanvas wide={wide} page={page} flow={flow} onFlow={(next) => actions.setConductorSettings({ map_flow: next })}>
              <Graph
                actions={actions}
                flow={flow}
                mission={mission}
                events={log?.events ?? null}
                end={end}
                selected={selected}
                onSelect={(key) => {
                  select(selected === key ? null : key);
                  const picked = selected === key ? undefined : mission.nodes.find((one) => one.key === key);
                  if (picked) onNode?.(picked);
                }}
              />
            </MissionMapCanvas>
          }
          sideRef={panel}
          sideData={{ "data-mission-panel": node?.key }}
          side={
            /* The column beside the map: a pick's panel, else the mission's summary (B19, B30, B31). */
            node ? (
              <MissionNodePanel mission={mission} node={node} events={log?.events ?? null} end={end} onSelect={select} onClose={() => select(null)} actions={actions} />
            ) : (
              <MissionSummary mission={mission} onSelect={select} />
            )
          }
        />
      )}
    </MapPage>
  );
}

/**
 * Whether the page has its two-column width (`mission-wide`, from
 * `--size-mission-wide`): the map's box then takes the height left beside
 * the panel instead of its map's own (B34).
 */
function useWide(root: HTMLElement | null): boolean {
  const [wide, setWide] = useState(false);
  useLayoutEffect(() => {
    if (!root) return undefined;
    const least = parseFloat(getComputedStyle(root).getPropertyValue("--size-mission-wide"));
    const measure = () => setWide(Number.isFinite(least) && root.clientWidth >= least);
    measure();
    if (typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(measure);
    observer.observe(root);
    return () => observer.disconnect();
  }, [root]);
  return wide;
}

/** Where the operator will look, over a plan that waits for approval (PRD mission-human-gate D-08, B1). */
function GatePlan({ mission, onSelect }: { mission: Mission; onSelect: (key: string) => void }) {
  return <GatePlanList nodes={mission.status === "planning" ? gatesAhead(mission) : []} onSelect={onSelect} className="px-md pb-sm" />;
}

/**
 * The head, one line (B24): the mission's state, its time, a cell per node,
 * `n / m` and its verification strength, which only the Conductor changes. A
 * device that does not answer stops the running time, so no number moves
 * without the record (B35).
 */
function Head({ mission, actions }: { mission: Mission; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const running = useElapsed(mission.status !== "done" && mission.status !== "cancelled" && !mission.stale ? mission.started_at_unix_ms : null);
  const { done, total } = missionCounts(mission);
  const held = useDelayPill(mission);
  const took = mission.started_at_unix_ms != null && mission.closed_at_unix_ms != null ? formatElapsed(language, mission.closed_at_unix_ms - mission.started_at_unix_ms) : null;
  const settings = useShellStore((s) => s.rest?.ui_state?.conductor_settings) ?? DEFAULT_CONDUCTOR;
  const events = useShellStore((s) => missionEventsOf(s.rest, mission.id)?.events ?? null);
  const options = useMemo(() => missionOptions(mission, settings, events, t), [mission, settings, events, t]);
  const time = took ? t("mission.elapsed.total", { elapsed: took }) : running ? t("mission.elapsed.running", { elapsed: running }) : null;
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-md gap-y-xs px-md py-sm text-body" data-mission-summary="true">
      <MissionNumber mission={mission} className="text-caption" />
      {held ? (
        <WorkPill tone={PILL_TONE.quiet} data-mission-pill="held">
          <HeldMark />
          {held}
        </WorkPill>
      ) : (
        <WorkPill tone={STATE_PILL_TONE[mission.status]} done={mission.status === "done"} data-mission-pill={mission.status}>
          {t(STATE_PILL[mission.status])}
        </WorkPill>
      )}
      {time ? <span className="shrink-0 text-caption text-muted-foreground" data-mission-stat="time">{time}</span> : null}
      {mission.nodes.length > 0 ? (
        <>
          <ProgressCells tones={missionHeadCells(mission)} fill={HEAD_FILL} size="head" className="max-w-(--size-pr-popover)" />
          <span className="shrink-0 text-caption text-muted-foreground tabular-nums" data-mission-stat="done">
            {t("mission.doneOf", { done: String(done), total: String(total) })}
          </span>
        </>
      ) : null}
      {/* Where the results go and what the mission applies (PRD mission-destination D-18, D-50): a tag only while a node lands in the destination, the options in grey. */}
      {mission.destination && landsInDestination(mission.nodes) ? <MissionDestinationTag branch={mission.destination} /> : null}
      <MissionOptionsLine options={options} className="shrink" />
      {/* An end the Conductor was asked for says so until it closes the mission. */}
      {mission.end_asked && mission.conductor.present && missionOpen(mission) ? <span className="shrink-0 text-caption text-mission-hold" data-mission-end-asked="true">{t("mission.end.askedNote")}</span> : null}
      <span className="ml-auto flex shrink-0 items-center gap-xs">
        {/* A closed mission opens again from its map (PRD mission-node-simple B18); a cancelled one never does. */}
        {mission.status === "done" || mission.status === "merge_wait" ? (
          <Button variant="text" size="inline" className="text-caption" disabled={mission.stale} onClick={() => actions.reopenMission(mission.id)} data-mission-reopen={mission.id}>
            {t("mission.reopen")}
          </Button>
        ) : null}
        <MissionEnd mission={mission} actions={actions} />
      </span>
    </div>
  );
}

/**
 * A finished or cancelled mission's report, one line under its results that
 * opens to the whole of it (B24; PRD mission-tool B20, B21). The report is
 * the Conductor's Markdown: the folded line is its first words without their
 * marks, past a first line that only repeats the title, and the opened
 * report is drawn as a Markdown document is.
 */
function ReportLine({ mission }: { mission: Mission }) {
  const { t } = useInterfaceTranslation();
  const [unfolded, setUnfolded] = useState(false);
  const report = mission.report ?? "";
  const plain = useMemo(() => markdownPlainText(report), [report]);
  // A first line that only repeats the mission's title gives way to the next (B21).
  const first = reportLine(plain, mission.title) ?? "";
  return (
    <div className="flex min-w-0 items-start gap-xs px-md pb-sm text-body" data-mission-report-text={unfolded ? "open" : "folded"}>
      <span aria-hidden="true" className={cn("mt-xxs h-(--size-icon-sm) w-px shrink-0", mission.status === "done" ? "bg-border" : "bg-mission-cancelled")} />
      <span className="shrink-0 font-semibold">{t("mission.report.label")}</span>
      {unfolded ? (
        <div className="min-w-0 flex-1 break-words" data-mission-report-body="true"><MarkdownText text={report} /></div>
      ) : (
        <p className="min-w-0 flex-1 truncate" title={plain}>{first}</p>
      )}
      <button type="button" aria-expanded={unfolded} onClick={() => setUnfolded(!unfolded)} className="shrink-0 rounded-xs text-caption text-primary outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring" data-mission-report-toggle="true">
        {t(unfolded ? "mission.report.collapse" : "mission.report.expand")}
      </button>
    </div>
  );
}

type GraphCard = { key: string; node: MissionNode | null };

function Graph({ mission, flow, events, end, selected, onSelect, actions }: { mission: Mission; flow: MissionMapFlow; events: MissionEvent[] | null; end: number; selected: string | null; onSelect: (key: string) => void; actions: Actions }) {
  const nodes = mission.nodes;
  const graph = useMemo(() => {
    const shown = new Set(nodes.map((node) => node.key));
    const cards: GraphCard[] = [...nodes.map((node) => ({ key: node.key, node })), { key: REPORT, node: null }];
    const byKey = new Map(cards.map((card) => [card.key, card]));
    const depended = new Set(nodes.flatMap((node) => node.depends_on));
    const blockers = (card: GraphCard): GraphCard[] => {
      // Every node no other node waits on ends in the report.
      if (!card.node) return nodes.filter((node) => !depended.has(node.key)).map((node) => byKey.get(node.key)!);
      return card.node.depends_on.filter((key) => shown.has(key)).map((key) => byKey.get(key)!);
    };
    return layerDependencies(cards, (card) => card.key, blockers);
  }, [nodes]);
  const defects = useMemo(() => defectEdges(mission), [mission]);
  const byKey = useMemo(() => new Map(nodes.map((node) => [node.key, node])), [nodes]);
  // A pick keeps itself and every card one edge away lit, the defect edges included (B30).
  const lit = selected ? neighbours([...graph.edges, ...defects], selected) : null;
  const away = (edge: GraphEdge) => selected !== null && edge.from !== selected && edge.to !== selected;
  const edgeLook = (edge: GraphEdge): EdgeLook => {
    const from = byKey.get(edge.from);
    const to = byKey.get(edge.to);
    const dimmed = away(edge);
    // A stack is the one purple dashed edge: the pull request below is the one above's base (B27).
    if (from && to && to.base_pr != null && from.prs.some((pr) => pr.number === to.base_pr)) return { tone: "text-mission-review", dashed: true, dimmed };
    // A line from a node at work is lit only while the mission is open; a cancelled mission's work stopped.
    if (from && from.status === "working" && missionOpen(mission)) return { tone: "text-mission-working", dimmed };
    return { tone: "text-border", dimmed };
  };
  // A defect a later node found is a red dashed edge back to the node it opened, until its line passes (B21).
  const overlay: OverlayEdge[] = defects.map((edge) => ({ ...edge, look: { tone: "text-mission-failed", dashed: true, dimmed: away(edge) } }));
  return (
    <DependencyGraphView
      graph={graph}
      idOf={(card) => card.key}
      flow={flow}
      gaps={{ layers: "--size-mission-layer-gap", nodes: "--size-mission-node-gap" }}
      edgeLook={edgeLook}
      overlay={overlay}
      draw={(card) =>
        card.node ? (
          <NodeCard key={card.key} mission={mission} node={card.node} events={events} end={end} actions={actions} selected={selected === card.key} dimmed={lit !== null && !lit.has(card.key)} onSelect={() => onSelect(card.key)} />
        ) : (
          <ReportCard key={card.key} mission={mission} dimmed={lit !== null && !lit.has(REPORT)} />
        )
      }
    />
  );
}

/** A card's border and surface by its look (B26): finished work a faint green surface inside its plain border, never a green border, in main or not. */
const LOOK_CARD: Record<NodeLook, string> = {
  waiting: "border-dashed border-mission-waiting bg-transparent",
  working: "border-mission-working ring-2 ring-mission-working/20",
  done: "border-border bg-mission-done/10",
  external: "border-dotted border-mission-waiting bg-transparent",
  cancelled: "border-dashed border-mission-cancelled bg-transparent",
  dropped: "border-dashed border-border bg-transparent",
};

/** A card waiting on the operator wears 내 차례's card colour as a thick border and ring over its look, readable at the map's smallest zoom (PRD mission-node-simple B2). */
const ATTENTION_CARD = "border-2 border-mission-hold ring-4 ring-mission-hold/20";

/**
 * A node on the map (PRD mission-node-simple D-07, B1-B3; PRD
 * mission-destination D-13-D-25, D-45, D-46): its state's mark and border,
 * its title on one line that ends in an ellipsis before anything else does,
 * with Hide's mark when Hide changed it last and, at the row's right, its
 * worker's agent mark, model and how long it has taken; a bar of one cell
 * per step of how its work goes (done, being worked, or only planned) with
 * the step being worked under it, or `계획 n단계 · 시작 전` before a worker
 * starts; the strip of the human steps its plan has, the gate's and a merge
 * its plan gives the operator; and its last line: for a node that changes
 * code the stations its result passes to the branch it lands in, for one
 * that does not, once done, `결론 ›` or the verdict; after them `! 나` or `!
 * Conductor` when it waits on someone to look, its reason on hover. A line
 * with nothing to hold is not drawn. Hovering or focusing it shows the rest
 * (`MissionNodeHover`); the other agents checking it stand beside it as
 * satellites tethered to its right side. Pressing it picks it (B30).
 */
function NodeCard({ mission, node, events, end, selected, dimmed, onSelect, actions }: { mission: Mission; node: MissionNode; events: MissionEvent[] | null; end: number; selected: boolean; dimmed: boolean; onSelect: () => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const look = nodeLook(mission, node);
  const attention = openAttention(mission, node) ? attentionWords(node, t) : null;
  // A place the plan has the operator look (PRD mission-human-gate D-06): ahead it is a tinted strip, reached it is the operator's turn, looked at it is grey with a check.
  const gate = node.gate ?? null;
  // A merge the node's own plan gives the operator wears the same strip (PRD mission-destination D-45); a scope `pr` mission's merges wear none (D-35).
  const merging = mergeOverridden(mission.scope, node);
  const minute = useMinuteClock();
  const long = gateWaitedLong(gate, minute);
  // Only a card waiting on the operator is the operator's turn; one waiting on the Conductor, or a gate still ahead, reads as work in progress (B9).
  const yours = (attention !== null && node.attention!.to === "operator") || gate?.state === "open";
  const warning = warningWords(mission, node, t);
  const hide = hideWords(mission, node, t);
  const satellites = satelliteLines(node.criteria);
  const word = markWord(node, look, t);
  // What its worker does now (hide-private issue 680): a resting or absent worker stops the ring and says so under the title; a working one needs no words.
  const doing = nodeWork(mission, node);
  const doingLine = doing && doing.state !== "working" && doing.state !== "stopped" ? nodeWorkWords(doing, t) : null;
  const stopLine = useStopWords(doing);
  const hold = nodeHold(mission, node);
  const holdLine = useHoldWords(hold);
  // A worker's person's turn is the step's waiting reason, a key and what and where (PRD ask-human D-18, B25); the Conductor's row carries Done.
  const escalationOf = useWorkerEscalation();
  const human = node.status === "working" ? escalationOf(node)?.human ?? null : null;
  const humanLine = human ? humanTurnText(t, human) : null;
  const worker = useNodeRun(mission, node);
  const device = useMissionDeviceName(node.machine);
  const run = worker?.run ?? null;
  const took = useNodeTimes(mission, node, events, end)?.took ?? null;
  // How the node's work goes is one bar of cells under its title, from the worker's plan once it exists, else the Conductor's steps; its count is in the card's name, never a second number on the card.
  const work = nodeSteps(node);
  // Where its result goes (D-13): stations for a node that changes code, else once done its conclusion or verdict (D-17).
  const stations = useMemo(() => nodeStations(stationNode(node), mission.destination ?? null, missionOpen(mission)), [node, mission]);
  const finished = stations ? null : nodeResult(node);
  const result = finished && (finished.kind === "conclusion" || finished.kind === "verdict" || finished.kind === "verify") ? finished : null;
  const titleText = (
    <span className={cn("min-w-0 flex-1 truncate text-body font-semibold", (look === "external" || look === "cancelled" || look === "waiting") && "text-muted-foreground", look === "dropped" && "text-muted-foreground line-through")}>
      {node.title}
    </span>
  );
  const card = (
    <button
      type="button"
      onClick={onSelect}
      aria-label={[
        node.title,
        word,
        ...(node.kind === "external" && look !== "external" ? [t("mission.look.external")] : []),
        ...(run ? [run.text] : []),
        ...(device ? [device] : []),
        ...(took ? [took] : []),
        ...(work ? [node.plan ? t("plan.chip", { progress: planCount(node.plan) }) : t("mission.steps.planned", { count: work.steps.length })] : []),
        ...node.prs.map((pr) => t("mission.attention.pr", { number: String(pr.number) })),
        ...(stations ? [`${t("mission.station.line")}: ${stationWords(stations, t)}`] : []),
        ...(result ? [resultWords(result, t)] : []),
        ...(merging ? [mergeWords(node, t)] : []),
        ...(attention ? [`${t("mission.attention")} · ${attention.who} · ${attention.why}`] : []),
        ...(gate ? [`${t("mission.gate.title")} · ${gate.look}${gate.state === "passed" ? ` · ${t("mission.gate.passed")}` : gate.state === "open" ? ` · ${t("agents.mark.turn")}` : ""}${long ? ` · ${t("mission.gate.long")}` : ""}`] : []),
        ...(humanLine ? [`${t("agents.human.mark")} · ${humanLine}`] : []),
        ...(stopLine ? [stopLine.full] : []),
        ...(holdLine ? [holdLine.full] : []),
        ...(warning ? warning.full : []),
        ...(hide ? [t("mission.hide.mark", { what: hide })] : []),
      ].join(" · ")}
      aria-pressed={selected}
      data-mission-node={node.key}
      data-node-status={node.status}
      data-node-look={look}
      data-node-attention={attention ? node.attention!.to : undefined}
      data-node-blocked={node.blocked ?? undefined}
      data-node-dimmed={dimmed || undefined}
      className={cn(
        "relative flex w-(--size-mission-node) min-w-0 flex-col gap-xxs rounded-md border bg-card px-sm py-xs text-left outline-none transition-opacity hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring",
        yours ? ATTENTION_CARD : LOOK_CARD[look],
        // Outside is the node's kind: the dotted border stays in whatever state it ends (B19).
        node.kind === "external" && "border-dotted",
        selected && "outline-2 outline-offset-2 outline-primary",
        dimmed && "opacity-(--opacity-dimmed)",
      )}
    >
      {/* A node that runs on another device wears that device's colour on its edge, the one the workspace's header band wears; a node here stays quiet. */}
      {device ? <span aria-hidden="true" className="absolute inset-y-xs left-0 w-xs rounded-r-xs bg-device-remote" data-node-remote="true" /> : null}
      {/* The operator's turn wears a dot on its corner in the colour of 내 차례's card. */}
      {yours ? <span aria-hidden="true" className="absolute -top-xs -right-xs size-(--size-agent-mark) rounded-full border-2 border-card bg-mission-hold" data-node-turn="true" /> : null}
      <span className="flex min-w-0 items-center gap-xs">
        <MissionNodeMark look={look} label={word} still={doing != null && doing.state !== "working"} />
        {titleText}
        {hide ? <HideMark words={hide} /> : null}
        {/* The worker on the title row's right (D-46): its agent mark, its model and how long the node has taken, kept whole while the title truncates first; its effort, profile and device are the hover's. */}
        {worker?.agent || worker?.model || took ? (
          <span className="flex max-w-[calc(var(--size-mission-node)/2)] shrink-0 items-center gap-xxs text-caption text-muted-foreground" data-node-run={run?.from ?? "none"}>
            {worker?.agent ? <span className="flex shrink-0" aria-hidden="true" data-node-worker-mark="true"><AgentMark kind={worker.agent} profile={run?.profile} /></span> : null}
            {worker?.model ? <span className="min-w-0 truncate" data-node-model={run?.from}>{worker.model}</span> : null}
            {run?.mark ? <span className="shrink-0" data-run-planned="true">{run.mark}</span> : null}
            {took ? <span className="shrink-0 whitespace-nowrap" data-node-took="true">{took}</span> : null}
          </span>
        ) : null}
      </span>
      {doingLine ? (
        <span className={cn("min-w-0 truncate text-caption", doing?.state === "no_worker" ? "text-warning" : "text-muted-foreground")} data-node-work={doing?.state}>
          {doingLine}
        </span>
      ) : null}
      {/* A worker stopped with nothing to wait on says why under the title as a held node does, the cause cut when it runs long and how long ago whole at the line's end; its Conductor, not the operator, is told (operator decision 2026-11-11). */}
      {stopLine ? (
        <span className="flex min-w-0 items-center gap-xxs text-caption text-muted-foreground" title={stopLine.full} data-node-work="stopped">
          <span className="min-w-0 truncate">{stopLine.why}</span>
          {stopLine.age ? <span className="shrink-0 whitespace-nowrap" data-node-stopped-age="true">· {stopLine.age}</span> : null}
        </span>
      ) : null}
      {/* A node its Conductor holds back says why in grey under the title, cut when it runs long, and keeps how long it still holds whole at the line's end (operator decision 2026-11-11). */}
      {holdLine ? (
        <span className="flex min-w-0 items-center gap-xxs text-caption text-muted-foreground" title={holdLine.full} data-node-hold="true">
          <PauseIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
          <span className="min-w-0 truncate">{holdLine.why}</span>
          {holdLine.left ? <span className="shrink-0 whitespace-nowrap" data-node-hold-left="true">· {holdLine.left}</span> : null}
        </span>
      ) : null}
      {humanLine ? (
        <span className="flex min-w-0 items-center gap-xxs text-caption text-warning" title={humanLine} data-node-human-turn="true">
          <KeyRoundIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
          <span className="min-w-0 truncate">{humanLine}</span>
        </span>
      ) : null}
      {work ? (
        <span className="flex min-w-0 flex-col gap-xxs" data-node-steps={work.source}>
          <ProgressCells tones={work.steps.map((step) => step.tone)} fill={STEP_FILL} size="head" className="h-(--size-mission-step-h) flex-none" />
          {work.now && node.status === "working" ? (
            <span className="min-w-0 truncate text-caption text-subtle-foreground" data-node-steps-now="true">{t("mission.steps.now")} <span className="font-medium text-foreground">{work.now}</span></span>
          ) : null}
          {work.source === "plan" && node.status === "planned" ? (
            <span className="min-w-0 truncate text-caption text-muted-foreground" data-node-steps-waiting="true">{t("mission.steps.planned", { count: work.steps.length })} · {t("mission.steps.notStarted")}</span>
          ) : null}
        </span>
      ) : null}
      {/* What the operator will look at, and a merge its plan gives them, are a strip of the card itself, read without a hover (PRD mission-human-gate D-06; PRD mission-destination D-45). */}
      <HumanStrip gate={gate} merge={merging} node={node} />
      {stations || result || attention || warning || gate?.state === "open" ? (
        <span className="flex min-w-0 items-center gap-x-xs text-caption text-muted-foreground" data-node-row="true">
          {/* The stations take the row and their last name truncates; `⚠` and `!` keep their place at the right end (B30). */}
          {stations ? <StationLine stations={stations} className="flex-1" /> : <span className="flex min-w-0 flex-1 items-center">{result ? <ResultTail result={result} stale={mission.github_stale} /> : null}</span>}
          {warning ? <NodeWarning words={warning} kinds={(node.warnings ?? []).map((item) => item.kind).join(" ")} /> : null}
          {gate?.state === "open" && !attention ? (
            <span className="shrink-0 font-semibold text-mission-hold" data-node-gate-tag="open">
              {t("agents.mark.turn")}
              {long ? <span className="font-normal text-muted-foreground" data-node-gate-long="true"> · {t("mission.gate.long")}</span> : null}
            </span>
          ) : null}
          {attention ? (
            <Hint label={attention.why}>
              <span className={cn("shrink-0", yours ? "font-semibold text-mission-hold" : "text-muted-foreground")} data-node-attention-tag={node.attention!.reason}>
                {yours ? t("agents.mark.turn") : attention.who}
                {long ? <span className="font-normal text-muted-foreground" data-node-gate-long="true"> · {t("mission.gate.long")}</span> : null}
              </span>
            </Hint>
          ) : null}
        </span>
      ) : null}
    </button>
  );
  const hovered = <MissionNodeHover mission={mission} node={node} facts={{ worker: run ? { agent: worker?.agent ?? null, words: run.text, device } : null, now: work?.now && node.status === "working" ? work.now : null, stations }}>{card}</MissionNodeHover>;
  // The card and its satellites are one box to the layout, the card centred in it, so edges meet the card's middle.
  // An empty column as wide as the satellites' keeps the card in the middle of the box.
  if (satellites.length === 0) return <div data-dependency-node={node.key} className="flex">{hovered}</div>;
  return (
    <div data-dependency-node={node.key} className="grid grid-cols-[var(--spacing-xl)_auto_var(--spacing-xl)] items-center">
      <span aria-hidden="true" />
      {hovered}
      {/* 노드 상세 opens the node's panel; it never shuts the panel already open, as a second press on the card does. */}
      <Satellites mission={mission} lines={satellites} dimmed={dimmed} onSelect={() => !selected && onSelect()} actions={actions} />
    </div>
  );
}

/**
 * A node's warning (hide-private issue 552): its worker looks stuck, and
 * Hide told the Conductor, never the operator. The warning colour and its
 * triangle beside the first warning's short word; each warning in full on
 * hover, which the card's accessible name says too.
 */
function NodeWarning({ words, kinds }: { words: { short: string; full: string[] }; kinds: string }) {
  return (
    <Hint label={words.full.join("\n")}>
      <span aria-hidden="true" className="flex shrink-0 items-center gap-xxs font-semibold text-warning" data-node-warning={kinds}>
        <TriangleAlertIcon className="size-(--size-icon-sm)" />
        {words.short}
      </span>
    </Hint>
  );
}

/** Hide's own mark on a card it changed last (B3): what it did and why on hover; the card's accessible name says the same. */
function HideMark({ words }: { words: string }) {
  const { t } = useInterfaceTranslation();
  return (
    <Hint label={t("mission.hide.mark", { what: words })}>
      <span aria-hidden="true" className="flex shrink-0 text-muted-foreground" data-node-hide="true">
        <CogIcon className="size-(--size-icon-sm)" />
      </span>
    </Hint>
  );
}

/** A finished node's conclusion or verdict as the card's name says it. */
function resultWords(result: NodeResult, t: TFunction<"translation">): string {
  switch (result.kind) {
    case "verdict":
      return `${t(result.verdict === "pass" ? "mission.result.pass" : "mission.result.fail")}${result.checks ? ` ${result.checks.passed}/${result.checks.total}` : ""}`;
    case "conclusion":
      return t("mission.result.conclusion");
    case "verify":
      return result.line;
    default:
      return "";
  }
}

function ReportCard({ mission, dimmed }: { mission: Mission; dimmed: boolean }) {
  const { t } = useInterfaceTranslation();
  const stack = stackWaiting(mission);
  const note = mission.status === "done" ? t("mission.report.done") : mission.status === "merge_wait" && stack > 0 ? t("mission.report.stack") : mission.status === "cancelled" ? t("mission.cancelled") : t("mission.report.waiting");
  const look: NodeLook = mission.status === "done" ? "done" : mission.status === "cancelled" ? "cancelled" : "waiting";
  return (
    <div data-dependency-node={REPORT} data-mission-report={mission.status} className={cn("flex w-(--size-mission-node) items-center justify-center gap-xs rounded-xl border px-md py-sm transition-opacity", LOOK_CARD[look], dimmed && "opacity-(--opacity-dimmed)")}>
      <MissionNodeMark look={look} label={note} />
      <span className="text-body font-semibold">{t("mission.report")}</span>
      <span className="text-caption text-muted-foreground">{note}</span>
    </div>
  );
}

/** A stopped worker's line: why first, then how long ago it stopped; `full` is the whole sentence, the agent's own line included, for the hover and the card's name. */
function useStopWords(work: MissionNodeWork | null): { why: string; age: string | null; full: string } | null {
  const { t } = useInterfaceTranslation();
  const stopped = work?.state === "stopped" ? work : null;
  const age = useElapsed(stopped?.since_unix_ms);
  if (!stopped) return null;
  const why = stopCauseWords(stopped.cause, t, true);
  return {
    why: nodeWorkWords(stopped, t),
    age,
    full: age ? t("mission.work.stoppedFull", { age, why }) : t("mission.work.stopped", { why }),
  };
}

/** A held node's line: why first, then how long it still holds while the core has not lifted it yet; `full` is the whole sentence for the hover and the card's name. */
function useHoldWords(hold: MissionNodeHold | null): { why: string; left: string | null; full: string } | null {
  const { t } = useInterfaceTranslation();
  const left = useRemaining(hold?.until_unix_ms);
  if (!hold) return null;
  const still = left && left !== "past" ? left.text : null;
  return {
    why: t("mission.hold", { why: hold.why }),
    left: still,
    full: still ? t("mission.holdLeft", { why: hold.why, left: still }) : t("mission.hold", { why: hold.why }),
  };
}

/** A mission's delay and how long it still holds; every delay ends by itself (`hide mission delay --for`). Shaped as a held node's line: the why first and cut when it runs long, the remaining time whole at the end. */
function DelayLine({ line, until }: { line: string; until: number | null | undefined }) {
  const { t } = useInterfaceTranslation();
  const left = useRemaining(until);
  const still = left && left !== "past" ? left.text : null;
  return (
    <p className="flex min-w-0 gap-xxs px-md pb-xs text-caption text-muted-foreground" title={still ? t("mission.delayLeft", { line, left: still }) : t("mission.delay", { line })} data-mission-delay="true">
      <span className="min-w-0 truncate">{t("mission.delay", { line })}</span>
      {still ? <span className="shrink-0 whitespace-nowrap">· {still}</span> : null}
    </p>
  );
}
