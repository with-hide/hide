// What a mission node's card says on hover or focus on the desktop (PRD
// mission-destination D-19, D-20, D-58, B32, B33): the card's own title row
// and the operator's band, then one row per fact under a grey label, each
// drawn with the card's parts at the card's text size: its state and time,
// its worker (agent, model, effort, a device away), its steps, where its
// result goes as the whole branch line with the pull request's state, or a
// node that changes no code its result, and who merges it. Each fact is said
// once; a merge the plan moved away from Settings says Settings' owner and
// the plan's reason, and how to change it under a hairline. It holds nothing
// to press and changes nothing: Escape or leaving the card closes it, a press
// on the card picks the node as before. A part the core does not send is
// left out. A mark on the card with its own hint (`!`, `⚠`, Hide's mark)
// shows that hint alone: the card's stays shut while the pointer is on it.

import type { PointerEvent, ReactNode } from "react";
import { AgentMark } from "../AgentMark";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import type { BranchLine, ResultLine } from "../missionLine";
import { pacedSteps, type NodeLook, type NodePace, type NodeStep } from "../missions";
import { lookWord, type RunLabel } from "../missionParts";
import { planCount } from "../plans";
import { PR_LOOK } from "../prMark";
import type { Mission, MissionGate, MissionNode } from "../snapshot";
import { DeviceChip } from "./device-chip";
import { OperatorBand } from "./mission-gate";
import { BranchLineView, ResultLineView } from "./mission-line";
import { MissionNodeMark } from "./mission-node-mark";
import { ProgressCells, STEP_FILL } from "./mission-tone";
import { Tooltip, TooltipContent, TooltipTrigger, useHintOpen } from "./ui/tooltip";

export type NodeHoverFacts = {
  /** The card's state: its look, its mark's word and how it moves now (`nodePace`), and a resting or absent worker's words. */
  look: NodeLook;
  word: string;
  pace: NodePace | null;
  /** A done general node that ended on a failed verdict: the mark is red, as on the card (PRD node-presets D-08). */
  failed: boolean;
  doing: string | null;
  /** `n째` while it runs, `n 걸림` once it ended; none for a planned node. */
  took: string | null;
  /** The worker as the card names it, with its run's words (model, effort, profile, a planned mark) and a device away from this window. */
  worker: { agent: string | null; run: RunLabel; device: string | null } | null;
  /** Its steps: the worker's plan once it exists, else the Conductor's. */
  work: { source: "worker" | "plan"; steps: NodeStep[]; now: string | null } | null;
  /** Where its result goes, for a node that changes code, or what it found for one that does not. */
  branch: BranchLine | null;
  result: ResultLine | null;
  /** What the operator does on the node, the card's band. */
  gate: Pick<MissionGate, "state" | "look"> | null;
  mergeDue: boolean;
};

/** The state word in the colour of the card's mark. */
const LOOK_TONE: Record<NodeLook, string> = {
  waiting: "text-muted-foreground",
  working: "text-mission-working",
  done: "text-mission-done",
  external: "text-muted-foreground",
  cancelled: "text-mission-cancelled",
  dropped: "text-muted-foreground",
};

export function MissionNodeHover({ mission, node, facts, children }: { mission: Mission; node: MissionNode; facts: NodeHoverFacts; children: ReactNode }) {
  const { open, onOpenChange, triggerProps } = useHintOpen();
  // On a mark with its own hint the move is not the card's: it closes the card's hover and is kept from opening it.
  const onPointerMove = (event: PointerEvent<HTMLElement>) => {
    const trigger = event.target instanceof Element ? event.target.closest('[data-slot="tooltip-trigger"]') : null;
    if (trigger === null || trigger === event.currentTarget) return;
    event.preventDefault();
    if (open) onOpenChange(false);
  };
  return (
    <Tooltip open={open} onOpenChange={onOpenChange}>
      <TooltipTrigger asChild {...triggerProps} onPointerMove={onPointerMove}>
        {children}
      </TooltipTrigger>
      <TooltipContent side="right" align="start" className="w-(--size-mission-popover) max-w-(--radix-tooltip-content-available-width) text-left text-wrap rounded-md p-md" data-node-hover={node.key}>
        <NodeHoverBody mission={mission} node={node} facts={facts} />
      </TooltipContent>
    </Tooltip>
  );
}

