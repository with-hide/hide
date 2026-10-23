import { ChevronRightIcon, WaypointsIcon } from "lucide-react";
import { useMemo } from "react";
import type { Actions } from "./actions";
import { DependencyGraphView, type EdgeLook } from "./DependencyGraph";
import type { GraphEdge } from "./dependencyLayout";
import { StatusMark } from "./components/status-mark";
import { Kbd } from "./components/ui/kbd";
import { Popover, PopoverAnchor, PopoverContent } from "./components/ui/popover";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { useWorkerRow } from "./MissionMap";
import { allAgents } from "./paneAgents";
import { MINI_REPORT, missionMiniGraph, missionOfPane, missionOpen, nodeMark, paneChipWords, type MiniCard } from "./missions";
import { PanePlanChip, usePaneChipPopover } from "./PanePlanChip";
import { machineName } from "./screenMachine";
import type { Mission, MissionNode } from "./snapshot";
import { commandLabel } from "./shortcutLabels";
import { useShellStore } from "./store";

// A Conductor pane's mission chip and its small map (PRD mission-pane-chip
// D-01, D-04, D-05, D-07-D-10, D-16): in the plan chip's place, the same
// pointer, pin and Esc rules, the nodes still to do top down into the
// report, and `크게 ›` for the big map in the lane's 보기 slot.

/** A done node draws one dot up to this many; the count says the rest. */
const DONE_DOTS = 10;

/** The mission the pane's session conducts, as its sidebar row finds it; none while the record cannot be read. */
export function usePaneMission(paneId: string): Mission | null {
  return useShellStore((s) => missionOfPane(s.rest, allAgents(s.agents, s.rest?.status?.remote), paneId));
}

/** The pane head's work chip: the mission chip on a Conductor's pane, its own plan chip on any other (D-07). */
export function PaneWorkChip({ paneId, actions }: { paneId: string; actions: Actions }) {
  const mission = usePaneMission(paneId);
  return mission ? <PaneMissionChip paneId={paneId} mission={mission} actions={actions} /> : <PanePlanChip paneId={paneId} actions={actions} />;
}

const CHIP_TONE: Record<ReturnType<typeof paneChipWords>["state"], string> = {
  planning: "",
  running: "text-foreground",
  // The state keeps its colour while the map is open, over the open chip's ring (approved board 3).
  decision: "border-warning text-warning data-[state=open]:border-warning data-[state=open]:text-warning",
  merge_wait: "text-foreground",
  done: "border-success text-success data-[state=open]:border-success data-[state=open]:text-success",
  cancelled: "",
};

