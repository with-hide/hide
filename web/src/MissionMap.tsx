import { CogIcon, KeyRoundIcon, PauseIcon, TriangleAlertIcon } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useState } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { formatElapsed, useElapsed, useMinuteClock, useRemaining } from "./components/elapsed";
import { ConductorRow } from "./components/mission-conductor-row";
import { MapPage, MapSplit } from "./components/map-page";
import { MissionEnd } from "./components/mission-end";
import { GatePlanList, OperatorBand } from "./components/mission-gate";
import { DeviceChip } from "./components/device-chip";
import { BranchLineView, ResultLineView, WaitingMark } from "./components/mission-line";
import { MissionNodeHover } from "./components/mission-node-hover";
import { MissionDestinationTag, MissionOptionsLine } from "./components/mission-options";
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
import { MissionGraph } from "./MissionGraph";
import { MissionGraphCard, MissionGraphReport } from "./components/mission-graph-card";
import { MissionMapCanvas } from "./MissionMapCanvas";
import { MissionNodePanel } from "./MissionNodePanel";
import { MissionTurn } from "./MissionTurn";
import { markWord, MissionNumber, useMissionDeviceName, useNodeRun, useNodeTimes } from "./missionParts";
import { branchLine, branchLineWords, lineNode, resultLine, resultLineWords } from "./missionLine";
import { landsInDestination, missionOptions } from "./missionOptions";
import { DEFAULT_CONDUCTOR } from "./conductorSettings";
import { STATE_PILL } from "./missionList";
import { planCount } from "./plans";
import { humanTurnText } from "./humanTurn";
import { useWorkerEscalation } from "./missionWorkers";
import {
  attentionWords,
  cardGate,
  defectEdges,
  failedVerdict,
  gatesAhead,
  gateWaitedLong,
  openAttention,
  hideWords,
  missionHeadCells,
  missionCounts,
  missionEventsOf,
  missionOf,
  missionOpen,
  nodeHold,
  nodeLook,
  nodeSteps,
  nodeWork,
  nodeWorkWords,
  stopCauseWords,
  reportLine,
  stackWaiting,
  warningWords,
  type NodeLook,
} from "./missions";
import { machineName } from "./screenMachine";
import type { Mission, MissionEvent, MissionNode, MissionNodeHold, MissionNodeWork } from "./snapshot";
import { useShellStore } from "./store";

// A mission's page in the lane's 보기 slot (PRD mission-map-git-criteria
// D-01-D-06, B24-B35; PRD mission-tool B12-B25): the head (state, time, a
// cell per node, `n / m`); a finished mission's results, its pull requests
// and its report's line; the Conductor row; what only the operator can do
// (내 차례, B25); the DAG; and beside it the summary, or the panel of the
// node the operator picked, beside the map when the page is wide and under
// it when narrow (B34). Picking a card keeps it and the cards one edge away
// lit and dims the rest (B30). Every value is the core's (`MissionView`, its
// events); this page words and draws it and edits nothing.


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
              <MissionGraph
                flow={flow}
                mission={mission}
                defects={defectEdges(mission)}
                drawNode={(item, picked, dimmed, pick) => <NodeCard mission={mission} node={item} events={log?.events ?? null} end={end} actions={actions} selected={picked} dimmed={dimmed} onSelect={pick} />}
                report={(dimmed) => <ReportCard mission={mission} dimmed={dimmed} />}
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

/**
 * Where the operator will look, over a plan that waits for approval (PRD
 * mission-human-gate D-08, B1). The rows above it are control-high, so their
 * words carry slack under them that its caption head does not: `pt-xxs`
 * puts its words as far below the Conductor's as those are below the head's.
 */
function GatePlan({ mission, onSelect }: { mission: Mission; onSelect: (key: string) => void }) {
  return <GatePlanList nodes={mission.status === "planning" ? gatesAhead(mission) : []} onSelect={onSelect} className="px-md pt-xxs pb-sm" />;
}