function NodeHoverBody({ mission, node, facts }: { mission: Mission; node: MissionNode; facts: NodeHoverFacts }) {
  const { t } = useInterfaceTranslation();
  const merge = mergeBy(mission, node);
  // The plan gave the operator a merge Settings give the Conductor (D-10): the core keeps its reason only then.
  const override = mission.scope === "merge" && node.merge?.by === "operator";
  const current = node.prs.at(-1) ?? null;
  const pr = current?.state ? PR_LOOK[current.state] : null;
  const work = facts.work;
  const finished = node.plan?.progress.finished ?? false;
  const count = node.plan ? (finished ? t("mission.popover.stepsDone", { progress: planCount(node.plan) }) : planCount(node.plan)) : work ? t("mission.steps.planned", { count: work.steps.length }) : null;
  return (
    <div className="flex min-w-0 flex-col gap-sm text-caption">
      <span className="flex min-w-0 items-center gap-xs">
        <MissionNodeMark look={facts.look} label={facts.word} pace={facts.pace} failed={facts.failed} hint={false} />
        <span className="min-w-0 break-words text-body font-semibold text-foreground" data-node-hover-title="true">{node.title}</span>
      </span>
      <OperatorBand gate={facts.gate} mergeDue={facts.mergeDue} />
      <dl className="m-0 grid min-w-0 grid-cols-[auto_minmax(0,1fr)] items-start gap-x-md gap-y-xs">
        <Row label={t("mission.popover.status")} data="status">
          <span className={cn("font-medium", facts.pace === "stopped" ? "text-mission-stopped" : LOOK_TONE[facts.look])}>{lookWord(facts.look, t)}</span>
          {facts.doing ? <span className="text-muted-foreground">· {facts.doing}</span> : null}
          {facts.took ? <span className="text-muted-foreground">· {facts.took}</span> : null}
        </Row>
        {facts.worker ? (
          <Row label={t("mission.popover.worker")} data="worker">
            {facts.worker.agent ? <span className="flex shrink-0" aria-hidden="true"><AgentMark kind={facts.worker.agent} profile={facts.worker.run.profile} /></span> : null}
            <span className="min-w-0 break-words text-foreground">{facts.worker.run.words}</span>
            {facts.worker.run.mark ? <span className="text-muted-foreground">{facts.worker.run.mark}</span> : null}
            {facts.worker.device ? <DeviceChip label={facts.worker.device} variant="remote" /> : null}
          </Row>
        ) : null}
        {work ? (
          <Row label={t("mission.popover.steps")} data="steps" column>
            <span className="flex min-w-0 items-center gap-xs">
              <ProgressCells tones={pacedSteps(work.steps, facts.pace)} fill={STEP_FILL} size="head" className="h-(--size-mission-step-h) min-w-0 flex-1" />
              {count ? <span className="shrink-0 tabular-nums text-muted-foreground" data-node-hover-count="true">{count}</span> : null}
            </span>
            {work.now && node.status === "working" ? <span className="break-words text-subtle-foreground">{t(facts.pace === "stopped" ? "mission.steps.stoppedAt" : "mission.steps.now")} <span className="font-medium text-foreground">{work.now}</span></span> : null}
          </Row>
        ) : null}
        {facts.branch ? (
          <Row label={t("mission.popover.into")} data="into">
            <BranchLineView line={facts.branch} whole />
          </Row>
        ) : null}
        {facts.branch && pr ? (
          <Row label={t("mission.popover.pr")} data="pr">
            <pr.icon aria-hidden="true" className={cn("size-(--size-icon-sm) shrink-0", pr.tone)} />
            <span className="text-foreground">{t(pr.word)}</span>
          </Row>
        ) : null}
        {facts.result ? (
          <Row label={t("mission.popover.result")} data="result">
            <ResultLineView line={facts.result} whole />
          </Row>
        ) : null}
        {merge ? (
          <Row label={t("mission.popover.merge")} data="merge" column>
            <span className="text-foreground" data-node-hover-merge={merge}>{t(`mission.popover.who.${merge}`)}</span>
            {/* Why the plan moved it away from Settings, small under who. */}
            {override ? (
              <span className="break-words text-micro text-muted-foreground" data-node-hover-override="true">
                {[t("mission.popover.setting", { who: t("mission.popover.who.conductor") }), node.merge?.why].filter(Boolean).join(" · ")}
              </span>
            ) : null}
          </Row>
        ) : null}
      </dl>
      {override ? <span className="border-t border-border pt-xs text-muted-foreground" data-node-hover-change="true">{t("mission.popover.change")}</span> : null}
    </div>
  );
}

/** Who merges a node's change: its merge's owner for a pull request, the Conductor for a mission's own direct change; none for an outside node or one that changes no code. */
function mergeBy(mission: Mission, node: MissionNode): "operator" | "conductor" | null {
  if (node.merge) return node.merge.by;
  return node.kind === "change" && mission.scope === "merge" ? "conductor" : null;
}

/** One fact: its grey label in the first column, the card's parts for it in the second. */
function Row({ label, data, column = false, children }: { label: string; data: string; column?: boolean; children: ReactNode }) {
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd className={cn("m-0 flex min-w-0", column ? "flex-col gap-xxs" : "flex-wrap items-center gap-x-xxs")} data-node-hover-part={data}>
        {children}
      </dd>
    </>
  );
}
