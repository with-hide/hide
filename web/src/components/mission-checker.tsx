import { useMemo, useState, type ReactNode } from "react";
import type { Actions } from "../actions";
import { AgentMark } from "../AgentMark";
import { agentMarkOf } from "../agentLogos";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { RunText, useRunWords } from "../missionParts";
import { checkerState } from "../missions";
import { allAgents, agentsByPane } from "../paneAgents";
import { sessionOfAgent } from "../sessionWork";
import type { Mission, MissionChecker } from "../snapshot";
import { useShellStore } from "../store";
import { Button } from "./ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import { Hint } from "./ui/tooltip";

// One agent checking a node, opened from its satellite on the map or its chip
// under a finish line (PRD mission-tool B17, PRD mission-map-git-criteria
// B32): the agent, the model its pane was launched with, its answer, the
// short reason it gave the operator with its full text folded, and the ways
// to its terminal and its record; from the map, also the node's panel.

/** The checker's words for its state, the same on the satellite, the chip and the popover. */
export function useCheckerStatus(kind: "review" | "verify", checker: MissionChecker): string {
  const { t } = useInterfaceTranslation();
  const state = checkerState(checker);
  return state === "pass" || state === "fail" ? t(`mission.checker.${state}`) : t(`mission.checker.${state}.${kind}`);
}

/**
 * A result's reason as the operator reads it (B32): the one line its agent
 * wrote for them (`--summary`), else the start of its evidence; the full
 * evidence folds under 원문, so a long English report never stands in the way.
 */
export function VerdictReason({ summary, evidence, className }: { summary?: string | null; evidence?: string | null; className?: string }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const raw = evidence?.trim() || null;
  const lead = summary?.trim() || null;
  if (!lead && !raw) return null;
  return (
    <div className={cn("flex min-w-0 flex-col gap-xxs", className)} data-verdict-reason={lead ? "summary" : "evidence"}>
      {lead ? <p className="min-w-0 break-words text-foreground" data-verdict-summary="true">{lead}</p> : null}
      {raw && (!lead || open) ? (
        <p className={cn("min-w-0 whitespace-pre-wrap break-words text-muted-foreground", lead && "rounded-xs bg-secondary px-xs py-xxs font-mono", !open && "line-clamp-2")} data-verdict-raw={open ? "open" : "clamped"}>{raw}</p>
      ) : null}
      {raw ? (
        <Button variant="text" size="inline" className="w-fit text-muted-foreground" aria-expanded={open} onClick={() => setOpen(!open)} data-verdict-raw-toggle={open ? "open" : "folded"}>
          {t(open ? "mission.verdict.rawHide" : "mission.verdict.raw")}
        </Button>
      ) : null}
    </div>
  );
}

/** The checker's live row and its session: the terminal follows the one, the record the other. */
function useCheckerWays(checker: MissionChecker) {
  const row = useShellStore((s) => (checker.pane_id ? agentsByPane(allAgents(s.agents, s.rest?.status?.remote)).get(checker.pane_id) ?? null : null));
  // Derived outside the selector: it builds a new session each call, which a selector must not return.
  const rest = useShellStore((s) => s.rest);
  const session = useMemo(() => (row ? sessionOfAgent(row) : null), [row, rest]);
  return { row, session };
}

/**
 * The popover of one checker around the trigger it is given; `hint` is the
 * trigger's hover while the popover is shut, and `onNode` adds the way to the
 * node's panel, for a satellite whose node may not be the one picked.
 */
export function CheckerPopover({ mission, kind, checker, actions, hint, onNode, children }: { mission: Mission; kind: "review" | "verify"; checker: MissionChecker; actions: Actions; hint: string; onNode?: () => void; children: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const state = checkerState(checker);
  const status = useCheckerStatus(kind, checker);
  const run = useRunWords(mission, checker.pane_id, checker.machine, checker.worker);
  const { row, session } = useCheckerWays(checker);
  const name = agentMarkOf(checker.worker.agent)?.label ?? checker.worker.agent;
  const go = (action: () => void) => () => {
    setOpen(false);
    action();
  };
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Hint label={hint} active={!open}>
        <PopoverTrigger asChild>{children}</PopoverTrigger>
      </Hint>
      <PopoverContent align="start" className="flex flex-col gap-sm text-caption" data-checker-popover={checker.worker.agent}>
        <div className="flex min-w-0 items-center gap-xs text-body">
          <AgentMark kind={checker.worker.agent} />
          <span className="shrink-0 font-semibold">{name}</span>
          {run ? <RunText run={run} className="min-w-0 flex-1 text-caption text-muted-foreground" /> : <span className="flex-1" />}
        </div>
        <span className={cn("w-fit rounded-full px-sm font-semibold", VERDICT_TONE[state])} data-checker-popover-state={state}>
          {state === "fail" ? t(`mission.verdict.rejected`) : status}
        </span>
        <VerdictReason summary={checker.summary} evidence={checker.evidence} />
        <div className="flex flex-wrap gap-xs">
          <Button size="sm" variant="secondary" disabled={!checker.pane_id || !row} onClick={go(() => checker.pane_id && actions.openAgent(checker.pane_id))} data-checker-terminal="true">{t("mission.toTerminal")}</Button>
          <Button size="sm" variant="secondary" disabled={!session} onClick={go(() => session && actions.openChildRecord(session))} data-checker-record="true">{t("mission.record")}</Button>
          {onNode ? <Button size="sm" variant="text" onClick={go(onNode)} data-checker-node="true">{t("mission.checker.node")}</Button> : null}
        </div>
      </PopoverContent>
    </Popover>
  );
}

/** A verdict's chip: rejected in the failed colour, passed green, at work in the review colour, not started muted. */
export const VERDICT_TONE: Record<ReturnType<typeof checkerState>, string> = {
  fail: "bg-mission-failed/15 text-mission-failed",
  pass: "bg-mission-done/15 text-mission-done",
  running: "bg-mission-review/15 text-mission-review",
  waiting: "bg-muted text-muted-foreground",
};
