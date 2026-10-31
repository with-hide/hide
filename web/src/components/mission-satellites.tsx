import { CheckIcon, XIcon } from "lucide-react";
import type { Actions } from "../actions";
import { AgentMark } from "../AgentMark";
import { agentMarkOf } from "../agentLogos";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { useRunWords } from "../missionParts";
import { checkerState } from "../missions";
import type { Mission, MissionChecker, MissionCriterion } from "../snapshot";
import { CheckerPopover, useCheckerStatus } from "./mission-checker";
import { Button } from "./ui/button";

// The other agents checking a node, each a round mark tethered to its card's
// right side (PRD mission-tool D-05, B17): the checker's agent mark inside a
// ring whose colour is where its check is (reviewing or verifying in the
// review colour, passed green with a small check, failed red with a small
// x, not started dashed). Several stack top down. Hovering one, and its
// accessible name, say the agent, the model and effort its pane was launched
// with (or the plan's, marked planned) and its state; pressing one opens its
// popover: the answer, the reason the agent gave the operator, and the ways to
// its terminal, its record and the node's panel.

const RING: Record<ReturnType<typeof checkerState>, string> = {
  running: "border-mission-review",
  pass: "border-mission-done",
  fail: "border-mission-failed",
  waiting: "border-dashed border-mission-waiting",
};

export type SatelliteLine = { kind: "review" | "verify"; checker: MissionChecker };

/** A node's checkers in order, each with the kind of line it checks. */
export function satelliteLines(criteria: MissionCriterion[]): SatelliteLine[] {
  return criteria.flatMap((line) => (line.kind === "review" || line.kind === "verify" ? (line.checkers ?? []).map((checker) => ({ kind: line.kind as "review" | "verify", checker })) : []));
}

export function Satellites({ mission, lines, dimmed, onSelect, actions }: { mission: Mission; lines: SatelliteLine[]; dimmed: boolean; onSelect: () => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return (
    <span role="group" aria-label={t("mission.checkers")} className={cn("relative flex flex-col gap-xxs pl-xs transition-opacity", dimmed && "opacity-(--opacity-dimmed)")} data-node-satellites={lines.length}>
      {/* The tether from the card's edge to the first mark. */}
      <span aria-hidden="true" className="absolute top-1/2 left-0 h-(--size-mission-edge) w-(--spacing-xs) -translate-y-1/2 bg-border" />
      {lines.map((line, index) => <Satellite key={index} mission={mission} line={line} onSelect={onSelect} actions={actions} />)}
    </span>
  );
}

function Satellite({ mission, line, onSelect, actions }: { mission: Mission; line: SatelliteLine; onSelect: () => void; actions: Actions }) {
  const { checker, kind } = line;
  const state = checkerState(checker);
  const run = useRunWords(mission, checker.pane_id, checker.machine, checker.worker);
  const status = useCheckerStatus(kind, checker);
  const label = [agentMarkOf(checker.worker.agent)?.label ?? checker.worker.agent, run?.text, status].filter(Boolean).join(" · ");
  return (
    <CheckerPopover mission={mission} kind={kind} checker={checker} actions={actions} hint={label} onNode={onSelect}>
      <Button
        variant="ghost"
        size="icon-xs"
        aria-label={label}
        className={cn("relative size-(--size-icon-lg) rounded-full border-2 bg-card p-0", RING[state])}
        data-node-satellite={checker.worker.agent}
        data-checker-state={state}
      >
        <AgentMark kind={checker.worker.agent} className="size-(--size-agent-mark)!" />
        {state === "pass" || state === "fail" ? (
          <span aria-hidden="true" className={cn("absolute -right-xs -bottom-xs flex size-(--size-agent-mark) items-center justify-center rounded-full text-status-foreground ring-2 ring-background", state === "pass" ? "bg-mission-done" : "bg-mission-failed")}>
            {state === "pass" ? <CheckIcon strokeWidth={3} className="size-full p-px" /> : <XIcon strokeWidth={3} className="size-full p-px" />}
          </span>
        ) : null}
      </Button>
    </CheckerPopover>
  );
}
