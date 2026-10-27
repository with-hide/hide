import { CheckIcon, GitBranchIcon, PencilIcon } from "lucide-react";
import { useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { planNowLine } from "./plans";
import type { Actions } from "./actions";
import { DependencyGraphView, type EdgeLook, type OverlayEdge } from "./DependencyGraph";
import type { GraphEdge } from "./dependencyLayout";
import { formatElapsed, useElapsed, useMinuteClock } from "./components/elapsed";
import { MissionNodeMark } from "./components/mission-node-mark";
import { ProgressCells } from "./components/mission-tone";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { MissionNodePanel } from "./MissionNodePanel";
import { MissionTurn } from "./MissionTurn";
import { CheckerRow, lookWord, useNodeTimes } from "./missionParts";
import {
  cardLine,
  checkerTally,
  criteriaTally,
  defectEdges,
  heldQuestion,
  landingLeft,
  missionCells,
  missionCounts,
  missionEventsOf,
  missionOf,
  missionOpen,
  neighbours,
  nodeLook,
  nodeResult,
  ownCriteria,
  shortCommit,
  stackWaiting,
  type NodeLook,
} from "./missions";
import { layerDependencies } from "./projectBoard";
import { machineName } from "./screenMachine";
import type { Mission, MissionCriterion, MissionEvent, MissionNode, MissionStatus } from "./snapshot";
import { useShellStore } from "./store";
import type { MessageKey } from "./i18n/catalogs";
import type { TFunction } from "i18next";

// A mission's page in the lane's 보기 slot (PRD mission-map-git-criteria
// D-01-D-06, B24-B35): the head (state, time, a cell per node, `n / m`),
// what only the operator can do (내 차례, B25), the DAG, and the panel of the
// node the operator picked, beside the map when the page is wide and under
// it when narrow (B34). Picking a card keeps it and the cards one edge away
// lit and dims the rest (B30). Every value is the core's (`MissionView`, its
// events); this page words and draws it and edits nothing.

/** The card that stands for the report every node ends in; a node key is letters, digits, `-`, `_` and `.`, so none is this. */
const REPORT = "#report";

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
  const [picked, setPicked] = useState<string | null>(null);
  const panel = useRef<HTMLDivElement>(null);
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
    <div className="@container/mission flex min-h-0 min-w-0 flex-1 flex-col" data-mission={mission.id} data-mission-status={mission.status} data-mission-stale={mission.stale || undefined} onKeyDown={onKeyDown}>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-auto mission-wide:overflow-hidden" data-mission-scroll="true">
        <Head mission={mission} />
        {mission.report && !open ? <ReportLine mission={mission} /> : null}
        <MissionTurn mission={mission} device={device} actions={actions} onSelect={select} />
        {mission.delay && open ? <p className="px-md pb-xs text-caption text-muted-foreground" data-mission-delay="true">{t("mission.delay", { line: mission.delay })}</p> : null}
        {device ? <p className="px-md pb-xs text-caption text-muted-foreground" data-mission-stale-note="true">{t("mission.stale", { device })}</p> : null}
        {mission.nodes.length === 0 ? (
          <p className="px-md py-sm text-body text-muted-foreground" data-mission-planning="true">{t("mission.planning")}</p>
        ) : (
          <div className={cn("flex min-w-0 flex-col border-t border-border mission-wide:min-h-0 mission-wide:flex-1 mission-wide:flex-row", mission.stale && "opacity-(--opacity-dimmed)")} data-mission-values={mission.stale ? "stale" : "live"}>
            <div
              className="min-w-0 px-md pb-md pt-sm mission-wide:min-h-0 mission-wide:flex-1 mission-wide:overflow-auto"
              onClick={(event: MouseEvent<HTMLDivElement>) => {
                // A press on the map outside every card lets go of the pick (B30).
                if (!(event.target as Element).closest("[data-mission-node]")) setPicked(null);
              }}
              data-mission-map="true"
            >
              <FitWidth>
                <Graph mission={mission} selected={selected} onSelect={(key) => select(selected === key ? null : key)} events={log?.events ?? null} end={end} />
              </FitWidth>
            </div>
            <div
              ref={panel}
              className="flex min-w-0 flex-col border-t border-border mission-wide:min-h-0 mission-wide:w-(--size-mission-side) mission-wide:shrink-0 mission-wide:overflow-auto mission-wide:border-l mission-wide:border-t-0"
              data-mission-panel={selected ?? "none"}
            >
              {node ? (
                <MissionNodePanel mission={mission} node={node} events={log?.events ?? null} end={end} onSelect={select} onClose={() => setPicked(null)} actions={actions} />
              ) : (
                <p className="px-md py-sm text-caption text-muted-foreground" data-mission-panel-hint="true">{t("mission.panel.hint")}</p>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

/**
 * The smallest the map is drawn: below it a card's caption stops being
 * readable, so a slot narrower than that scrolls the map sideways instead.
 */
const MIN_SCALE = 0.6;

/**
 * The map shrunk to the width it has, never grown and never below
 * `MIN_SCALE`: a wide row of nodes stays one row, so what waits on what is
 * read at a glance. The box holds the shrunk size, so the map scrolls down,
 * and sideways only in a slot too narrow for that floor.
 */
function FitWidth({ children }: { children: ReactNode }) {
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
      const scale = width > outer.clientWidth ? Math.max(MIN_SCALE, outer.clientWidth / width) : 1;
      setDrawn((current) => (current?.scale === scale && current.width === width && current.height === height ? current : { scale, width, height }));
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(outer);
    observer.observe(inner);
    return () => observer.disconnect();
  }, []);
  const scale = drawn?.scale ?? 1;
  return (
    <div ref={room} className="min-w-0 overflow-x-auto" data-mission-scale={scale.toFixed(2)}>
      <div className="relative" style={drawn ? { width: drawn.width * scale, height: drawn.height * scale } : undefined}>
        <div ref={content} className={cn("w-max origin-top-left", drawn && "absolute left-0 top-0")} style={scale < 1 ? { transform: `scale(${scale})` } : undefined}>
          {children}
        </div>
      </div>
    </div>
  );
}

/** The pill's words and colour: the operator's merge waits in the decision colour, a finished mission is grey (D-02). */
const PILL: Record<MissionStatus, { key: MessageKey; tone: string }> = {
  planning: { key: "mission.pill.planning", tone: "bg-muted text-muted-foreground" },
  running: { key: "mission.pill.running", tone: "bg-mission-working/15 text-mission-working" },
  merge_wait: { key: "mission.pill.mergeWait", tone: "bg-mission-hold/15 text-mission-hold" },
  done: { key: "mission.pill.done", tone: "bg-muted text-muted-foreground" },
  cancelled: { key: "mission.pill.cancelled", tone: "bg-muted text-mission-cancelled" },
};

/**
 * The head, one line (B24): the mission's state, its time, a cell per node and
 * `n / m`. A device that does not answer stops the running time, so no number
 * moves without the record (B35).
 */
function Head({ mission }: { mission: Mission }) {
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
          <ProgressCells tones={missionCells(mission)} size="head" className="max-w-(--size-pr-popover)" />
          <span className="shrink-0 text-caption text-muted-foreground tabular-nums" data-mission-stat="done">
            {t("mission.doneOf", { done: String(done), total: String(total) })}
          </span>
        </>
      ) : null}
    </div>
  );
}

