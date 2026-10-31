import { CheckIcon, GitBranchIcon, PencilIcon } from "lucide-react";
import { useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { planNowLine } from "./plans";
import type { Actions } from "./actions";
import { DependencyGraphView, type EdgeLook, type OverlayEdge } from "./DependencyGraph";
import type { GraphEdge } from "./dependencyLayout";
import { formatElapsed, useElapsed, useMinuteClock } from "./components/elapsed";
import { ConductorRow } from "./components/mission-conductor-row";
import { MissionEnd } from "./components/mission-end";
import { MissionNodeMark } from "./components/mission-node-mark";
import { MissionPrMark } from "./components/mission-pr";
import { PullList, ResultTiles } from "./components/mission-results";
import { satelliteLines, Satellites } from "./components/mission-satellites";
import { MissionSummary } from "./components/mission-summary";
import { HEAD_FILL, ProgressCells } from "./components/mission-tone";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { MarkdownText } from "./MarkdownText";
import { markdownPlainText } from "./markdownPlain";
import { MissionMapCanvas } from "./MissionMapCanvas";
import { MissionNodePanel } from "./MissionNodePanel";
import { MissionTurn } from "./MissionTurn";
import { AgentMark } from "./AgentMark";
import { lookWord, RunText, useNodeTimes, useRunWords } from "./missionParts";
import {
  cardLine,
  criteriaTally,
  defectEdges,
  heldQuestion,
  landingLeft,
  missionHeadCells,
  missionCounts,
  missionEventsOf,
  missionOf,
  missionOpen,
  neighbours,
  nodeLook,
  nodeResult,
  ownCriteria,
  reportLine,
  shortCommit,
  stackWaiting,
  stageWord,
  waitingFor,
  type NodeLook,
} from "./missions";
import { layerDependencies } from "./projectBoard";
import { machineName } from "./screenMachine";
import type { Mission, MissionEvent, MissionNode, MissionStatus } from "./snapshot";
import { useShellStore } from "./store";
import type { MessageKey } from "./i18n/catalogs";
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
  const [picked, setPicked] = useState<string | null>(null);
  const panel = useRef<HTMLDivElement>(null);
  const [root, rootRef] = useState<HTMLDivElement | null>(null);
  const [page, pageRef] = useState<HTMLDivElement | null>(null);
  const wide = useWide(root);
  const device = useShellStore((s) => (mission.stale ? machineName(s.rest?.navigator?.devices, mission.device_id, t) : null));
  const log = useShellStore((s) => missionEventsOf(s.rest, mission.id));
  const open = missionOpen(mission);
  // The page's clock ticks by the minute; a closed mission's times end where it closed.
  const minute = useMinuteClock();
  const end = mission.closed_at_unix_ms ?? minute;
  // A pick whose node a replan took away is no pick.
  const selected = picked && mission.nodes.some((node) => node.key === picked) ? picked : null;
  const node = selected ? mission.nodes.find((candidate) => candidate.key === selected)! : null;
  const select = (key: string | null) => {
    setPicked(key);
    // Under a narrow map the panel may be out of view; it comes into it (B34).
    if (key) requestAnimationFrame(() => panel.current?.scrollIntoView?.({ block: "nearest" }));
  };
  // Escape lets go of the pick before the lane answers it (B30).
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Escape" || event.defaultPrevented || !selected) return;
    event.preventDefault();
    setPicked(null);
  };
  return (
    <div ref={rootRef} className="@container/mission flex min-h-0 min-w-0 flex-1 flex-col" data-mission={mission.id} data-mission-status={mission.status} data-mission-stale={mission.stale || undefined} onKeyDown={onKeyDown}>
      <div ref={pageRef} className="flex min-h-0 min-w-0 flex-1 flex-col overflow-auto mission-wide:overflow-hidden" data-mission-scroll="true">
        <Head mission={mission} actions={actions} />
        {/* A finished mission leads with what it left (B20); its report's line follows, then its Conductor. */}
        {!open ? <ResultTiles mission={mission} /> : null}
        {!open ? <PullList mission={mission} onSelect={select} /> : null}
        {mission.report && !open ? <ReportLine mission={mission} /> : null}
        <ConductorRow mission={mission} actions={actions} />
        <MissionTurn mission={mission} device={device} actions={actions} onSelect={select} />
        {mission.delay && open ? <p className="px-md pb-xs text-caption text-muted-foreground" data-mission-delay="true">{t("mission.delay", { line: mission.delay })}</p> : null}
        {device ? <p className="px-md pb-xs text-caption text-muted-foreground" data-mission-stale-note="true">{t("mission.stale", { device })}</p> : null}
        {mission.nodes.length === 0 ? (
          <p className="px-md py-sm text-body text-muted-foreground" data-mission-planning="true">{t("mission.planning")}</p>
        ) : (
          <div className={cn("flex min-w-0 flex-col border-t border-border mission-wide:min-h-0 mission-wide:flex-1 mission-wide:flex-row", mission.stale && "opacity-(--opacity-dimmed)")} data-mission-values={mission.stale ? "stale" : "live"}>
            <div
              className="flex min-w-0 flex-col mission-wide:min-h-0 mission-wide:flex-1"
              onClick={(event: MouseEvent<HTMLDivElement>) => {
                // A press on the map outside every card and its checkers lets go of the pick (B30).
                // A checker's popover is portalled out of the map, yet its presses bubble here through React.
                if (!event.currentTarget.contains(event.target as Node)) return;
                if (!(event.target as Element).closest("[data-mission-node], [data-node-satellites]")) setPicked(null);
              }}
              data-mission-map="true"
            >
              <MissionMapCanvas wide={wide} page={page}>
                <Graph
                  actions={actions}
                  mission={mission}
                  selected={selected}
                  onSelect={(key) => {
                    select(selected === key ? null : key);
                    const picked = selected === key ? undefined : mission.nodes.find((one) => one.key === key);
                    if (picked) onNode?.(picked);
                  }}
                  events={log?.events ?? null}
                  end={end}
                />
              </MissionMapCanvas>
            </div>
            {/* The column beside the map: a pick's panel, else the mission's summary (B19, B30, B31). */}
            <div
              ref={panel}
              className="flex min-w-0 flex-col border-t border-border mission-wide:min-h-0 mission-wide:w-(--size-mission-side) mission-wide:shrink-0 mission-wide:overflow-auto mission-wide:border-l mission-wide:border-t-0"
              data-mission-panel={node?.key}
            >
              {node ? (
                <MissionNodePanel mission={mission} node={node} events={log?.events ?? null} end={end} onSelect={select} onClose={() => setPicked(null)} actions={actions} />
              ) : (
                <MissionSummary mission={mission} onSelect={select} />
              )}
            </div>
          </div>
        )}
      </div>
    </div>
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

/** The pill's words and colour: the operator's merge waits in the decision colour, a finished mission is grey (D-02). */
export const PILL: Record<MissionStatus, { key: MessageKey; tone: string }> = {
  planning: { key: "mission.pill.planning", tone: "bg-muted text-muted-foreground" },
  running: { key: "mission.pill.running", tone: "bg-mission-working/15 text-mission-working" },
  merge_wait: { key: "mission.pill.mergeWait", tone: "bg-mission-hold/15 text-mission-hold" },
  done: { key: "mission.pill.done", tone: "bg-muted text-muted-foreground" },
  cancelled: { key: "mission.pill.cancelled", tone: "bg-muted text-mission-cancelled" },
};

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
  const took = mission.started_at_unix_ms != null && mission.closed_at_unix_ms != null ? formatElapsed(language, mission.closed_at_unix_ms - mission.started_at_unix_ms) : null;
  const time = took ? t("mission.elapsed.total", { elapsed: took }) : running ? t("mission.elapsed.running", { elapsed: running }) : null;
  const pill = PILL[mission.status];
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-md gap-y-xs px-md py-sm text-body" data-mission-summary="true">
      <span className={cn("flex shrink-0 items-center gap-xxs rounded-full px-sm text-caption font-semibold", pill.tone)} data-mission-pill={mission.status}>
        {mission.status === "done" ? <CheckIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : null}
        {t(pill.key)}
      </span>
      {time ? <span className="shrink-0 text-caption text-muted-foreground" data-mission-stat="time">{time}</span> : null}
      {mission.nodes.length > 0 ? (
        <>
          <ProgressCells tones={missionHeadCells(mission)} fill={HEAD_FILL} size="head" className="max-w-(--size-pr-popover)" />
          <span className="shrink-0 text-caption text-muted-foreground tabular-nums" data-mission-stat="done">
            {t("mission.doneOf", { done: String(done), total: String(total) })}
          </span>
        </>
      ) : null}
      {mission.strength ? (
        <span className="shrink-0 text-caption text-muted-foreground" data-mission-stat="strength" data-mission-strength={mission.strength}>
          {t("mission.head.strength", { strength: t(`mission.strength.${mission.strength}`) })}
        </span>
      ) : null}
      {/* An end the Conductor was asked for says so until it closes the mission. */}
      {mission.end_asked && mission.conductor.present && missionOpen(mission) ? <span className="shrink-0 text-caption text-mission-hold" data-mission-end-asked="true">{t("mission.end.askedNote")}</span> : null}
      <span className="ml-auto shrink-0"><MissionEnd mission={mission} actions={actions} /></span>
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

function Graph({ mission, selected, onSelect, events, end, actions }: { mission: Mission; selected: string | null; onSelect: (key: string) => void; events: MissionEvent[] | null; end: number; actions: Actions }) {
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
    if (from && to && from.pr != null && to.base_pr === from.pr) return { tone: "text-mission-review", dashed: true, dimmed };
    // A line from a node at work is lit only while the mission is open; a cancelled mission's work stopped.
    if (from && (from.status === "working" || from.status === "review") && missionOpen(mission)) return { tone: "text-mission-working", dimmed };
    return { tone: "text-border", dimmed };
  };
  // A defect a later node found is a red dashed edge back to the node it opened, until its line passes (B21).
  const overlay: OverlayEdge[] = defects.map((edge) => ({ ...edge, look: { tone: "text-mission-failed", dashed: true, dimmed: away(edge) } }));
  return (
    <DependencyGraphView
      graph={graph}
      idOf={(card) => card.key}
      flow="down"
      gaps={{ layers: "--size-mission-layer-gap", nodes: "--size-mission-node-gap" }}
      edgeLook={edgeLook}
      overlay={overlay}
      draw={(card) =>
        card.node ? (
          <NodeCard key={card.key} mission={mission} node={card.node} actions={actions} selected={selected === card.key} dimmed={lit !== null && !lit.has(card.key)} onSelect={() => onSelect(card.key)} events={events} end={end} />
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
  review: "border-mission-review ring-2 ring-mission-review/20",
  hold: "border-mission-hold ring-2 ring-mission-hold/20",
  failed: "border-mission-failed",
  done: "border-border bg-mission-done/10",
  external: "border-dotted border-mission-waiting bg-transparent",
  cancelled: "border-dashed border-mission-cancelled bg-transparent",
  dropped: "border-dashed border-border bg-transparent",
};

/**
 * A node on the map (B26-B28; PRD mission-tool B16, B17): its state's mark
 * and border, its title on one line that ends in an ellipsis (the whole
 * title on hover and in its accessible name), and one line of facts: who
 * works it (its agent's face and the model and effort as a checker's are
 * named), its pull request's mark or result, else its stage while it moves,
 * what a held, failed or waiting node waits on, `머지 남음` or `합치기 남음`,
 * how long it took or a decision held it, and `기준 n/m` with `⤳k`. The other agents checking it stand beside it as
 * satellites tethered to its right side. Pressing it picks it (B30).
 */
function NodeCard({ mission, node, selected, dimmed, onSelect, events, end, actions }: { mission: Mission; node: MissionNode; selected: boolean; dimmed: boolean; onSelect: () => void; events: MissionEvent[] | null; end: number; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const look = nodeLook(mission, node);
  const moving = look === "working" || look === "review";
  // The worker's own plan says where it is, else the Conductor's `--line` (PRD hide-plan B16, D-18).
  const line = cardLine(mission, node, moving && node.plan ? planNowLine(node.plan, t) : null, t);
  const result = cardResult(node, mission.github_stale, t);
  // A moving node's line is the summary's and the panel's; its card says its stage (B16, B19).
  const stage = moving && !result ? t(stageWord(node)) : null;
  const shown = line && !moving ? line : null;
  const waits = shown && look === "waiting" && !node.blocked && node.kind !== "external" ? waitingFor(mission, node) : [];
  const tally = criteriaTally(mission, node);
  const landing = landingLeft(node);
  const times = useNodeTimes(mission, node, events, end);
  const time = look === "hold" ? (times?.held ? t("mission.time.held", { elapsed: times.held }) : null) : times?.took ?? null;
  // Who works it, from the same launch proof as a checker's; an outside node has no worker.
  const run = useRunWords(mission, node.pane_id, node.machine, node.worker);
  const worker = node.worker ? node.worker.agent : null;
  // A mission from before criteria shows its sentence where the count would be (B23).
  const before = mission.exempt ? ownCriteria(node) : null;
  const satellites = satelliteLines(node.criteria);
  const word = lookWord(look, t);
  const card = (
    <button
      type="button"
      onClick={onSelect}
      aria-label={[node.title, word, ...(run ? [run.text] : []), ...(line && line.text !== word ? [line.text] : []), ...badgeWords(mission, node, t), ...(tally ? [criteriaWords(tally, t)] : [])].join(" · ")}
      aria-pressed={selected}
      data-mission-node={node.key}
      data-node-status={node.status}
      data-node-look={look}
      data-node-blocked={node.blocked ?? undefined}
      data-node-dimmed={dimmed || undefined}
      className={cn(
        "flex w-(--size-mission-node) min-w-0 flex-col gap-xxs rounded-md border bg-card px-sm py-xs text-left outline-none transition-opacity hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring",
        LOOK_CARD[look],
        // Outside is the node's kind: the dotted border stays in whatever state it ends (B19).
        node.kind === "external" && "border-dotted",
        selected && "outline-2 outline-offset-2 outline-primary",
        dimmed && "opacity-(--opacity-dimmed)",
      )}
    >
      <span className="flex min-w-0 items-center gap-xs">
        <MissionNodeMark look={look} label={word} />
        <span className={cn("min-w-0 flex-1 truncate text-body font-semibold", (look === "external" || look === "cancelled" || look === "waiting") && "text-muted-foreground", look === "dropped" && "text-muted-foreground line-through")} title={node.title}>
          {node.title}
        </span>
        <Badges mission={mission} node={node} />
      </span>
      <span className="flex min-w-0 items-center gap-xs text-caption text-muted-foreground">
        {worker ? <span className="flex shrink-0" title={run?.text} data-node-worker={worker}><AgentMark kind={worker} /></span> : null}
        {/* One line laid out from the right that wraps out of sight: the count or the old sentence, the result or
            stage and the landing tag keep their place, and the line and the worker's model and effort truncate first,
            the planned mark never, leaving the line once they cannot keep a readable width; the face then stands
            alone with its hover and the card's name keeping them. The facts start beside the worker's mark (`justify-end`
            packs a reversed row to the left), and the free space goes before the count. */}
        <span className="flex h-lh min-w-0 flex-1 flex-row-reverse flex-wrap items-center justify-end gap-x-xs overflow-hidden" data-node-row="true">
          {tally ? (
            <span className="ml-auto shrink-0 tabular-nums" data-node-criteria={`${tally.passed}/${tally.total}`}>
              {t("mission.criteria.count", { passed: String(tally.passed), total: String(tally.total) })}
              {tally.skipped > 0 ? <span className="ml-xxs text-mission-hold" data-node-skipped={tally.skipped}>⤳{tally.skipped}</span> : null}
            </span>
          ) : before ? (
            <span className="min-w-0 flex-1 truncate" title={before} data-node-old-criteria="true">{before}</span>
          ) : null}
          {shown ? (
            <span className="flex min-w-auto max-w-max flex-1 items-center" title={shown.text}>
              <span className="grid min-w-(--size-mission-run-min) grid-cols-[minmax(0,1fr)]">
                <span className={cn("flex min-w-0 items-center gap-xxs overflow-hidden", shown.attention && "text-mission-failed")} data-node-line={shown.attention ? "attention" : "true"}>
                  {waits.length > 0 ? <WaitsFor nodes={waits} stale={mission.github_stale} /> : <span className="truncate">{shown.text}</span>}
                </span>
              </span>
            </span>
          ) : null}
          {result}
          {stage ? <span className="shrink-0" data-node-stage={node.stage ?? node.kind}>{stage}</span> : null}
          {landing ? <span className="shrink-0 rounded-xs bg-mission-hold/15 px-xs text-micro font-semibold text-mission-hold" data-node-landing={landing}>{t(landing === "merge" ? "mission.landing.merge" : "mission.landing.join")}</span> : null}
          {time ? <span className="shrink-0 tabular-nums" data-node-time={look === "hold" ? "held" : "took"}>{time}</span> : null}
          {run ? (
            <RunText run={run} className="min-w-auto max-w-max flex-1" wordsClassName="min-w-(--size-mission-run-min)">
              {result || stage || landing || shown || time ? <span aria-hidden="true" className="shrink-0">·</span> : null}
            </RunText>
          ) : null}
        </span>
      </span>
    </button>
  );
  // The card and its satellites are one box to the layout, the card centred in it, so edges meet the card's middle.
  // An empty column as wide as the satellites' keeps the card in the middle of the box.
  if (satellites.length === 0) return <div data-dependency-node={node.key} className="flex">{card}</div>;
  return (
    <div data-dependency-node={node.key} className="grid grid-cols-[var(--spacing-xl)_auto_var(--spacing-xl)] items-center">
      <span aria-hidden="true" />
      {card}
      {/* 노드 상세 opens the node's panel; it never shuts the panel already open, as a second press on the card does. */}
      <Satellites mission={mission} lines={satellites} dimmed={dimmed} onSelect={() => !selected && onSelect()} actions={actions} />
    </div>
  );
}

/** What a waiting node waits for, by its pull request's mark where it has one, else its title, in the line's own words (B18, B28). */
function WaitsFor({ nodes, stale }: { nodes: MissionNode[]; stale: boolean }) {
  const { t } = useInterfaceTranslation();
  const [head, tail] = t("mission.line.waitsFor", { names: "\u0000" }).split("\u0000");
  return (
    <span className="flex min-w-0 items-center gap-xxs">
      {head ? <span className="shrink-0">{head.trim()}</span> : null}
      {nodes.map((dep) => (dep.pr != null ? <MissionPrMark key={dep.key} number={dep.pr} state={dep.pr_state} stale={stale} className="shrink" /> : <span key={dep.key} className="min-w-0 truncate">{dep.title}</span>))}
      {tail ? <span className="min-w-0 truncate">{tail.trim()}</span> : null}
    </span>
  );
}

/** What a card names as its result (B28): its pull request's mark, its branch, or its commit on main, whichever the record holds. */
function cardResult(node: MissionNode, stale: boolean, t: TFunction<"translation">): ReactNode {
  const result = nodeResult(node);
  // An outside node's pull request or branch carries `↗` in every state (B19).
  if (node.kind === "external" && node.external) {
    return (
      <span className="flex min-w-0 shrink items-center gap-xxs font-mono text-caption text-muted-foreground" data-node-outside="true">
        <span aria-hidden="true" className="shrink-0">↗</span>
        {"pr" in node.external
          ? <MissionPrMark number={node.external.pr} state={node.pr_state} stale={stale} data-node-pr={node.external.pr} />
          : <span className="truncate" title={node.external.branch} data-node-branch={node.external.branch}>{node.external.branch}</span>}
      </span>
    );
  }
  if (!result) return null;
  switch (result.kind) {
    // A pull request carries the one PR mark, its state's icon and colour, as the panel does; one Hide never read keeps the muted icon (B18).
    case "pr":
      return <MissionPrMark number={result.number} state={result.state} stale={stale} data-node-pr={result.number} />;
    case "commit":
      return <span className="shrink-0 font-mono text-caption text-muted-foreground" title={result.commit} data-node-commit={result.commit}>{t("mission.result.main", { commit: shortCommit(result.commit) })}</span>;
    case "branch":
      return (
        <span className="flex min-w-0 shrink items-center gap-xxs text-caption text-muted-foreground" data-node-branch={result.branch}>
          <GitBranchIcon aria-hidden="true" className="size-(--size-pr-icon) shrink-0" />
          <span className="truncate font-mono" title={result.branch}>{result.branch}</span>
        </span>
      );
    default:
      return null;
  }
}

/** `기준 n/m`, with the skipped count, in words for the card's accessible name. */
function criteriaWords(tally: { passed: number; total: number; skipped: number }, t: TFunction<"translation">): string {
  const count = t("mission.criteria.count", { passed: String(tally.passed), total: String(tally.total) });
  return tally.skipped > 0 ? `${count} · ${t("mission.criteria.skipped", { count: tally.skipped })}` : count;
}

/** What a card's badges say, in its accessible name: a badge inside the card's button cannot take focus of its own, so the button speaks for it. */
function badgeWords(mission: Mission, node: MissionNode, t: TFunction): string[] {
  return [
    // A finished outside node still says it is outside, which its state word no longer does.
    ...(node.kind === "external" && nodeLook(mission, node) !== "external" ? [t("mission.look.external")] : []),
    ...(heldQuestion(mission, node) ? [heldQuestion(mission, node)!] : []),
    ...(node.reopened > 0 ? [t("mission.badge.reopened", { count: node.reopened })] : []),
    ...(node.change ? [`${t("mission.changed")} · ${node.change}`] : []),
  ];
}

/**
 * `↻n` for a node a later node's defect opened again (B21) and `✎` for one
 * the last replan changed, each explained on hover; the card's accessible
 * name carries the same words.
 */
function Badges({ mission, node }: { mission: Mission; node: MissionNode }) {
  const { t } = useInterfaceTranslation();
  if (node.reopened === 0 && !node.change) return null;
  return (
    <>
      {node.reopened > 0 ? (
        <Hint label={t("mission.badge.reopened", { count: node.reopened })}>
          <span className="shrink-0 rounded-xs bg-muted px-xxs font-mono text-micro text-mission-failed" data-node-reopened={node.reopened}>↻{node.reopened}</span>
        </Hint>
      ) : null}
      {node.change && missionOpen(mission) ? (
        <Hint label={`${t("mission.changed")} · ${node.change}`}>
          <span className="flex shrink-0 items-center rounded-xs bg-muted px-xxs text-muted-foreground" data-node-changed="true">
            <PencilIcon aria-hidden="true" className="size-(--size-icon-sm)" />
          </span>
        </Hint>
      ) : null}
    </>
  );
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
