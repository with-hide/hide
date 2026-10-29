import { CheckIcon, WaypointsIcon } from "lucide-react";
import type { Actions } from "./actions";
import { formatElapsed } from "./components/elapsed";
import { ProgressCells, TONE_TEXT } from "./components/mission-tone";
import { StatusMark } from "./components/status-mark";
import { Popover, PopoverAnchor, PopoverContent } from "./components/ui/popover";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { markdownPlainText } from "./markdownPlain";
import { useWorkerRow, ResultTail } from "./missionParts";
import { allAgents } from "./paneAgents";
import { liveNodes, missionCells, missionOfPane, missionOpen, missionResults, missionTally, nodeMark, paneChipWords, stageWord, type PaneChipState } from "./missions";
import { PanePlanChip, usePaneChipPopover } from "./PanePlanChip";
import { machineName } from "./screenMachine";
import type { Mission, MissionNode } from "./snapshot";
import { useShellStore } from "./store";

// A Conductor pane's mission chip and its popover (PRD mission-pane-chip
// D-01, D-07; PRD mission-plan-visibility B8, B10, B11): in the plan chip's
// place, the pointer or the keyboard's focus shows the popover, the nodes a
// worker is at now while the mission runs and what it left once it ends, and
// a press, Enter or Space on the chip opens the mission's map in the lane's
// 보기 slot, the one place the map is drawn.

/** A finished mission's popover lists this many results; `+N` says the rest (B11). */
export const POPOVER_RESULTS = 3;

/** A finished mission's popover quotes this many lines of the Conductor's report (B11). */
const REPORT_LINES = 2;

/** The mission the pane's session conducts, as its sidebar row finds it; none while the record cannot be read. */
export function usePaneMission(paneId: string): Mission | null {
  return useShellStore((s) => missionOfPane(s.rest, allAgents(s.agents, s.rest?.status?.remote), paneId));
}

/** The pane head's work chip: the mission chip on a Conductor's pane, its own plan chip on any other (D-07). */
export function PaneWorkChip({ paneId, actions }: { paneId: string; actions: Actions }) {
  const mission = usePaneMission(paneId);
  return mission ? <PaneMissionChip paneId={paneId} mission={mission} actions={actions} /> : <PanePlanChip paneId={paneId} actions={actions} />;
}

const CHIP_TONE: Record<PaneChipState, string> = {
  planning: "",
  running: "text-foreground",
  // The state keeps its colour while the popover is open, over the open chip's ring (approved board 3).
  decision: "border-mission-hold text-mission-hold data-[state=open]:border-mission-hold data-[state=open]:text-mission-hold",
  merge_wait: "text-foreground",
  done: "text-mission-done data-[state=open]:text-mission-done",
  cancelled: "text-mission-cancelled",
};