/** A finished or cancelled mission's report, one line under the head that opens to the whole of it (B24). */
function ReportLine({ mission }: { mission: Mission }) {
  const { t } = useInterfaceTranslation();
  const [unfolded, setUnfolded] = useState(false);
  const report = mission.report ?? "";
  const first = report.split("\n")[0] ?? "";
  return (
    <div className="flex min-w-0 items-start gap-xs px-md pb-xs text-body" data-mission-report-text={unfolded ? "open" : "folded"}>
      <span aria-hidden="true" className={cn("mt-xxs h-(--size-icon-sm) w-px shrink-0", mission.status === "done" ? "bg-border" : "bg-mission-cancelled")} />
      <p className={cn("min-w-0 flex-1", unfolded ? "whitespace-pre-wrap break-words" : "truncate")} title={unfolded ? undefined : report}>{unfolded ? report : first}</p>
      <button type="button" aria-expanded={unfolded} onClick={() => setUnfolded(!unfolded)} className="shrink-0 rounded-xs text-caption text-primary outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring" data-mission-report-toggle="true">
        {t(unfolded ? "mission.report.collapse" : "mission.report.expand")}
      </button>
    </div>
  );
}

type GraphCard = { key: string; node: MissionNode | null };

function Graph({ mission, selected, onSelect, events, end }: { mission: Mission; selected: string | null; onSelect: (key: string) => void; events: MissionEvent[] | null; end: number }) {
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
          <NodeCard key={card.key} mission={mission} node={card.node} selected={selected === card.key} dimmed={lit !== null && !lit.has(card.key)} onSelect={() => onSelect(card.key)} events={events} end={end} />
        ) : (
          <ReportCard key={card.key} mission={mission} dimmed={lit !== null && !lit.has(REPORT)} />
        )
      }
    />
  );
}

