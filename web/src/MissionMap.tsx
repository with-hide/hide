import { CheckIcon, ChevronRightIcon, GitBranchIcon, PencilIcon } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { PlanChecklist } from "./PlanChecklist";
import { planNowLine } from "./plans";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { DependencyGraphView, type EdgeLook } from "./DependencyGraph";
import type { GraphEdge } from "./dependencyLayout";
import { formatElapsed, useElapsed, useMinuteClock } from "./components/elapsed";
import { ProgressCells, TONE_BORDER, TONE_FILL, ToneSwatch } from "./components/mission-tone";
import { PrMark } from "./components/pr-mark";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { ToggleGroup, ToggleGroupItem } from "./components/ui/toggle-group";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { formatDateTime } from "./i18n/format";
import { requireInterfaceLanguage, type InterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { DecisionBanner } from "./MissionDecision";
import {
  axisMarks,
  eventWords,
  flowShows,
  flowSpan,
  kindWord,
  launchModelOf,
  heldQuestion,
  missionCells,
  missionCounts,
  missionEventsOf,
  missionLegend,
  missionOf,
  missionOpen,
  missionResults,
  nodeBar,
  nodeLine,
  nodeMark,
  nodeMarks,
  nodeResult,
  nodeSegments,
  nodeTone,
  orderedNodes,
  shortCommit,
  stackWaiting,
  stepsOf,
  workerWords,
  type FlowFilter,
  type FlowMarkKind,
  type LegendTone,
  type NodeResult,
  type SegmentTone,
} from "./missions";
import { allAgents, agentsByPane } from "./paneAgents";
import { staleLabel } from "./prMark";
import { layerDependencies } from "./projectBoard";
import { machineName } from "./screenMachine";
import { sessionOfAgent } from "./sessionWork";
import type { Mission, MissionEvent, MissionEventsSnapshot, MissionNode, MissionStatus, PlanView, RowMark } from "./snapshot";
import { useShellStore } from "./store";
import type { MessageKey } from "./i18n/catalogs";
import type { TFunction } from "i18next";

// A mission's detail in the lane's 보기 slot (PRD software-factory-conductor
// D-06, B14-B21, B43; PRD mission-plan-visibility D-10-D-14, D-19, D-25,
// B12-B17, B19, B20, B22-B24): one page that scrolls down, its head with the
// state, the time and a cell per node, the decision banner when the operator
// has something to decide (B25-B30) or a line when no Conductor holds it,
// the map at the slot's full width and one 흐름 table, a row per node on the
// mission's time axis drawn from its events (B18-B21). A finished or
// cancelled mission puts its report and results first. Every value is the
// core's (`MissionView`, `MissionEventsSnapshot`); the page words and draws it.

/** The card that stands for the report every node ends in; a node key is letters, digits, `-`, `_` and `.`, so none is this. */
const REPORT = "#report";

/** A report longer than this many lines shows its first ones and `펼치기` (B23). */
const REPORT_FOLD_LINES = 3;

/** How many marks the time axis has, its two ends included (B17). */
const AXIS_TICKS = 4;

export function MissionMap({ missionId, actions }: { missionId: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const mission = useShellStore((s) => missionOf(s.rest, missionId));
  const failed = useShellStore((s) => s.rest?.missions?.failed ?? null);
  if (failed) return <MapState text={t("mission.unreadable")} tone="text-destructive" state="unreadable" />;
  if (!mission) return <MapState text={t("mission.gone")} state="gone" />;
  return <MissionBody mission={mission} actions={actions} />;
}

function MapState({ text, tone, state }: { text: string; tone?: string; state: string }) {
  return <p className={cn("px-md py-md text-body text-muted-foreground", tone)} data-mission-state={state}>{text}</p>;
}

function MissionBody({ mission, actions }: { mission: Mission; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [expanded, setExpanded] = useState<string | null>(null);
  const [filter, setFilter] = useState<FlowFilter>("all");
  const [zoom, setZoom] = useState<Zoom>("fit");
  const device = useShellStore((s) => (mission.stale ? machineName(s.rest?.navigator?.devices, mission.device_id, t) : null));
  const log = useShellStore((s) => missionEventsOf(s.rest, mission.id));
  const open = missionOpen(mission);
  const ended = mission.status === "done" || mission.status === "cancelled";
  const takeover = !mission.conductor.present && open;
  // A card pressed on the map opens its row, shown whatever the filter hid, and pressed again closes it (B20).
  const openRow = (key: string) => {
    if (expanded === key) return setExpanded(null);
    const node = mission.nodes.find((candidate) => candidate.key === key);
    if (node && !flowShows(node, filter)) setFilter("all");
    setExpanded(key);
  };
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-auto" data-mission={mission.id} data-mission-status={mission.status} data-mission-stale={mission.stale || undefined} data-mission-scroll="true">
      <Head mission={mission} zoom={mission.nodes.length > 0 ? zoom : null} onZoom={setZoom} />
      {mission.decision && open ? (
        <DecisionBanner mission={mission} device={device} actions={actions} />
      ) : takeover ? (
        <Banner
          mark={<span aria-hidden="true" className="size-(--size-agent-mark) shrink-0 rounded-full border border-dashed border-muted-foreground" />}
          text={t("mission.noConductor")}
          action={<Button size="sm" onClick={() => actions.takeOverMission(mission.id)} data-mission-takeover="true">{t("mission.takeOver")}</Button>}
          data-mission-banner="no-conductor"
        />
      ) : null}
      {mission.delay && open ? <p className="px-md pb-xs text-caption text-muted-foreground" data-mission-delay="true">{t("mission.delay", { line: mission.delay })}</p> : null}
      {device ? <p className="px-md pb-xs text-caption text-muted-foreground" data-mission-stale-note="true">{t("mission.stale", { device })}</p> : null}
      <div className={cn("flex min-w-0 flex-col", mission.stale && "opacity-(--opacity-dimmed)")} data-mission-values={mission.stale ? "stale" : "live"}>
        {ended ? <Ending mission={mission} log={log} /> : null}
        {mission.nodes.length === 0 ? (
          <p className="px-md py-sm text-body text-muted-foreground" data-mission-planning="true">{t("mission.planning")}</p>
        ) : (
          <>
            <div className={cn("min-w-0 px-md pb-md pt-sm", zoom === "actual" && "overflow-x-auto")}>
              <FitWidth fit={zoom === "fit"}>
                <Graph mission={mission} selected={expanded} onSelect={openRow} />
              </FitWidth>
            </div>
            <FlowTable mission={mission} log={log} filter={filter} onFilter={setFilter} expanded={expanded} onExpand={setExpanded} actions={actions} />
          </>
        )}
      </div>
    </div>
  );
}

/** How big the map is drawn: shrunk to the slot's width so every edge shows at once, or at its own size with scrolling. */
type Zoom = "fit" | "actual";

/**
 * The map shrunk to the width it has, never grown: a wide row of nodes stays
 * one row, so what waits on what is read at a glance. The box holds the
 * shrunk size, so the page scrolls only down, or the map sideways at 100%.
 */
function FitWidth({ fit, children }: { fit: boolean; children: ReactNode }) {
  const room = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const [drawn, setDrawn] = useState<{ scale: number; width: number; height: number } | null>(null);
  useLayoutEffect(() => {
    const outer = room.current;
    const inner = content.current;
    if (!outer || !inner) return;
    const measure = () => {
      // Layout sizes, which a transform does not change.
      const width = inner.offsetWidth;
      const height = inner.offsetHeight;
      const scale = fit && width > outer.clientWidth ? outer.clientWidth / width : 1;
      setDrawn((current) => (current?.scale === scale && current.width === width && current.height === height ? current : { scale, width, height }));
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(outer);
    observer.observe(inner);
    return () => observer.disconnect();
  }, [fit]);
  const scale = drawn?.scale ?? 1;
  return (
    <div ref={room} className="min-w-0" data-mission-fit={fit ? "fit" : "actual"} data-mission-scale={scale.toFixed(2)}>
      <div className="relative" style={drawn ? { width: drawn.width * scale, height: drawn.height * scale } : undefined}>
        <div ref={content} className={cn("w-max origin-top-left", drawn && "absolute left-0 top-0")} style={scale < 1 ? { transform: `scale(${scale})` } : undefined}>
          {children}
        </div>
      </div>
    </div>
  );
}

function Banner({ mark, text, tone, action, ...data }: { mark: ReactNode; text: string; tone?: string; action: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <div className="mx-md mb-xs flex min-w-0 items-center gap-sm rounded-md bg-muted px-sm py-xs" {...data}>
      {mark}
      <span className={cn("min-w-0 flex-1 truncate text-body", tone)} title={text}>{text}</span>
      {action}
    </div>
  );
}

const PILL: Record<MissionStatus, { key: MessageKey; tone: string }> = {
  planning: { key: "mission.pill.planning", tone: "bg-muted text-muted-foreground" },
  running: { key: "mission.pill.running", tone: "bg-mission-working/15 text-mission-working" },
  merge_wait: { key: "mission.pill.mergeWait", tone: "bg-mission-review/15 text-mission-review" },
  done: { key: "mission.pill.done", tone: "bg-mission-done/15 text-mission-done" },
  cancelled: { key: "mission.pill.cancelled", tone: "bg-muted text-mission-cancelled" },
};

/**
 * The head (B13): the mission's state, its time, a cell per node and
 * `n / m 끝`, then one legend line counting each state with nodes in it. No
 * PR count, no main count and no cap: the results and the map carry those.
 */
function Head({ mission, zoom, onZoom }: { mission: Mission; zoom: Zoom | null; onZoom: (zoom: Zoom) => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const running = useElapsed(mission.status !== "done" && mission.status !== "cancelled" ? mission.started_at_unix_ms : null);
  const { done, total } = missionCounts(mission);
  const took = mission.started_at_unix_ms != null && mission.closed_at_unix_ms != null ? formatElapsed(language, mission.closed_at_unix_ms - mission.started_at_unix_ms) : null;
  const time = took ? t("mission.elapsed.total", { elapsed: took }) : running ? t("mission.elapsed.running", { elapsed: running }) : null;
  const pill = PILL[mission.status];
  const legend = missionLegend(mission);
  return (
    <div className="flex min-w-0 flex-col gap-xs px-md pb-sm pt-sm" data-mission-summary="true">
      <div className="flex min-w-0 flex-wrap items-center gap-x-md gap-y-xs text-body">
        <span className={cn("flex shrink-0 items-center gap-xxs rounded-full px-sm text-caption font-semibold", pill.tone)} data-mission-pill={mission.status}>
          {mission.status === "done" ? <CheckIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : null}
          {t(pill.key)}
        </span>
        {time ? <span className="shrink-0 text-caption text-muted-foreground" data-mission-stat="time">{time}</span> : null}
        {mission.nodes.length > 0 ? (
          <>
            <ProgressCells tones={missionCells(mission)} size="head" className="max-w-(--size-pr-popover)" />
            <span className="shrink-0 text-caption text-muted-foreground" data-mission-stat="done">
              {t("mission.doneOf", { done: String(done), total: String(total) })}
            </span>
          </>
        ) : null}
      </div>
      {legend.length > 0 || zoom ? (
        <div className="flex min-w-0 items-center gap-md">
          <ul aria-label={t("mission.legend")} className="flex min-w-0 flex-1 flex-wrap gap-x-md gap-y-xxs text-caption text-muted-foreground" data-mission-legend={legend.map((entry) => `${entry.tone}:${entry.count}`).join(" ")}>
            {legend.map((entry) => (
              <li key={entry.tone} className="flex items-center gap-xs">
                <ToneSwatch tone={entry.tone} />
                {t(`mission.legend.${entry.tone}`)}
                <span className="font-semibold tabular-nums text-foreground">{entry.count}</span>
              </li>
            ))}
          </ul>
          {/* How big the map below is drawn (맞춤 / 100%). */}
          {zoom ? (
            <ToggleGroup type="single" value={zoom} onValueChange={(value) => value && onZoom(value as Zoom)} aria-label={t("mission.zoom")} className="shrink-0" data-mission-zoom={zoom}>
              <ToggleGroupItem size="sm" value="fit" data-mission-zoom-choice="fit">{t("mission.zoom.fit")}</ToggleGroupItem>
              <ToggleGroupItem size="sm" value="actual" data-mission-zoom-choice="actual">{t("mission.zoom.actual")}</ToggleGroupItem>
            </ToggleGroup>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/** A finished or cancelled mission's first part (B23, B24): when it was cancelled, the Conductor's report and the results table. */
function Ending({ mission, log }: { mission: Mission; log: MissionEventsSnapshot | null }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const cancelledAt = mission.status === "cancelled" ? (log?.events.find((event) => event.kind === "cancelled")?.at_unix_ms ?? mission.closed_at_unix_ms) : null;
  const [unfolded, setUnfolded] = useState(false);
  const lines = mission.report?.split("\n") ?? [];
  const long = lines.length > REPORT_FOLD_LINES;
  const shown = long && !unfolded ? lines.slice(0, REPORT_FOLD_LINES) : lines;
  const results = missionResults(mission);
  return (
    <div className="flex min-w-0 flex-col gap-sm px-md pb-sm">
      {cancelledAt != null ? (
        <p className="text-caption text-mission-cancelled tabular-nums" data-mission-cancelled-at="true">
          {t("mission.cancelledAt", { time: formatDateTime(language, cancelledAt, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" }) })}
        </p>
      ) : null}
      {mission.report ? (
        <div className={cn("flex min-w-0 flex-col gap-xxs rounded-md border-l-2 bg-muted px-sm py-xs text-body", mission.status === "done" ? "border-mission-done" : "border-mission-cancelled")} data-mission-report-text={unfolded ? "open" : "folded"}>
          <p className="min-w-0 whitespace-pre-wrap break-words">{shown.join("\n")}</p>
          {long ? (
            <button type="button" aria-expanded={unfolded} onClick={() => setUnfolded(!unfolded)} className="w-fit rounded-xs text-caption text-primary outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring" data-mission-report-toggle="true">
              {t(unfolded ? "mission.report.collapse" : "mission.report.expand")}
            </button>
          ) : null}
        </div>
      ) : null}
      {results.length > 0 ? (
        <table className="w-full table-fixed border-collapse text-body" data-mission-results={results.length}>
          <thead>
            <tr className="border-b border-border text-left text-caption text-muted-foreground">
              <th className="py-xxs pr-sm font-normal">{t("mission.flow.task")}</th>
              <th className="w-(--size-flow-result) py-xxs pr-sm font-normal">{t("mission.results.kind")}</th>
              <th className="py-xxs font-normal">{t("mission.flow.result")}</th>
            </tr>
          </thead>
          <tbody>
            {results.map(({ node, result }) => (
              <tr key={node.key} className="border-b border-border/50" data-mission-result-row={node.key}>
                <td className="truncate py-xxs pr-sm" title={node.title}>{node.title}</td>
                <td className="py-xxs pr-sm text-muted-foreground">{t(shortKind(node))}</td>
                <td className="min-w-0 py-xxs"><ResultTail result={result} stale={mission.github_stale} full /></td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
    </div>
  );
}

/** A node's kind in one word (B17, B23): `조사`, `직접`, `검증`, or `PR` for a change through a pull request. */
function shortKind(node: MissionNode): MessageKey {
  if (node.kind === "research") return "mission.kind.short.research";
  if (node.kind === "verify") return "mission.kind.short.verify";
  return node.delivery === "direct" ? "mission.kind.short.direct" : "mission.kind.short.pr";
}

/**
 * What a node left, by its kind (B22): `⑂ #n` in its pull request's state
 * colour, a direct change's branch, `결론 ▸` for research or a verify node's
 * line; `full` writes the conclusion itself, as the results table does.
 */
export function ResultTail({ result, stale, full = false }: { result: NodeResult; stale: boolean; full?: boolean }) {
  const { t } = useInterfaceTranslation();
  switch (result.kind) {
    case "pr":
      return result.state ? <PrMark state={result.state} number={result.number} stale={stale} data-node-pr={result.number} /> : <span className="shrink-0 font-mono text-caption text-subtle-foreground" data-node-pr={result.number}>#{result.number}</span>;
    case "commit":
      return <span className="shrink-0 font-mono text-caption text-mission-done" title={result.commit} data-node-commit={result.commit}>{t("mission.result.main", { commit: shortCommit(result.commit) })}</span>;
    case "verdict":
      return (
        <span className={cn("shrink-0 font-mono text-caption font-semibold", result.verdict === "pass" ? "text-mission-done" : "text-mission-failed")} data-node-verdict={result.verdict}>
          {t(result.verdict === "pass" ? "mission.result.pass" : "mission.result.fail")}
          {result.checks ? ` ${result.checks.passed}/${result.checks.total}` : ""}
        </span>
      );
    case "branch":
      return (
        <span className="flex min-w-0 items-center gap-xxs text-caption text-subtle-foreground" data-node-branch={result.branch}>
          <GitBranchIcon aria-hidden="true" className="size-(--size-pr-icon) shrink-0" />
          <span className="truncate font-mono">{result.branch}</span>
        </span>
      );
    case "conclusion":
      return full ? (
        <span className="min-w-0 break-words text-caption" data-node-conclusion="full">{t("mission.result.conclusion")}: {result.line}</span>
      ) : (
        <span className="flex shrink-0 items-center text-caption text-foreground" title={result.line} data-node-conclusion="folded">
          {t("mission.result.conclusion")}
          <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />
        </span>
      );
    case "verify":
      return <span className={cn("min-w-0 text-caption text-muted-foreground", full ? "break-words" : "truncate")} title={result.line} data-node-verify="true">{result.line}</span>;
  }
}

type GraphCard = { key: string; node: MissionNode | null };

function Graph({ mission, selected, onSelect }: { mission: Mission; selected: string | null; onSelect: (key: string) => void }) {
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
  const byKey = useMemo(() => new Map(nodes.map((node) => [node.key, node])), [nodes]);
  const edgeLook = (edge: GraphEdge): EdgeLook => {
    const from = byKey.get(edge.from);
    const to = byKey.get(edge.to);
    // A stack's edge: the pull request below is the one above's base (B15).
    if (from && to && from.pr != null && to.base_pr === from.pr) return { tone: "text-pr-merged", dashed: true };
    // A line from a node at work is lit only while the mission is open; a cancelled mission's work stopped.
    if (from && (from.status === "working" || from.status === "review") && missionOpen(mission)) return { tone: "text-mission-working" };
    return { tone: "text-border" };
  };
  return (
    <DependencyGraphView
      graph={graph}
      idOf={(card) => card.key}
      flow="down"
      gaps={{ layers: "--size-mission-layer-gap", nodes: "--size-mission-node-gap" }}
      edgeLook={edgeLook}
      draw={(card) =>
        card.node ? <NodeCard key={card.key} mission={mission} node={card.node} selected={selected === card.key} onSelect={() => onSelect(card.key)} /> : <ReportCard key={card.key} mission={mission} />
      }
    />
  );
}

/** The worker's live row, whose status mark the card wears (B15). */
export function useWorkerRow(node: MissionNode) {
  return useShellStore((s) => {
    if (!node.pane_id) return null;
    const rows = agentsByPane(allAgents(s.agents, s.rest?.status?.remote));
    return rows.get(node.pane_id) ?? null;
  });
}

/**
 * A node on the map (B14-B16): its whole border is its state, a card with a
 * worker at it or a decision glows, and it carries its status mark, title,
 * badges, worker, result, a change node's four steps and a working node's
 * line. Pressing it opens its row in the 흐름 table below (B20).
 */
function NodeCard({ mission, node, selected, onSelect }: { mission: Mission; node: MissionNode; selected: boolean; onSelect: () => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const row = useWorkerRow(node);
  const launchModel = useShellStore((s) => launchModelOf(s.rest, mission, node));
  const tone = nodeTone(mission, node);
  const mark = shownMark(node, tone, row?.mark ?? null);
  const worker = workerWords(node, launchModel, t("mission.auto"));
  const moving = nodeMoving(mission, node);
  // The worker's own plan says where it is, else the Conductor's `--line` (PRD hide-plan B16, D-18).
  const nowText = moving ? ((node.plan ? planNowLine(node.plan, t) : null) ?? node.line) : null;
  const result = nodeResult(node);
  const staleTip = staleLabel({ stale: mission.github_stale, lastRead: mission.github_read_at_unix_ms }, t, requireInterfaceLanguage(i18n.language));
  // A node with no result and no worker at it still says why it waits or what it is (B51, B53).
  const quietLine = !result && !nowText ? nodeLine(node, t) : null;
  const attention = node.status === "failed" || node.blocked === "upstream_failed";
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-label={[node.title, t(`mission.status.${node.status}`), ...badgeWords(mission, node, t)].join(" · ")}
      aria-pressed={selected}
      data-dependency-node={node.key}
      data-mission-node={node.key}
      data-node-status={node.status}
      data-node-tone={tone}
      data-node-blocked={node.blocked ?? undefined}
      data-node-moving={moving || undefined}
      className={cn(
        "flex w-(--size-mission-node) min-w-0 flex-col gap-xxs rounded-md border bg-card px-sm py-xs text-left outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring",
        TONE_BORDER[tone],
        selected && "outline-1 outline-offset-2 outline-primary",
      )}
    >
      <span className="flex min-w-0 items-center gap-xs">
        <StatusMark mark={mark} />
        <span className={cn("min-w-0 flex-1 truncate text-body font-semibold", tone === "done" && "text-muted-foreground", node.status === "dropped" && "line-through")} title={node.title}>{node.title}</span>
        <Badges mission={mission} node={node} />
      </span>
      {worker || result || node.base_pr != null ? (
        <span className="flex min-w-0 items-center gap-xs text-caption text-subtle-foreground">
          {worker ? (
            <>
              <AgentMark kind={node.worker?.agent} />
              <span className="min-w-0 flex-1 truncate" data-node-worker={worker}>{worker}</span>
            </>
          ) : <span className="flex-1" />}
          {result ? (staleTip && result.kind === "pr" ? <Hint label={staleTip}><span className="flex min-w-0 shrink items-center"><ResultTail result={result} stale={mission.github_stale} /></span></Hint> : <ResultTail result={result} stale={mission.github_stale} />) : null}
          {node.base_pr != null ? <span className="shrink-0 font-mono text-caption text-pr-merged" data-node-base={node.base_pr}>↰ {t("mission.base", { number: String(node.base_pr) })}</span> : null}
        </span>
      ) : null}
      <Steps node={node} tone={tone} />
      {nowText ? <span className="min-w-0 truncate text-caption text-foreground" title={nowText} data-node-now={node.plan ? "plan" : "line"}>{nowText}</span> : null}
      {quietLine ? <span className={cn("min-w-0 truncate text-caption", attention ? "text-mission-failed" : "text-muted-foreground")} title={quietLine} data-node-line="true">{quietLine}</span> : null}
    </button>
  );
}

/** A worker is at the node; a cancelled mission's work stopped, whatever its last status said (B24). */
function nodeMoving(mission: Mission, node: MissionNode): boolean {
  return nodeTone(mission, node) !== "cancelled" && (node.status === "working" || node.status === "review");
}

/** A node's status mark: a cancelled mission's unfinished node is at rest, whatever its last status said (B24). */
function shownMark(node: MissionNode, tone: ReturnType<typeof nodeTone>, worker: RowMark | null): RowMark {
  return tone === "cancelled" ? "quiet" : nodeMark(node, worker);
}

/**
 * What a card's badges say, in its accessible name (B36): a badge inside the
 * card's button cannot take focus of its own, so the button speaks for it.
 */
function badgeWords(mission: Mission, node: MissionNode, t: TFunction): string[] {
  return [
    // The status word already says held; the badge adds the question of the node it stopped.
    ...(heldQuestion(mission, node) ? [heldQuestion(mission, node)!] : []),
    ...(node.attempts >= 2 ? [t("mission.badge.attempts", { count: node.attempts })] : []),
    ...(node.change ? [`${t("mission.changed")} · ${node.change}`] : []),
  ];
}

/** A decision stopped the node; a cancelled mission's decision is over with it. */
function showsHeld(mission: Mission, node: MissionNode): boolean {
  return node.status === "held" && nodeTone(mission, node) !== "cancelled";
}

function heldWords(mission: Mission, node: MissionNode, t: TFunction): string {
  const question = heldQuestion(mission, node);
  return question ? `${t("mission.status.held")} · ${question}` : t("mission.status.held");
}

/**
 * `!` for a node a decision stopped, `↻n` for a node tried twice or more, `✎`
 * for one the last replan changed, each explained on hover (B16); the card's
 * accessible name carries the same words for a keyboard or a screen reader.
 */
function Badges({ mission, node }: { mission: Mission; node: MissionNode }) {
  const { t } = useInterfaceTranslation();
  return (
    <>
      {showsHeld(mission, node) ? (
        <Hint label={heldWords(mission, node, t)}>
          <span className="shrink-0 rounded-xs bg-muted px-xxs font-mono text-micro font-semibold text-mission-hold" data-node-held="true">!</span>
        </Hint>
      ) : null}
      {node.attempts >= 2 ? (
        <Hint label={t("mission.badge.attempts", { count: node.attempts })}>
          <span className="shrink-0 rounded-xs bg-muted px-xxs font-mono text-micro text-mission-failed" data-node-attempts={node.attempts}>↻{node.attempts}</span>
        </Hint>
      ) : null}
      {node.change ? (
        <Hint label={`${t("mission.changed")} · ${node.change}`}>
          <span className="flex shrink-0 items-center rounded-xs bg-muted px-xxs text-mission-review" data-node-changed="true">
            <PencilIcon aria-hidden="true" className="size-(--size-icon-sm)" />
          </span>
        </Hint>
      ) : null}
    </>
  );
}

/** A change node's four steps (B15): code, PR or checks, review, merge or main, the step it is at in its state's colour; research and verify draw none. */
function Steps({ node, tone }: { node: MissionNode; tone: ReturnType<typeof nodeTone> }) {
  const { t } = useInterfaceTranslation();
  const steps = stepsOf(node);
  if (steps === 0) return null;
  const reached = node.stage ?? 0;
  const failing = node.status === "failed" || node.pr_state === "failed";
  const label = t("mission.step", { step: reached, steps: t(node.delivery === "direct" ? "mission.steps.direct" : "mission.steps.pr") });
  const current: LegendTone = failing ? "failed" : tone === "working" || tone === "review" || tone === "hold" || tone === "cancelled" ? tone : "done";
  return (
    <span role="img" aria-label={label} className="flex min-w-0 items-center gap-xxs" data-node-steps={reached}>
      {Array.from({ length: steps }, (_, index) => {
        const filled = index < reached;
        const last = index === reached - 1 && node.status !== "done";
        return <span key={index} aria-hidden="true" className={cn("h-(--size-mission-step-h) flex-1 rounded-full", !filled ? "bg-muted" : last ? TONE_FILL[current] : TONE_FILL.done)} />;
      })}
    </span>
  );
}

function ReportCard({ mission }: { mission: Mission }) {
  const { t } = useInterfaceTranslation();
  const stack = stackWaiting(mission);
  const note = mission.status === "done" ? t("mission.report.done") : mission.status === "merge_wait" && stack > 0 ? t("mission.report.stack") : mission.status === "cancelled" ? t("mission.cancelled") : t("mission.report.waiting");
  const done = mission.status === "done" || mission.status === "merge_wait";
  return (
    <div data-dependency-node={REPORT} data-mission-report={mission.status} className={cn("flex w-(--size-mission-node) items-center justify-center gap-xs rounded-xl border px-md py-sm", done ? "border-mission-done/60 bg-card" : "border-dashed border-mission-waiting")}>
      <StatusMark mark={mission.status === "done" ? "done" : "quiet"} />
      <span className="text-body font-semibold">{t("mission.report")}</span>
      <span className="text-caption text-muted-foreground">{note}</span>
    </div>
  );
}

const ROW_GRID = "grid grid-cols-[var(--size-agent-mark)_minmax(0,var(--size-flow-title))_var(--size-flow-result)_minmax(0,1fr)] items-center gap-sm";

/** `HH:MM` on the axis, with the day once the axis spans more than one (B17). */
function axisTime(language: InterfaceLanguage, at: number, long: boolean): string {
  return formatDateTime(language, at, long ? { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" } : { hour: "2-digit", minute: "2-digit" });
}

/** A bar's stretch in its colour (B18): worked green, review purple, a failed attempt red, stopped grey. */
const SEGMENT_FILL: Record<SegmentTone, string> = {
  worked: "bg-mission-done",
  review: "bg-mission-review",
  failed: "bg-mission-failed",
  paused: "bg-mission-waiting",
};

/** A mark's glyph on a bar or the axis (B18). */
const MARK_GLYPH: Record<FlowMarkKind, string> = { merged: "⑂", landed: "↓", asked: "!", replanned: "✎", cancelled: "×" };
const MARK_TONE: Record<FlowMarkKind, string> = {
  merged: "text-pr-merged",
  landed: "text-mission-done",
  asked: "text-mission-hold",
  replanned: "text-mission-review",
  cancelled: "text-mission-cancelled",
};

/** One mark on a bar or on the axis, its event's words on hover (B18). */
function FlowMark({ at, kind, event }: { at: number; kind: FlowMarkKind; event: MissionEvent }) {
  const { t } = useInterfaceTranslation();
  const words = eventWords(event, t);
  return (
    <Hint label={[words.what, words.line].filter(Boolean).join(" · ")}>
      <span className={cn("absolute top-1/2 -translate-x-1/2 -translate-y-1/2 font-mono text-micro font-semibold leading-none", MARK_TONE[kind])} style={{ left: `${at}%` }} data-flow-mark={kind}>
        {MARK_GLYPH[kind]}
      </span>
    </Hint>
  );
}

/**
 * The 흐름 table (B17-B21): a row per node in the order they wait on each
 * other, its state, title and kind, result and its bar on the mission's time
 * axis, drawn from its events when the mission has them and from its start
 * and finish otherwise. `전체 · 지금 · 끝남` filters the rows; pressing a
 * row, or Enter on it, opens it.
 */
function FlowTable({ mission, log, filter, onFilter, expanded, onExpand, actions }: { mission: Mission; log: MissionEventsSnapshot | null; filter: FlowFilter; onFilter: (filter: FlowFilter) => void; expanded: string | null; onExpand: (key: string | null) => void; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  // The clock ticks by the minute, so an open mission's axis runs to the end
  // of the current minute: work that started or ended within it stays on it.
  const now = useMinuteClock() + 60_000;
  const span = flowSpan(mission, now);
  const rows = orderedNodes(mission).filter((node) => flowShows(node, filter));
  const long = span.to - span.from > 86_400_000;
  const open = mission.closed_at_unix_ms == null;
  const ticks = Array.from({ length: AXIS_TICKS }, (_, index) => {
    const at = span.from + ((span.to - span.from) * index) / (AXIS_TICKS - 1);
    return index === AXIS_TICKS - 1 && open ? t("mission.flow.nowTick") : axisTime(language, at, long);
  });
  const marks = log ? axisMarks(log.events, span) : [];
  return (
    <section className="flex min-w-0 flex-col border-t border-border px-md pb-md pt-sm" data-mission-flow={filter}>
      <div className="flex min-w-0 items-center gap-sm pb-xs">
        <h3 className="text-body font-semibold">{t("mission.flow")}</h3>
        <span className="flex-1" />
        <ToggleGroup type="single" value={filter} onValueChange={(value) => value && onFilter(value as FlowFilter)} aria-label={t("mission.flow.filter")} data-mission-flow-filter={filter}>
          <ToggleGroupItem size="sm" value="all" data-mission-flow-choice="all">{t("mission.flow.all")}</ToggleGroupItem>
          <ToggleGroupItem size="sm" value="now" data-mission-flow-choice="now">{t("mission.flow.now")}</ToggleGroupItem>
          <ToggleGroupItem size="sm" value="finished" data-mission-flow-choice="finished">{t("mission.flow.finished")}</ToggleGroupItem>
        </ToggleGroup>
      </div>
      <div className={cn(ROW_GRID, "px-xs pb-xxs text-micro text-muted-foreground")} aria-hidden="true">
        <span />
        <span>{t("mission.flow.task")}</span>
        <span>{t("mission.flow.result")}</span>
        <span className="flex justify-between tabular-nums" data-mission-flow-axis={ticks.join("|")}>
          {ticks.map((tick, index) => <span key={index}>{tick}</span>)}
        </span>
      </div>
      {marks.length > 0 ? (
        <div className={cn(ROW_GRID, "px-xs")}>
          <span />
          <span />
          <span />
          <span className="relative h-(--size-flow-track) min-w-0" data-flow-axis-marks={marks.map((mark) => mark.kind).join(" ")}>
            {marks.map((mark, index) => <FlowMark key={index} {...mark} />)}
          </span>
        </div>
      ) : null}
      {log && log.dropped > 0 ? <p className="px-xs py-xxs text-caption text-muted-foreground" data-mission-flow-dropped={log.dropped}>{t("mission.flow.dropped", { count: log.dropped })}</p> : null}
      {rows.length === 0 ? <p className="px-xs py-xs text-caption text-muted-foreground" data-mission-flow-empty="true">{t("mission.flow.empty")}</p> : null}
      <ul className="flex min-w-0 flex-col">
        {rows.map((node) => (
          <FlowRow key={node.key} mission={mission} node={node} events={log?.events ?? null} span={span} open={expanded === node.key} onOpen={(next) => onExpand(next ? node.key : null)} actions={actions} />
        ))}
      </ul>
    </section>
  );
}

function FlowRow({ mission, node, events, span, open, onOpen, actions }: { mission: Mission; node: MissionNode; events: MissionEvent[] | null; span: { from: number; to: number }; open: boolean; onOpen: (open: boolean) => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const row = useWorkerRow(node);
  const ref = useRef<HTMLButtonElement>(null);
  const tone = nodeTone(mission, node);
  // With its events the bar is its stretches and marks (B18); without, one bar from its times (B19).
  const drawn = events ? nodeSegments(mission, node, events, span) : null;
  const marks = drawn && events ? nodeMarks(node, events, span) : [];
  const bar = drawn ? null : nodeBar(node, span);
  const liveEnd = drawn?.live ? drawn.segments[drawn.segments.length - 1] : null;
  const result = nodeResult(node);
  const kind = shortKind(node);
  // An opened row comes into view, as a card pressed on the map asks (B20).
  useEffect(() => {
    if (open) ref.current?.scrollIntoView?.({ block: "nearest" });
  }, [open]);
  return (
    <li className="flex min-w-0 flex-col" data-flow-row={node.key} data-node-tone={tone}>
      <button
        ref={ref}
        type="button"
        aria-expanded={open}
        onClick={() => onOpen(!open)}
        className={cn(ROW_GRID, "rounded-xs px-xs py-xxs text-left text-body outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring", open && "bg-accent", (tone === "waiting" || tone === "cancelled" || tone === "dropped") && "text-muted-foreground")}
      >
        <StatusMark mark={shownMark(node, tone, row?.mark ?? null)} label={t(`mission.status.${node.status}`)} />
        <span className="flex min-w-0 items-baseline gap-xs">
          <span className={cn("min-w-0 truncate", node.status === "dropped" && "line-through")} title={node.title}>{node.title}</span>
          {kind !== "mission.kind.short.pr" ? <span className="shrink-0 text-micro text-muted-foreground">{t(kind)}</span> : null}
        </span>
        <span className="flex min-w-0 items-center">{result ? <ResultTail result={result} stale={mission.github_stale} /> : null}</span>
        <span className="relative h-(--size-flow-track) min-w-0 rounded-xs bg-muted/30" data-flow-track={node.key} data-flow-drawn={drawn ? "events" : "times"}>
          {drawn?.segments.map((segment, index) => (
            <span
              key={index}
              className={cn("absolute top-1/2 h-(--size-flow-bar) -translate-y-1/2 rounded-xs", SEGMENT_FILL[segment.tone])}
              style={{ left: `${segment.left}%`, width: `max(${segment.width}%, var(--size-hairline))` }}
              data-flow-segment={segment.tone}
            />
          ))}
          {liveEnd ? (
            <span
              aria-hidden="true"
              className="absolute top-1/2 size-(--size-flow-bar) -translate-x-1/2 -translate-y-1/2 rounded-full bg-mission-working motion-safe:animate-pulse"
              style={{ left: `${liveEnd.left + liveEnd.width}%` }}
              data-flow-live="true"
            />
          ) : null}
          {marks.map((mark, index) => <FlowMark key={index} {...mark} />)}
          {bar ? (
            <span
              className={cn("absolute top-1/2 h-(--size-flow-bar) -translate-y-1/2 rounded-xs", TONE_FILL[tone === "dropped" ? "cancelled" : tone], tone === "waiting" && "bg-mission-waiting")}
              style={{ left: `${bar.left}%`, width: `max(${bar.width}%, var(--size-hairline))` }}
              data-flow-bar={`${bar.left.toFixed(1)} ${bar.width.toFixed(1)}`}
            />
          ) : null}
        </span>
      </button>
      {open ? <FlowDetail mission={mission} node={node} events={events} actions={actions} /> : null}
    </li>
  );
}

/**
 * An opened row (B19, B20): the node's events in order, what the map card
 * has no room for, the way to its terminal, its record and its PR card, and
 * the worker's own plan, folded. A node with no events, as in a mission
 * recorded before them, shows when it started and finished instead.
 */
function FlowDetail({ mission, node, events, actions }: { mission: Mission; node: MissionNode; events: MissionEvent[] | null; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const row = useWorkerRow(node);
  // Derived outside the selector: it builds a new session each call, which a selector must not return.
  const rest = useShellStore((s) => s.rest);
  const session = useMemo(() => (row ? sessionOfAgent(row, rest) : null), [row, rest]);
  const depends = node.depends_on.map((key) => mission.nodes.find((other) => other.key === key)?.title ?? key);
  const worker = node.worker;
  // A moving node's line says where it is, a waiting, held or failed one's why;
  // once a research or verify node ended the line is what it left (B22), and
  // a done or cancelled node has no current line.
  const result = nodeResult(node);
  const resultLine = result?.kind === "conclusion" || result?.kind === "verify" ? result : null;
  const current = nodeTone(mission, node) !== "cancelled" && node.status !== "done" && node.status !== "dropped";
  // A stopped node's own line says why; the worker's plan says where it is only while it moves.
  const nowLine = !current || resultLine ? null : nodeMoving(mission, node) ? ((node.plan ? planNowLine(node.plan, t) : null) ?? node.line) : node.line;
  const time = (at: number) => formatDateTime(language, at, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });
  const own = events?.filter((event) => event.node === node.key) ?? [];
  const fields: [string, ReactNode][] = [
    [t("mission.field.kind"), t(kindWord(node.kind))],
    ...(worker ? [[t("mission.field.agent"), (
      <span className="flex min-w-0 items-center gap-xs"><AgentMark kind={worker.agent} /><span className="truncate">{[worker.agent, worker.model, worker.effort_auto && worker.effort ? t("mission.effortAuto", { effort: worker.effort }) : worker.effort].filter(Boolean).join(" · ")}</span></span>
    )] as [string, ReactNode]] : []),
    ...(node.attempts > 0 ? [[t("mission.field.attempts"), String(node.attempts)] as [string, ReactNode]] : []),
    ...(depends.length > 0 ? [[t("mission.field.depends"), depends.join(", ")] as [string, ReactNode]] : []),
    ...(stepsOf(node) > 0 ? [[t("mission.field.pr"), <span key="s" className="text-muted-foreground" data-node-step-names="true">{t(node.delivery === "direct" ? "mission.steps.direct" : "mission.steps.pr")}</span>] as [string, ReactNode]] : []),
    ...(node.review ? [[t("mission.field.review"), node.review] as [string, ReactNode]] : []),
    ...(nowLine ? [[t("mission.field.now"), nowLine] as [string, ReactNode]] : []),
    ...(resultLine ? [[t(resultLine.kind === "conclusion" ? "mission.result.conclusion" : "mission.result.verify"), <span key="r" data-flow-result={resultLine.kind}>{resultLine.line}</span>] as [string, ReactNode]] : []),
    ...(node.change ? [[t("mission.field.change"), node.change] as [string, ReactNode]] : []),
    ...(node.criteria ? [[t("mission.field.criteria"), node.criteria] as [string, ReactNode]] : []),
  ];
  return (
    <div className="ml-lg flex min-w-0 flex-col gap-sm border-l border-border py-xs pl-md" data-mission-detail={node.key}>
      {own.length > 0 ? (
        <ol aria-label={t("mission.flow.events")} className="grid grid-cols-[auto_auto_1fr] gap-x-md gap-y-xxs text-caption" data-flow-events={own.length}>
          {own.map((event, index) => {
            const words = eventWords(event, t);
            return (
              <li key={index} className="contents" data-flow-event={event.kind}>
                <span className="text-muted-foreground tabular-nums">{time(event.at_unix_ms)}</span>
                <span className="font-semibold">{words.what}</span>
                <span className="min-w-0 break-words text-muted-foreground">{words.line}</span>
              </li>
            );
          })}
        </ol>
      ) : node.started_at_unix_ms != null ? (
        <p className="flex flex-wrap gap-x-md text-caption text-muted-foreground tabular-nums" data-flow-times="true">
          <span>{t("mission.flow.started", { time: time(node.started_at_unix_ms) })}</span>
          {node.finished_at_unix_ms != null ? <span>{t("mission.flow.ended", { time: time(node.finished_at_unix_ms) })}</span> : null}
        </p>
      ) : null}
      <dl className="grid grid-cols-[auto_1fr] gap-x-md gap-y-xxs text-caption">
        {fields.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-muted-foreground">{label}</dt>
            <dd className="min-w-0 break-words">{value}</dd>
          </div>
        ))}
      </dl>
      <div className="flex flex-wrap gap-xs">
        <Button size="sm" disabled={!node.pane_id || !row} onClick={() => node.pane_id && actions.openAgent(node.pane_id)} data-node-terminal="true">{t("mission.goTerminal")}</Button>
        <Button size="sm" variant="secondary" disabled={!session} onClick={() => session && actions.openChildRecord(session)} data-node-record="true">{t("mission.record")}</Button>
        {node.pr != null ? (
          <Button size="sm" variant="secondary" disabled={!node.pr_url} onClick={() => node.pr_url && actions.openSessionPullRequest({ workspace_id: mission.workspace_id, url: node.pr_url, number: node.pr! })} data-node-pr-card="true">{t("mission.prCard")}</Button>
        ) : null}
      </div>
      {node.plan ? <OwnPlan plan={node.plan} /> : null}
    </div>
  );
}

/** The worker's own plan in a node's row, folded (B20): its head says only when it was updated, and opening it lists the steps. */
function OwnPlan({ plan }: { plan: PlanView }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const elapsed = useElapsed(plan.updated_at_unix_ms);
  return (
    <div className="flex min-w-0 flex-col gap-xs" data-node-plan={open ? "open" : "folded"}>
      <button type="button" aria-expanded={open} onClick={() => setOpen(!open)} className="flex min-w-0 items-center gap-xs rounded-xs text-left text-body outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" data-node-plan-toggle="true">
        <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon-sm) shrink-0 text-muted-foreground transition-transform", open && "rotate-90")} />
        <span className="font-semibold">{t("plan.own")}</span>
        {elapsed ? <span className="text-caption text-muted-foreground">· {t("plan.updated", { elapsed })}</span> : null}
      </button>
      {open ? <div className="min-w-0 text-caption"><PlanChecklist plan={plan} /></div> : null}
    </div>
  );
}