function PaneMissionChip({ paneId, mission, actions }: { paneId: string; mission: Mission; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const chip = usePaneChipPopover(paneId, true);
  const words = paneChipWords(mission);
  const [first] = liveNodes(mission);
  const state = [words.state === "decision" ? t("mission.askDecision") : null, t(words.key), words.counts, first ? t("mission.pane.now", { title: first.title }) : null].filter(Boolean).join(" ");
  const label = t("mission.pane.label", { title: mission.title, state });
  const Mark = words.state === "done" ? CheckIcon : WaypointsIcon;
  return (
    <Popover {...chip.popover}>
      <PopoverAnchor asChild>
        <button
          {...chip.anchor}
          // The press opens the map rather than pinning the popover; the Show plan chord still pins it.
          aria-pressed={undefined}
          onClick={() => {
            chip.close();
            actions.openMission(mission.id);
          }}
          aria-label={label}
          data-pane-mission-chip={mission.id}
          data-mission-chip-state={words.state}
          className={cn(
            "flex h-(--size-sidebar-line-detail) shrink-0 items-center gap-xxs rounded-xs border border-border bg-background px-xs text-micro text-muted-foreground outline-none hover:bg-popover hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:border-ring data-[state=open]:text-foreground",
            CHIP_TONE[words.state],
          )}
        >
          <Mark aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
          {words.state === "decision" ? <span aria-hidden="true" className="font-semibold" data-mission-chip-decision="true">!</span> : null}
          <span className="whitespace-nowrap">{t(words.key)}</span>
          {mission.nodes.length > 0 ? <ProgressCells tones={missionCells(mission)} size="chip" /> : null}
          {words.counts ? <span className="font-mono tabular-nums">{words.counts}</span> : null}
        </button>
      </PopoverAnchor>
      <PopoverContent
        side="bottom"
        align="start"
        collisionPadding={8}
        className="max-h-(--radix-popover-content-available-height) w-(--size-pr-popover) overflow-y-auto p-0 text-caption"
        data-pane-mission={mission.id}
        {...chip.content}
      >
        <MissionPopover
          paneId={paneId}
          mission={mission}
          onConversation={() => {
            chip.close();
            actions.openAgent(paneId);
          }}
        />
      </PopoverContent>
    </Popover>
  );
}

/**
 * The popover's body: an open mission's head, its decision, the nodes a
 * worker is at and one tally line (B10); a finished or cancelled mission's
 * head, its report's first lines and what it left (B11). No record footer:
 * the pane ⋯ menu and the sidebar row open the record (B7), and the chip
 * itself opens the map, which the head's end says.
 */
export function MissionPopover({ paneId, mission, onConversation }: { paneId: string; mission: Mission; onConversation: () => void }) {
  const { t } = useInterfaceTranslation();
  const device = useShellStore((s) => (mission.stale ? machineName(s.rest?.navigator?.devices, mission.device_id, t) : null));
  const ended = mission.status === "done" || mission.status === "cancelled";
  return (
    <div className="flex min-w-0 flex-col gap-xs pb-sm [word-break:keep-all]" data-mission-popover={paneId} data-mission-popover-state={ended ? mission.status : "open"}>
      <div className="flex min-w-0 items-center gap-xs px-sm pt-sm">
        <span className="min-w-0 flex-1 truncate">{ended ? <EndedHead mission={mission} /> : <OpenHead mission={mission} />}</span>
        <span className="shrink-0 text-micro text-muted-foreground" data-mission-mini-open="true">{t("mission.mini.open")}</span>
      </div>
      {device ? <p className="px-sm text-muted-foreground" data-mission-stale-note="true">{t("mission.stale", { device })}</p> : null}
      <div className={cn("flex min-w-0 flex-col gap-xs", mission.stale && "opacity-(--opacity-dimmed)")}>
        {ended ? <EndedBody mission={mission} /> : <OpenBody mission={mission} onConversation={onConversation} />}
      </div>
    </div>
  );
}

/** `미션 n/m` while it runs, else the chip's own state word (B10). */
function OpenHead({ mission }: { mission: Mission }) {
  const { t } = useInterfaceTranslation();
  // The head reads the mission's own state; the decision has its own line below it.
  const head = paneChipWords({ ...mission, decision: null });
  return (
    <span className="text-muted-foreground" data-mission-mini-head={head.state}>
      {t(head.key)}
      {head.counts ? <span className="ml-xxs font-semibold tabular-nums text-foreground">{head.counts}</span> : null}
    </span>
  );
}

/** `✓ 끝남 · <걸린 시간>`, or the cancelled mission's grey `취소` (B11). */
function EndedHead({ mission }: { mission: Mission }) {
  const { t, i18n } = useInterfaceTranslation();
  const done = mission.status === "done";
  const took = mission.started_at_unix_ms != null && mission.closed_at_unix_ms != null ? formatElapsed(requireInterfaceLanguage(i18n.language), mission.closed_at_unix_ms - mission.started_at_unix_ms) : null;
  return (
    <span className="flex min-w-0 items-center gap-xs" data-mission-mini-head={mission.status}>
      <span className={cn("flex shrink-0 items-center gap-xxs font-semibold", done ? TONE_TEXT.done : TONE_TEXT.cancelled)}>
        {done ? <CheckIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : null}
        {t(done ? "mission.pill.done" : "mission.pill.cancelled")}
      </span>
      {took ? <span className="truncate text-muted-foreground" data-mission-mini-took="true">· {took}</span> : null}
    </span>
  );
}

function OpenBody({ mission, onConversation }: { mission: Mission; onConversation: () => void }) {
  const { t } = useInterfaceTranslation();
  const live = liveNodes(mission);
  const { done, next } = missionTally(mission);
  const deciding = mission.decision !== null && missionOpen(mission);
  return (
    <>
      {deciding ? (
        <div className="mx-sm flex min-w-0 items-center gap-xs rounded-md border border-mission-hold px-sm py-xs" data-mission-mini-decision="true">
          <span className="min-w-0 flex-1 truncate text-mission-hold" title={mission.decision ?? undefined}>
            <span aria-hidden="true" className="font-semibold">! </span>
            {mission.decision}
          </span>
          <button type="button" className="shrink-0 rounded-xs px-xs py-xxs text-foreground outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" onClick={onConversation} data-mission-mini-conversation="true">
            {t("mission.mini.conversation")}
          </button>
        </div>
      ) : null}
      {mission.nodes.length === 0 ? (
        <p className="px-sm text-muted-foreground" data-mission-mini-planning="true">{t("mission.planning")}</p>
      ) : (
        <>
          {live.length > 0 ? (
            <ul className="flex min-w-0 flex-col px-xs">
              {live.map((node) => <LiveRow key={node.key} node={node} />)}
            </ul>
          ) : null}
          <p className="px-sm text-muted-foreground" data-mission-mini-tally={`${done} ${next}`}>{t("mission.mini.tally", { done, next })}</p>
        </>
      )}
    </>
  );
}

/** A node a worker is at: its mark, its title, its PR number and where its work is (B10). */
function LiveRow({ node }: { node: MissionNode }) {
  const { t } = useInterfaceTranslation();
  const row = useWorkerRow(node);
  return (
    <li className="flex min-w-0 items-center gap-xs rounded-xs px-xs py-xxs" data-mission-mini-node={node.key} data-node-status={node.status}>
      <StatusMark mark={nodeMark(node, row?.mark ?? null)} label={t(`mission.status.${node.status}`)} />
      <span className="min-w-0 flex-1 truncate" title={node.title}>{node.title}</span>
      {node.pr != null ? <span className="shrink-0 font-mono text-micro text-muted-foreground" data-node-pr={node.pr}>#{node.pr}</span> : null}
      <span className="shrink-0 text-micro text-muted-foreground" data-node-stage={node.stage ?? 0}>{t(stageWord(node))}</span>
    </li>
  );
}

function EndedBody({ mission }: { mission: Mission }) {
  const { t } = useInterfaceTranslation();
  const results = missionResults(mission);
  const shown = results.slice(0, POPOVER_RESULTS);
  // The report is the Conductor's Markdown; the popover quotes its words, not its marks.
  const lines = mission.report ? markdownPlainText(mission.report).split("\n").filter(Boolean).slice(0, REPORT_LINES) : [];
  return (
    <>
      {lines.length > 0 ? (
        <div className="mx-sm flex min-w-0 flex-col rounded-md border-l-2 border-mission-done bg-muted px-sm py-xs" data-mission-mini-report="true">
          {lines.map((line, index) => <p key={index} className={cn("min-w-0 break-words", index > 0 && "text-muted-foreground")}>{line}</p>)}
        </div>
      ) : null}
      {shown.length > 0 ? (
        <ul className="flex min-w-0 flex-col gap-xxs px-sm" data-mission-mini-results={results.length}>
          {shown.map(({ node, result }) => (
            <li key={node.key} className="flex min-w-0 items-center gap-xs" data-mission-mini-result={node.key}>
              <ResultTail result={result} stale={mission.github_stale} />
              <span className="min-w-0 flex-1 truncate text-muted-foreground" title={node.title}>{node.title}</span>
            </li>
          ))}
          {results.length > shown.length ? <li className="text-muted-foreground" data-mission-mini-more={results.length - shown.length}>{t("mission.mini.more", { count: results.length - shown.length })}</li> : null}
        </ul>
      ) : null}
    </>
  );
}