/** A card's border and surface by its look (B26): colour only for work, review, a decision and a failure; finished work grey, in main with no surface. */
const LOOK_CARD: Record<NodeLook, string> = {
  waiting: "border-dashed border-mission-waiting bg-transparent",
  working: "border-mission-working ring-2 ring-mission-working/20",
  review: "border-mission-review ring-2 ring-mission-review/20",
  hold: "border-mission-hold ring-2 ring-mission-hold/20",
  failed: "border-mission-failed",
  done: "border-border",
  landed: "border-border bg-transparent",
  external: "border-dotted border-mission-waiting bg-transparent",
  cancelled: "border-dashed border-mission-cancelled bg-transparent",
  dropped: "border-dashed border-border bg-transparent",
};

/**
 * A node on the map (B26-B29): its state's mark and border, its title on up
 * to two lines, where it is or what it waits on with its result, how long it
 * took, `머지 남음` or `합치기 남음`, `기준 n/m` with `⤳k`, and, glued under
 * it, the strip of the other agents checking it. Pressing it picks it (B30).
 */
function NodeCard({ mission, node, selected, dimmed, onSelect, events, end }: { mission: Mission; node: MissionNode; selected: boolean; dimmed: boolean; onSelect: () => void; events: MissionEvent[] | null; end: number }) {
  const { t } = useInterfaceTranslation();
  const look = nodeLook(mission, node);
  const moving = look === "working" || look === "review";
  // The worker's own plan says where it is, else the Conductor's `--line` (PRD hide-plan B16, D-18).
  const line = cardLine(mission, node, moving && node.plan ? planNowLine(node.plan, t) : null, t);
  const result = cardResult(node, t);
  const tally = criteriaTally(mission, node);
  const landing = landingLeft(mission, node);
  const times = useNodeTimes(mission, node, events, end);
  const time = look === "hold" ? (times?.held ? t("mission.time.held", { elapsed: times.held }) : null) : times?.took ?? null;
  // A mission from before criteria shows its sentence where the count would be (B23).
  const before = mission.exempt ? ownCriteria(node) : null;
  const checks = node.criteria.filter((criterion) => (criterion.kind === "review" || criterion.kind === "verify") && (criterion.checkers?.length ?? 0) > 0);
  const word = lookWord(look, t);
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-label={[node.title, word, ...badgeWords(mission, node, t), ...(tally ? [criteriaWords(tally, t)] : [])].join(" · ")}
      aria-pressed={selected}
      data-dependency-node={node.key}
      data-mission-node={node.key}
      data-node-status={node.status}
      data-node-look={look}
      data-node-blocked={node.blocked ?? undefined}
      data-node-dimmed={dimmed || undefined}
      className={cn(
        "flex w-(--size-mission-node) min-w-0 flex-col gap-xxs rounded-md border bg-card px-sm pt-xs text-left outline-none transition-opacity hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring",
        checks.length > 0 ? "pb-0" : "pb-xs",
        LOOK_CARD[look],
        selected && "outline-2 outline-offset-2 outline-primary",
        dimmed && "opacity-(--opacity-dimmed)",
      )}
    >
      <span className="flex min-w-0 items-start gap-xs">
        <MissionNodeMark look={look} label={word} className="mt-xxs" />
        <span className={cn("line-clamp-2 min-w-0 flex-1 break-words text-body font-semibold", (look === "done" || look === "landed" || look === "external" || look === "cancelled" || look === "waiting") && "text-muted-foreground", look === "dropped" && "text-muted-foreground line-through")} title={node.title}>
          {node.title}
        </span>
        <Badges mission={mission} node={node} />
      </span>
      {line || result ? (
        <span className="flex min-w-0 items-center gap-xs text-caption">
          {line ? <span className={cn("min-w-0 flex-1 truncate", line.attention ? "text-mission-failed" : "text-muted-foreground")} title={line.text} data-node-line={line.attention ? "attention" : "true"}>{line.text}</span> : <span className="flex-1" />}
          {result}
        </span>
      ) : null}
      {landing || time || tally || before ? (
        <span className="flex min-w-0 items-center gap-xs text-caption text-muted-foreground">
          {landing ? <span className="shrink-0 rounded-xs bg-mission-hold/15 px-xs text-micro font-semibold text-mission-hold" data-node-landing={landing}>{t(landing === "merge" ? "mission.landing.merge" : "mission.landing.join")}</span> : null}
          {time ? <span className="shrink-0 tabular-nums" data-node-time={look === "hold" ? "held" : "took"}>{time}</span> : null}
          <span className="flex-1" />
          {tally ? (
            <span className="shrink-0 tabular-nums" data-node-criteria={`${tally.met}/${tally.total}`}>
              {t("mission.criteria.count", { met: String(tally.met), total: String(tally.total) })}
              {tally.skipped > 0 ? <span className="ml-xxs text-mission-hold" data-node-skipped={tally.skipped}>⤳{tally.skipped}</span> : null}
            </span>
          ) : before ? (
            <span className="min-w-0 truncate" title={before} data-node-old-criteria="true">{before}</span>
          ) : null}
        </span>
      ) : null}
      {checks.length > 0 ? <CheckerStrip lines={checks} /> : null}
    </button>
  );
}

