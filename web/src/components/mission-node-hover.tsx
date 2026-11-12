// What a mission node's card says on hover or focus on the desktop (PRD
// mission-destination D-19, D-20, B32, B33): its title and one line, its
// worker in full (agent, model, effort, profile, device), where its work is
// now, where its result goes with who merges it and its worker's worktree
// branch, and the Conductor's last line. It holds nothing to press and
// changes nothing: Escape or leaving the card closes it, a press on the card
// picks the node as before. A part the core does not send is left out. A
// mark on the card with its own hint (`!`, `⚠`, Hide's mark) shows that hint
// alone: the card's stays shut while the pointer is on it.

import type { PointerEvent, ReactNode } from "react";
import { AgentMark } from "../AgentMark";
import { agentProductName } from "../agentAdapters";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { branchLineWords, type BranchLine } from "../missionLine";
import { planCount } from "../plans";
import type { Mission, MissionNode } from "../snapshot";
import { useElapsed } from "./elapsed";
import { Tooltip, TooltipContent, TooltipTrigger, useHintOpen } from "./ui/tooltip";

export type NodeHoverFacts = {
  /** The worker as the card names it: its agent, the run's words (profile, model, effort, a planned mark) and the device away from this window. */
  worker: { agent: string | null; words: string; device: string | null } | null;
  /** The step being worked, when a worker's plan has one. */
  now: string | null;
  /** Where its result goes, for a node that changes code. */
  branch: BranchLine | null;
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
  const pushed = useElapsed(node.pushed_at_unix_ms ?? null);
  const plan = node.plan ? t("plan.chip", { progress: planCount(node.plan) }) : null;
  const now = [plan, facts.now, pushed ? t("mission.popover.pushed", { ago: pushed }) : null].filter(Boolean).join(" · ");
  const merge = mergeBy(mission, node);
  // The plan gave the operator a merge Settings give the Conductor (D-10): the core keeps its reason only then.
  const override = mission.scope === "merge" && node.merge?.by === "operator";
  return (
    <div className="flex min-w-0 flex-col gap-sm text-caption">
      <div className="flex min-w-0 flex-col gap-xxs">
        <span className="break-words text-body font-semibold text-foreground" data-node-hover-title="true">{node.title}</span>
        {node.change ? <span className="break-words text-muted-foreground" data-node-hover-change="true">{node.change}</span> : null}
      </div>
      {facts.worker ? (
        <Part label={t("mission.role.worker")} data="worker">
          <span className="flex min-w-0 items-start gap-xs">
            {facts.worker.agent ? <span className="mt-xxs flex shrink-0"><AgentMark kind={facts.worker.agent} /></span> : null}
            <span className="min-w-0 break-words">{[agentProductName(facts.worker.agent), facts.worker.words, facts.worker.device].filter(Boolean).join(" · ")}</span>
          </span>
        </Part>
      ) : null}
      {now ? <Part label={t("mission.steps.now")} data="now"><span className="break-words">{now}</span></Part> : null}
      {facts.branch ? (
        <Part label={t("mission.foot.where")} data="where">
          <span className="break-words font-medium text-foreground" data-node-hover-path="true">{branchLineWords(facts.branch, t)}</span>
          {node.prs.length > 1 ? <span className="break-words font-mono" data-node-hover-prs="true">{t("mission.popover.prs")} {node.prs.map((pr) => `#${pr.number}`).join(", ")}</span> : null}
          {merge ? <span data-node-hover-merge={merge}>{t(`mission.popover.mergeBy.${merge}`)}</span> : null}
          {override ? (
            <span className="flex min-w-0 flex-col gap-xxs rounded-xs bg-mission-hold/10 px-xs py-xxs text-foreground" data-node-hover-override="true">
              <span className="break-words">{t("mission.popover.override")}{node.merge?.why ? ` · ${node.merge.why}` : ""}</span>
              <span className="text-muted-foreground">{t("mission.popover.change")}</span>
            </span>
          ) : null}
          {node.branch ? (
            <span className="flex min-w-0 gap-xs" data-node-hover-branch={node.branch}>
              <span className="shrink-0 text-muted-foreground">{t("mission.popover.branch")}</span>
              <span className="min-w-0 break-all font-mono">{node.branch}</span>
            </span>
          ) : null}
        </Part>
      ) : null}
      {node.line ? <Part label={t("mission.popover.line")} data="line"><span className="break-words">{node.line}</span></Part> : null}
    </div>
  );
}

/** Who merges a node's change: its merge's owner for a pull request, the Conductor for a mission's own direct change; none for an outside node or one that changes no code. */
function mergeBy(mission: Mission, node: MissionNode): "operator" | "conductor" | null {
  if (node.merge) return node.merge.by;
  return node.kind === "change" && mission.scope === "merge" ? "conductor" : null;
}

function Part({ label, data, children }: { label: string; data: string; children: ReactNode }) {
  return (
    <div className={cn("flex min-w-0 flex-col gap-xxs")} data-node-hover-part={data}>
      <span className="text-micro font-semibold text-muted-foreground">{label}</span>
      {children}
    </div>
  );
}
