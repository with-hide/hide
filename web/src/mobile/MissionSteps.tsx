// The mission page's steps as a list (Doop 14), the page's default view; the
// desktop graph (`PhoneMissionMap`) waits behind the head's map button. A
// step's row is its mark, its title and its pull request, then one line:
// what the operator owes it, how long its worker has worked on it, or where
// its work goes. Where a step stands is `stepState` (`missionModel.ts`), the
// rule the graph's card reads too, and its line is `missionLine.ts`.

import { useMemo } from "react";
import { useElapsed } from "../components/elapsed";
import { MissionNodeMark } from "../components/mission-node-mark";
import { StatusMark } from "../components/status-mark";
import { RowButton } from "../components/ui/row-button";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { branchLine, linePrLabel, lineToWords, resultLine, resultLineWords, waitLine, waitLineWords } from "../missionLine";
import { PR_LOOK } from "../prMark";
import type { PrState } from "../snapshot";
import { statusText } from "../agentStatus";
import { openDetail } from "./connection";
import { TwoLineRow } from "./layout";
import { phoneLineNode, stepState } from "./missionModel";
import type { PhoneMission, PhoneMissionNode } from "./protocol";
import { agentOf, usePhone } from "./store";

export function MissionSteps({ mission }: { mission: PhoneMission }) {
  const { t } = useInterfaceTranslation();
  const omitted = mission.nodes_omitted ?? 0;
  return (
    <section className="pt-lg" data-phone-mission-steps="true" data-phone-mission-view="steps">
      <h2 className="flex gap-sm pb-xs text-subhead text-muted-foreground">
        <span>{t("mobile.mission.steps")}</span>
        {mission.nodes.length > 0 ? <span className="font-mono">{mission.nodes.length}</span> : null}
      </h2>
      {omitted > 0 ? (
        <p className="text-body text-muted-foreground" data-phone-mission-steps-omitted={omitted}>{t("mobile.mission.stepsOmitted", { count: omitted })}</p>
      ) : mission.nodes.length === 0 ? (
        <p className="text-body text-muted-foreground" data-phone-mission-empty="true">{t(mission.status === "done" ? "mission.report.done" : mission.status === "cancelled" ? "mission.cancelled" : "mission.planning")}</p>
      ) : (
        <ul className="-mx-lg">
          {mission.nodes.map((node) => <li key={node.key}><StepRow mission={mission} node={node} /></li>)}
        </ul>
      )}
    </section>
  );
}

/**
 * One step (Doop 14): a step that waits on the operator stands in the hold band with why and `내 차례`; a worked one names its
 * worker's task, its state and, from a Mac that sends when it went to work, for how long; the rest say where their work goes.
 * Its lead is the step's mark, or the operator's turn mark while it waits on them. A press opens the worker's conversation while the phone's list holds it (B36).
 */
function StepRow({ mission, node }: { mission: PhoneMission; node: PhoneMissionNode }) {
  const { t } = useInterfaceTranslation();
  const { look, pace, turn: owed, mergeDue, stopped, asleep, why: reason, gate, word: lookWord, working } = stepState(mission, node, t);
  const agent = usePhone((s) => (node.agent ? agentOf(s.groups, node.agent) : null));
  // How long its worker has worked counts only while it works: a stopped or sleeping step's start says nothing of now.
  const elapsed = useElapsed(working ? node.worked_at_unix_ms : null);
  const line = useMemo(() => phoneLineNode(node, mission.nodes), [node, mission.nodes]);
  const branch = useMemo(() => (line.kind === "wait" && !line.external ? null : branchLine(line, mission.destination ?? null)), [line, mission.destination]);
  // A waiting step on CI, another mission or a node of one says what it waits on, as its map card does (PRD core-wait D-15, D-16).
  const target = useMemo(() => waitLine(line), [line]);
  const result = branch || target ? null : resultLine(line);
  // The list's row counts a merge that is the operator's now as their turn too: it has no band to draw it in.
  const turn = owed || mergeDue;
  const why = reason ?? (mergeDue ? t("mission.merge.turn") : gate?.state === "open" ? gate.look : null);
  const word = stopped ?? asleep ?? (look === "working" ? statusText(t, "working") : lookWord);
  // A step nobody has started yet is its title alone (Doop 14).
  const text = turn
    ? [why ?? word, t("agents.mark.turn")].join(" · ")
    : look === "waiting" && !asleep ? null
      : look === "working" ? [agent?.title, word, elapsed].filter(Boolean).join(" · ")
        : target ? waitLineWords(target, t)
        : branch ? [branch.plan ? t(branch.plan === "pr" ? "mission.foot.planPr" : "mission.foot.planDirect") : branch.mark in PR_LOOK ? t(PR_LOOK[branch.mark as PrState].word) : word, branch.to ? `→ ${lineToWords(branch.to, t)}` : null].filter(Boolean).join(" ")
          : result ? resultLineWords(result, t) : word;
  const pr = branch?.pr ? { label: linePrLabel(branch.pr), look: branch.mark in PR_LOOK ? PR_LOOK[branch.mark as PrState] : null } : null;
  const Icon = pr?.look?.icon;
  return (
    <RowButton
      aria-label={[node.title, word, text, pr?.label].filter(Boolean).join(" · ")}
      aria-disabled={!agent}
      data-phone-step={node.key}
      data-phone-step-turn={turn || undefined}
      className={cn("min-h-(--size-touch-target) rounded-none border-l-2 px-lg py-md hover:bg-transparent active:bg-accent", turn ? "border-mission-hold bg-mission-hold/10" : "border-transparent")}
      onClick={() => { if (agent) openDetail({ device_id: agent.device_id, pane_id: agent.pane_id }); }}
    >
      <TwoLineRow
        lead={turn ? <StatusMark mark="turn" /> : <MissionNodeMark look={look} pace={pace} label={word} hint={false} />}
        head={
          <>
            <span className={cn("min-w-0 flex-1 truncate text-title", look === "waiting" ? "text-subtle-foreground" : "text-foreground")}>{node.title}</span>
            {pr ? (
              <span className="flex shrink-0 items-center gap-xxs rounded-full bg-secondary px-sm font-mono text-body text-subtle-foreground" data-phone-step-pr={pr.label}>
                {Icon ? <Icon aria-hidden="true" className={cn("size-(--size-pr-icon)", pr.look?.tone)} /> : null}
                {pr.label}
              </span>
            ) : null}
          </>
        }
        line={text ? <span className={cn("min-w-0 truncate", turn ? "text-mission-hold" : "text-muted-foreground")} data-phone-step-line="true">{text}</span> : null}
      />
    </RowButton>
  );
}