/** What a card names as its result (B28): its pull request's number, its branch, or its commit on main, whichever the record holds. */
function cardResult(node: MissionNode, t: TFunction<"translation">): ReactNode {
  const result = nodeResult(node);
  if (node.kind === "external" && node.external) {
    return "pr" in node.external
      ? <span className="shrink-0 font-mono text-caption text-muted-foreground" data-node-pr={node.external.pr}>#{node.external.pr}</span>
      : <span className="min-w-0 truncate font-mono text-caption text-muted-foreground" title={node.external.branch} data-node-branch={node.external.branch}>{node.external.branch}</span>;
  }
  if (!result) return null;
  switch (result.kind) {
    // A card's number is plain: the PR state's own colours belong to the panel, so the card's colour says only its state (D-02).
    case "pr":
      return <span className="shrink-0 font-mono text-caption text-muted-foreground" data-node-pr={result.number}>#{result.number}</span>;
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
function criteriaWords(tally: { met: number; total: number; skipped: number }, t: TFunction<"translation">): string {
  const count = t("mission.criteria.count", { met: String(tally.met), total: String(tally.total) });
  return tally.skipped > 0 ? `${count} · ${t("mission.criteria.skipped", { count: tally.skipped })}` : count;
}

/** What a card's badges say, in its accessible name: a badge inside the card's button cannot take focus of its own, so the button speaks for it. */
function badgeWords(mission: Mission, node: MissionNode, t: TFunction): string[] {
  return [
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

/**
 * The other agents checking a node, glued under its card (B29): each with its
 * face, its model and effort, and where its check is, and the rule when
 * several decide. A node's own checks are not here; they are not lines.
 */
function CheckerStrip({ lines }: { lines: MissionCriterion[] }) {
  const { t } = useInterfaceTranslation();
  return (
    <span role="group" aria-label={t("mission.checkers")} className="-mx-sm mt-xxs flex min-w-0 flex-col gap-xxs rounded-b-md border-t border-border bg-muted/40 px-sm py-xxs text-caption" data-node-checkers={lines.map((line) => line.kind).join(" ")}>
      {lines.map((line) => (
        <span key={line.key} className="flex min-w-0 flex-col gap-xxs" data-node-check={line.kind}>
          {(line.checkers ?? []).map((checker, index) => <CheckerRow key={index} kind={line.kind as "review" | "verify"} checker={checker} />)}
          {(line.checkers?.length ?? 0) > 1 ? (
            <span className="pl-lg text-muted-foreground" data-node-check-rule={line.rule ?? "all"}>
              {t(line.rule === "majority" ? "mission.rule.majority" : "mission.rule.all", { count: line.checkers!.length })}
              {checkerTally(line) && line.state === "open" ? ` · ${t("mission.line.passedOf", { passed: String(checkerTally(line)!.passed), total: String(checkerTally(line)!.total) })}` : ""}
            </span>
          ) : null}
        </span>
      ))}
    </span>
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