function PaneMissionChip({ paneId, mission, actions }: { paneId: string; mission: Mission; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const chip = usePaneChipPopover(paneId, true);
  const words = paneChipWords(mission);
  const state = [t(words.key), words.counts].filter(Boolean).join(" ");
  const label = t("mission.pane.label", { title: mission.title, state });
  return (
    <Popover {...chip.popover}>
      <PopoverAnchor asChild>
        <button
          {...chip.anchor}
          aria-label={label}
          data-pane-mission-chip={mission.id}
          data-mission-chip-state={words.state}
          className={cn(
            "flex h-(--size-sidebar-line-detail) shrink-0 items-center gap-xxs rounded-xs border border-border bg-background px-xs text-micro text-muted-foreground outline-none hover:bg-popover hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:border-ring data-[state=open]:text-foreground",
            CHIP_TONE[words.state],
          )}
        >
          <WaypointsIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
          <span className="whitespace-nowrap">{t(words.key)}</span>
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
        <MiniMap
          paneId={paneId}
          mission={mission}
          onLarge={() => {
            chip.close();
            actions.openMission(mission.id);
          }}
          onConversation={() => {
            chip.close();
            actions.openAgent(paneId);
          }}
          onRecord={
            mission.conductor.session
              ? () => {
                  chip.close();
                  if (mission.conductor.session) actions.openSessionRecord(mission.conductor.session);
                }
              : null
          }
        />
      </PopoverContent>
    </Popover>
  );
}

/** The popover's body: its head, the decision, the done line, the small map and the record (B5-B10). */
export function MiniMap({ paneId, mission, onLarge, onConversation, onRecord }: { paneId: string; mission: Mission; onLarge: () => void; onConversation: () => void; onRecord: (() => void) | null }) {
  const { t } = useInterfaceTranslation();
  const device = useShellStore((s) => (mission.stale ? machineName(s.rest?.navigator?.devices, mission.device_id, t) : null));
  const { done, graph, stacked } = useMemo(() => missionMiniGraph(mission), [mission]);
  // The head reads the mission's own state; the decision has its own words beside it.
  const head = paneChipWords({ ...mission, decision: null });
  const deciding = mission.decision !== null && missionOpen(mission);
  return (
    <div className="flex min-w-0 flex-col [word-break:keep-all]" data-mission-mini={paneId}>
      <div className="flex min-w-0 items-center gap-xs px-sm pt-sm">
        <span className="min-w-0 flex-1 truncate text-muted-foreground" data-mission-mini-head={head.state}>
          {t(head.key)}
          {head.counts ? <span className="ml-xxs font-semibold tabular-nums text-foreground">{head.counts}</span> : null}
          {deciding ? <span className="text-warning"> · {t("mission.pane.decision")}</span> : null}
        </span>
        <button
          type="button"
          className="flex shrink-0 items-center gap-xxs rounded-xs px-xs py-xxs text-primary outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring"
          onClick={onLarge}
          data-mission-mini-large="true"
        >
          {t("plan.large")}
          <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />
        </button>
      </div>
      {deciding ? (
        <div className="mx-sm mt-xs flex min-w-0 items-center gap-xs rounded-md border border-warning px-sm py-xs" data-mission-mini-decision="true">
          <span className="min-w-0 flex-1 truncate text-warning" title={mission.decision ?? undefined}>
            <span aria-hidden="true" className="font-semibold">! </span>
            {mission.decision}
          </span>
          <button type="button" className="shrink-0 rounded-xs px-xs py-xxs text-foreground outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" onClick={onConversation} data-mission-mini-conversation="true">
            {t("mission.mini.conversation")}
          </button>
        </div>
      ) : null}
      {device ? <p className="px-sm pt-xs text-muted-foreground" data-mission-stale-note="true">{t("mission.stale", { device })}</p> : null}
      {done > 0 ? (
        <button type="button" className="mx-sm mt-xs flex w-fit items-center gap-xs rounded-xs px-xxs py-xxs text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring" onClick={onLarge} data-mission-mini-done={done}>
          <span aria-hidden="true" className="flex items-center gap-xxs">
            {Array.from({ length: Math.min(done, DONE_DOTS) }, (_, index) => (
              <span key={index} className="size-(--size-status-mark) rounded-full bg-success" />
            ))}
          </span>
          <span>{t("mission.mini.done", { count: done })}</span>
          <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />
        </button>
      ) : null}
      <div className={cn("overflow-x-auto px-sm py-sm", mission.stale && "opacity-(--opacity-dimmed)")}>
        {mission.nodes.length === 0 ? (
          <p className="text-muted-foreground" data-mission-mini-planning="true">{t("mission.planning")}</p>
        ) : graph.layers.length === 0 ? (
          <ReportFace mission={mission} />
        ) : (
          <DependencyGraphView
            graph={graph}
            idOf={(card) => card.key}
            flow="down"
            gaps={{ layers: "--spacing-lg", nodes: "--spacing-sm" }}
            edgeLook={(edge: GraphEdge): EdgeLook => edgeLookOf(edge, mission, stacked)}
            draw={(card) => <MiniFace key={card.key} mission={mission} card={card} />}
          />
        )}
      </div>
      {onRecord ? (
        <div className="border-t border-border p-xxs">
          <button
            type="button"
            className="flex w-full items-center justify-between gap-sm rounded-xs px-xs py-xs text-left outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring"
            onClick={onRecord}
            data-mission-mini-record="true"
          >
            <span>{t("mission.mini.record")}</span>
            <Kbd>{commandLabel("toggle_plan")}</Kbd>
          </button>
        </div>
      ) : null}
    </div>
  );
}

function edgeLookOf(edge: GraphEdge, mission: Mission, stacked: ReadonlySet<string>): EdgeLook {
  // A stack's edge: the pull request below is the one above's base (B7).
  if (stacked.has(`${edge.from}>${edge.to}`)) return { tone: "text-pr-merged", dashed: true };
  const from = mission.nodes.find((node) => node.key === edge.from);
  if (from && (from.status === "working" || from.status === "review")) return { tone: "text-agent-working" };
  // A line still ahead stays bright on the popover, as the plan's small map draws it.
  return { tone: "text-muted-foreground" };
}

const FACE = "flex w-(--size-plan-node-compact) min-w-0 items-center gap-xs rounded-md border border-border bg-card px-xs py-xxs text-left";
/** A card still waiting: dashed in the colour the plan's waiting steps use, which reads on the popover. */
const PENDING = "border-dashed border-muted-foreground bg-transparent";
const pending = (node: MissionNode) => node.status === "planned" || node.blocked !== null;

function MiniFace({ mission, card }: { mission: Mission; card: MiniCard }) {
  const { t } = useInterfaceTranslation();
  if (card.kind === "node") return <NodeFace node={card.node} />;
  if (card.kind === "report") return <ReportFace mission={mission} />;
  const titles = card.nodes.map((node) => node.title).join(", ");
  return (
    <span data-dependency-node={card.key} data-mission-mini-more={card.nodes.length} className={cn(FACE, card.nodes.every(pending) && PENDING)} title={titles}>
      <span className="min-w-0 flex-1 truncate">{t("mission.mini.more", { count: card.nodes.length })}</span>
      {/* The folded titles, read out where the pointer's title cannot reach. */}
      <span className="sr-only">: {titles}</span>
    </span>
  );
}

/** One node in one line: its status mark, its title and its pull request (D-16). */
function NodeFace({ node }: { node: MissionNode }) {
  const { t } = useInterfaceTranslation();
  const row = useWorkerRow(node);
  const live = node.status === "working" || node.status === "review";
  return (
    <span
      data-dependency-node={node.key}
      data-mission-mini-node={node.key}
      data-node-status={node.status}
      className={cn(FACE, pending(node) && PENDING, live && "border-agent-working", node.status === "failed" && "border-destructive")}
    >
      <StatusMark mark={nodeMark(node, row?.mark ?? null)} label={t(`mission.status.${node.status}`)} />
      <span className="min-w-0 flex-1 truncate" title={node.title}>{node.title}</span>
      {node.pr != null ? <span className="shrink-0 font-mono text-micro text-muted-foreground" data-node-pr={node.pr}>#{node.pr}</span> : null}
    </span>
  );
}

function ReportFace({ mission }: { mission: Mission }) {
  const { t } = useInterfaceTranslation();
  const done = mission.status === "done" || mission.status === "merge_wait";
  return (
    <span data-dependency-node={MINI_REPORT} data-mission-mini-report={mission.status} className={cn(FACE, "justify-center text-muted-foreground", !done && PENDING)}>
      <span className="truncate">{t("mission.report")}</span>
    </span>
  );
}