/**
 * The head, one line (B24): the mission's state, its time, a cell per node,
 * `n / m` and its verification strength, which only the Conductor changes;
 * a plan waiting for approval says only how many steps it has. A
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
      {/* A plan waiting for approval has nothing done yet, so it counts its steps as a plan under review does (PlanMap's `검토 · 8단계`). */}
      {mission.nodes.length > 0 && mission.status === "planning" ? (
        <span className="shrink-0 text-caption text-muted-foreground tabular-nums" data-mission-stat="steps">
          {t("plan.steps", { count: total })}
        </span>
      ) : mission.nodes.length > 0 ? (
        <>
          <ProgressCells tones={missionHeadCells(mission)} fill={HEAD_FILL} size="head" className="max-w-(--size-pr-popover)" />
          <span className="shrink-0 text-caption text-muted-foreground tabular-nums" data-mission-stat="done">
            {t("mission.doneOf", { done: String(done), total: String(total) })}
          </span>
        </>
      ) : null}
      {/* Where the results go and what the mission applies (PRD mission-destination D-18, D-50): a tag only while a node lands in the destination, the options in grey. */}
      {mission.destination && landsInDestination(mission.nodes) ? <MissionDestinationTag branch={mission.destination} /> : null}
      {/* The head stays one line (B24): the options take what room is left and truncate, moving to a line of their own only when less than their least width is left. */}
      <MissionOptionsLine options={options} className="min-w-(--size-mission-options-min) flex-1 basis-0" />
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

/**
 * A node on the map (PRD mission-node-simple D-07, B1-B3; PRD
 * mission-destination D-51-D-55, B54-B60), five rows apart by one spacing
 * token: its state's mark and its title on one line that ends in an ellipsis
 * before anything else does, with Hide's mark when Hide changed it last; the
 * band of what the operator does, its gate's `볼 것` and a merge that is
 * theirs now; its worker's agent mark, model and how long it has taken, with
 * its device when away as a tab on the card's top edge, never a cut chip in
 * that row; a bar of one cell per step of how its work goes
 * (done, being worked, or only planned) with the step being worked under it,
 * or `계획 n단계 · 시작 전` before a worker starts; and under a hairline its
 * bottom line: for a node that changes code where its branch goes, for one
 * that does not its result, then `! 나` or `! Conductor` when it waits on
 * someone to look, its reason on hover. A line with nothing to hold is not
 * drawn. Hovering or focusing it shows the rest (`MissionNodeHover`); the
 * other agents checking it stand beside it as satellites tethered to its
 * right side. Pressing it picks it (B30).
 */
function NodeCard({ mission, node, events, end, selected, dimmed, onSelect, actions }: { mission: Mission; node: MissionNode; events: MissionEvent[] | null; end: number; selected: boolean; dimmed: boolean; onSelect: () => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const look = nodeLook(mission, node);
  const attention = openAttention(mission, node) ? attentionWords(node, t) : null;
  // A place the plan has the operator look (PRD mission-human-gate D-06): ahead it is a tinted strip, reached it is the operator's turn, looked at it is grey with a check; a start gate once passed is the panel's alone (PRD node-presets D-09).
  const gate = cardGate(node.gate);
  // A merge the core says is the operator's now is the band's too (PRD mission-destination D-53); one ahead or done is the person on the branch line.
  const mergeDue = node.merge?.state === "due";
  const minute = useMinuteClock();
  const long = gateWaitedLong(gate, minute);
  // Only a card waiting on the operator is the operator's turn: a raised reason to them, an open gate, or a merge the core says is theirs now (PRD mission-destination B56); one waiting on the Conductor, or a gate or merge still ahead, reads as work in progress (B9).
  const yours = (attention !== null && node.attention!.to === "operator") || gate?.state === "open" || mergeDue;
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
  // How long it has been worked, `n째` while it runs and `n 걸림` once it ended (D-51, B58); a planned node has no time.
  const times = useNodeTimes(mission, node, events, end);
  const took = times?.took ? t(times.running ? "mission.elapsed.running" : "mission.elapsed.total", { elapsed: times.took }) : null;
  // How the node's work goes is one bar of cells under its title, from the worker's plan once it exists, else the Conductor's steps; its count is in the card's name, never a second number on the card.
  const work = nodeSteps(node);
  // Where its result goes (D-52), or for a node that changes no code what it found (D-54).
  const branch = useMemo(() => branchLine(lineNode(node, mission.nodes), mission.destination ?? null), [node, mission.nodes, mission.destination]);
  const result = branch ? null : resultLine(lineNode(node));
  const waits = node.kind === "external";
  const failed = failedVerdict(node);
  const titleText = (
    <span className={cn("min-w-0 flex-1 truncate text-body font-semibold", (look === "external" || look === "cancelled" || look === "waiting") && "text-muted-foreground", look === "dropped" && "text-muted-foreground line-through")} data-node-title="true">
      {node.title}
    </span>
  );
  // The foot's right-end marks, one group: a warning, then the operator's turn or who a raised reason waits on. Short of
  // room the warning's words give way, down to its icon, then a long wait's words; the tag's own word never does, and the
  // group clips rather than cover the line's number. The card's name says each mark in full.
  const marks = warning || gate?.state === "open" || attention ? (
    <span className="flex min-w-0 items-center justify-end gap-x-xs overflow-hidden" data-node-foot-end="true">
      {warning ? <NodeWarning words={warning} kinds={(node.warnings ?? []).map((item) => item.kind).join(" ")} /> : null}
      {gate?.state === "open" && !attention ? (
        <span className="flex items-center gap-x-xxs font-semibold text-mission-hold" data-node-gate-tag="open">
          <span className="shrink-0">{t("agents.mark.turn")}</span>
          {long ? <span className="min-w-0 shrink-[10] truncate font-normal text-muted-foreground" data-node-gate-long="true"> · {t("mission.gate.long")}</span> : null}
        </span>
      ) : null}
      {attention ? (
        <Hint label={attention.why}>
          <span className={cn("flex items-center gap-x-xxs", yours ? "font-semibold text-mission-hold" : "text-muted-foreground")} data-node-attention-tag={node.attention!.reason}>
            <span className="shrink-0">{yours ? t("agents.mark.turn") : attention.who}</span>
            {long ? <span className="min-w-0 shrink-[10] truncate font-normal text-muted-foreground" data-node-gate-long="true"> · {t("mission.gate.long")}</span> : null}
          </span>
        </Hint>
      ) : null}
    </span>
  ) : null;
  const card = (
    <MissionGraphCard
      node={node} look={look} word={word} yours={yours} selected={selected} dimmed={dimmed} onClick={onSelect} failed={failed}
      still={doing != null && doing.state !== "working"}
      title={titleText}
      accessory={hide ? <HideMark words={hide} /> : null}
      tab={device ? <DeviceChip label={device} variant="remote" className="max-w-[calc(var(--size-mission-node)*2/3)] rounded-none rounded-tr-sm rounded-bl-sm" /> : null}
      data-node-attention={attention ? node.attention!.to : undefined}
      aria-label={[
        node.title,
        word,
        ...(node.kind === "external" && look !== "external" ? [t("mission.look.external")] : []),
        ...(run ? [run.text] : []),
        ...(device ? [device] : []),
        ...(took ? [took] : []),
        ...(work ? [node.plan ? t("plan.chip", { progress: planCount(node.plan) }) : t("mission.steps.planned", { count: work.steps.length })] : []),
        ...node.prs.map((pr) => t("mission.attention.pr", { number: String(pr.number) })),
        ...(branch ? [`${t("mission.foot.where")}: ${branchLineWords(branch, t)}`] : []),
        ...(result ? [resultLineWords(result, t)] : []),
        ...(attention ? [`${t("mission.attention")} · ${attention.who} · ${attention.why}`] : []),
        ...(gate ? [`${t("mission.gate.title")} · ${gate.look}${gate.state === "passed" ? ` · ${t("mission.gate.passed")}` : gate.state === "open" ? ` · ${t("agents.mark.turn")}` : ""}${long ? ` · ${t("mission.gate.long")}` : ""}`] : []),
        ...(humanLine ? [`${t("agents.human.mark")} · ${humanLine}`] : []),
        ...(stopLine ? [stopLine.full] : []),
        ...(holdLine ? [holdLine.full] : []),
        ...(warning ? warning.full : []),
        ...(hide ? [t("mission.hide.mark", { what: hide })] : []),
      ].join(" · ")}
    >
      {device ? <span aria-hidden="true" className="absolute inset-y-xs left-0 w-xs rounded-r-xs bg-device-remote" data-node-remote="true" /> : null}
      {/* What the operator does on this node, right under its title (D-51): the gate's strip and a merge that is theirs now, read without a hover. */}
      <OperatorBand gate={gate} mergeDue={mergeDue} />
      {/* Its worker: the agent's mark, the model and how long; effort, profile and the device's full name are the hover's (D-51), the device away the card's tab. A waiting node has no worker and says `⏳ 기다림` in its place (PRD node-presets D-13). */}
      {waits || worker?.agent || worker?.model || took ? (
        <span className="flex min-w-0 items-center gap-xxs text-caption text-muted-foreground" data-node-run={waits ? "waiting" : (run?.from ?? "none")}>
          {waits ? <WaitingMark /> : null}
          {worker?.agent ? <span className="flex shrink-0" aria-hidden="true" data-node-worker-mark="true"><AgentMark kind={worker.agent} profile={run?.profile} /></span> : null}
          {worker?.model ? <span className="min-w-0 truncate text-subtle-foreground" data-node-model={run?.from}>{worker.model}</span> : null}
          {run?.mark ? <span className="shrink-0" data-run-planned="true">{run.mark}</span> : null}
          {took ? <span className="shrink-0 whitespace-nowrap" data-node-took={times?.running ? "running" : "total"}>{worker?.model || worker?.agent || waits ? "· " : ""}{took}</span> : null}
        </span>
      ) : null}
      {doingLine ? (
        <span className={cn("min-w-0 truncate text-caption", doing?.state === "no_worker" ? "text-warning" : "text-muted-foreground")} data-node-work={doing?.state}>
          {doingLine}
        </span>
      ) : null}
      {/* A worker stopped with nothing to wait on says why as a held node does, the cause cut when it runs long and how long ago whole at the line's end; its Conductor, not the operator, is told (operator decision 2026-11-11). */}
      {stopLine ? (
        <span className="flex min-w-0 items-center gap-xxs text-caption text-muted-foreground" title={stopLine.full} data-node-work="stopped">
          <span className="min-w-0 truncate">{stopLine.why}</span>
          {stopLine.age ? <span className="shrink-0 whitespace-nowrap" data-node-stopped-age="true">· {stopLine.age}</span> : null}
        </span>
      ) : null}
      {/* A node its Conductor holds back says why in grey, cut when it runs long, and keeps how long it still holds whole at the line's end (operator decision 2026-11-11). */}
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
      {/* The bottom line, under a hairline with the rows' one spacing above its words (D-51): the branch line for a change or a waiting node, the result line for a general node (D-52, D-54; PRD node-presets D-08). `⚠`, 내 차례 and `!` stand at the right end of its first row and take only the width the line leaves there, their words cut before the line's number or icon ever is (D-56). With neither line, the marks alone stand at the right end; with nothing at all, there is no foot. */}
      {branch || result || marks ? (
        <span className={cn("min-w-0 border-t border-border pt-xs text-caption text-muted-foreground", branch ? "block" : "flex items-center justify-end gap-x-xs")} data-node-foot="true">
          {branch ? <BranchLineView line={branch} end={marks} /> : (
            <>
              {result ? <ResultLineView line={result} className="flex-1" /> : null}
              {marks}
            </>
          )}
        </span>
      ) : null}
    </MissionGraphCard>
  );
  const hovered = (
    <MissionNodeHover mission={mission} node={node} facts={{ look, word, still: doing != null && doing.state !== "working", failed, doing: doingLine, took, worker: run ? { agent: worker?.agent ?? null, run, device } : null, work, branch, result, gate, mergeDue }}>
      {card}
    </MissionNodeHover>
  );
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
      <span aria-hidden="true" className="flex min-w-(--size-icon-sm) shrink-[100] items-center gap-xxs overflow-hidden font-semibold text-warning" data-node-warning={kinds}>
        <TriangleAlertIcon className="size-(--size-icon-sm) shrink-0" />
        <span className="min-w-0 truncate">{words.short}</span>
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

function ReportCard({ mission, dimmed }: { mission: Mission; dimmed: boolean }) {
  const { t } = useInterfaceTranslation();
  const stack = stackWaiting(mission);
  const note = mission.status === "done" ? t("mission.report.done") : mission.status === "merge_wait" && stack > 0 ? t("mission.report.stack") : mission.status === "cancelled" ? t("mission.cancelled") : t("mission.report.waiting");
  const look: NodeLook = mission.status === "done" ? "done" : mission.status === "cancelled" ? "cancelled" : "waiting";
  return <MissionGraphReport status={mission.status} note={note} look={look} dimmed={dimmed} />;
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
