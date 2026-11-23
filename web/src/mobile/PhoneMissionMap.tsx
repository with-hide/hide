import { useMemo, useState } from "react";
import { AgentMark } from "../AgentMark";
import { agentProductName } from "../agentAdapters";
import { useMinuteClock } from "../components/elapsed";
import { OperatorBand } from "../components/mission-gate";
import { NodeWaitLine, useWaitWords } from "../components/mission-hold";
import { MissionGraphCard, MissionGraphReport } from "../components/mission-graph-card";
import { BranchLineView, ResultLineView, WaitCardLines, WaitStateMark } from "../components/mission-line";
import { TooltipProvider } from "../components/ui/tooltip";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { MissionGraph } from "../MissionGraph";
import { MissionMapCanvas } from "../MissionMapCanvas";
import { branchLine, branchLineWords, resultLine, resultLineWords, waitCard, waitCardWords } from "../missionLine";
import { gateWaitedLong, holdReason } from "../missions";
import type { MissionMapFlow } from "../snapshot";
import { openDetail } from "./connection";
import { phoneLineNode, stepState, workerOf } from "./missionModel";
import type { PhoneMission, PhoneMissionNode } from "./protocol";
import { agentOf, usePhone } from "./store";

/** Only the PRs actually sent to the phone, with no invented desktop metadata. */
function graphNode(node: PhoneMissionNode) {
  return { ...node, prs: node.pr === null ? [] : [{ number: node.pr }] };
}

export function PhoneMissionMap({ mission }: { mission: PhoneMission }) {
  const { t } = useInterfaceTranslation();
  const [flow, setFlow] = useState<MissionMapFlow>("down");
  const nodes = useMemo(() => mission.nodes.map(graphNode), [mission.nodes]);
  const graph = useMemo(() => ({ status: mission.status, nodes }), [mission.status, nodes]);
  const focus = nodes.filter((node) => node.status === "working" || node.merge?.state === "due" || holdReason(mission, node, workerOf) !== null).map((node) => node.key);
  const note = t(mission.status === "done" ? "mission.report.done" : mission.status === "cancelled" ? "mission.cancelled" : "mission.report.waiting");
  const omitted = mission.nodes_omitted ?? 0;
  return (
    <TooltipProvider><section className="pt-lg" data-phone-mission-steps="true">
      <h2 className="pb-xs text-body text-muted-foreground">{t("mobile.mission.steps")}</h2>
      {omitted > 0 ? (
        <p className="text-body text-muted-foreground" data-phone-mission-steps-omitted={omitted}>{t("mobile.mission.stepsOmitted", { count: omitted })}</p>
      ) : nodes.length === 0 ? (
        <p className="text-body text-muted-foreground" data-phone-mission-empty="true">{t(mission.status === "done" ? "mission.report.done" : mission.status === "cancelled" ? "mission.cancelled" : "mission.planning")}</p>
      ) : (
        <div className="flex h-[50svh] min-w-0 flex-col" data-phone-mission-map="true">
          <MissionMapCanvas wide page={null} flow={flow} onFlow={setFlow} touch focusNodes={focus}>
            <MissionGraph
              mission={graph} flow={flow} selected={null} onSelect={() => {}}
              drawNode={(node) => <PhoneNode mission={mission} node={node} />}
              report={(dimmed) => <MissionGraphReport status={mission.status} note={note} look={mission.status === "done" ? "done" : mission.status === "cancelled" ? "cancelled" : "waiting"} dimmed={dimmed} />}
            />
          </MissionMapCanvas>
        </div>
      )}
    </section></TooltipProvider>
  );
}

/**
 * A step's card on the phone (PRD mission-destination D-51-D-55, B59): the
 * desktop's frame and rows from what the phone's frame carries. Its title;
 * right under it the operator's band, the gate's strip and a merge that is
 * theirs now; its worker's mark and name; why it waits, when it does; the
 * wait its Conductor set, as the desktop's card says it (B21); and its
 * bottom line under a hairline, the desktop's branch line without the branch
 * name (the frame carries none) or a general step's preset icon and name, then
 * `내 차례` at its first row's end when the step waits on the operator for more than its merge. The
 * model, the time, the steps bar and a finished step's conclusion or verdict
 * are not in the frame and are not drawn (`docs/INSTALL.md`). A tap opens the
 * worker's conversation; the phone has no popover (D-38).
 */
function PhoneNode({ mission, node }: { mission: PhoneMission; node: PhoneMissionNode }) {
  const { t } = useInterfaceTranslation();
  // Where it stands, by the one rule the step list reads too (`stepState`); a start gate once passed is the desktop panel's alone (PRD node-presets D-09).
  const { look, pace, gate, attention, turn, mergeDue, stopped, asleep, why: detail, word } = stepState(mission, node, t);
  const minute = useMinuteClock();
  const long = gateWaitedLong(gate, minute);
  const agent = usePhone((s) => node.agent && agentOf(s.groups, node.agent) ? node.agent : null);
  const line = useMemo(() => phoneLineNode(node, mission.nodes), [node, mission.nodes]);
  const branch = useMemo(() => branchLine(line, mission.destination ?? null), [line, mission.destination]);
  // A waiting step says what it waits for and how that stands, as the desktop's card does (option A, operator decision on with-hide/hide pull request 2848; PRD core-wait B22); a step a Mac before waits sent with nothing it watches has none.
  const waits = useMemo(() => waitCard(line, minute), [line, minute]);
  const result = branch || waits || line.kind === "wait" ? null : resultLine(line);
  // `내 차례` stands at the right end of the line's first row as the desktop's marks do (D-56); a long wait's words give way first, the tag's own word never.
  const marks = turn ? (
    <span className="flex min-w-0 items-center justify-end gap-x-xs overflow-hidden" data-node-foot-end="true">
      <span className="flex items-center gap-x-xxs font-semibold text-mission-hold" data-node-gate-tag={gate?.state}>
        <span className="shrink-0">{t("agents.mark.turn")}</span>
        {long ? <span className="min-w-0 shrink-[10] truncate font-normal text-muted-foreground" data-node-gate-long="true"> · {t("mission.gate.long")}</span> : null}
      </span>
    </span>
  ) : null;
  const waitLine = useWaitWords(node.wait);
  return (
    <div data-dependency-node={node.key} className="flex">
      <MissionGraphCard
        node={node} look={look} word={word} yours={mergeDue || turn} pace={pace}
        mark={waits ? <WaitStateMark state={waits.state} label={word} /> : undefined}
        aria-label={[
          node.title, word, node.worker_agent ? agentProductName(node.worker_agent) : null, asleep, detail, waitLine?.full ?? null,
          // The lines are drawn for the eye, so their words and states are said here (B28).
          branch ? `${t("mission.foot.where")}: ${branchLineWords(branch, t)}` : null,
          waits ? waitCardWords(waits, t) : null,
          result ? resultLineWords(result, t) : null,
          gate ? `${t("mission.gate.title")} · ${gate.look}${gate.state === "passed" ? ` · ${t("mission.gate.passed")}` : gate.state === "open" ? ` · ${t("agents.mark.turn")}` : ""}${long ? ` · ${t("mission.gate.long")}` : ""}` : null,
          mergeDue && !branch?.person ? t("mission.merge.turn") : null,
        ].filter(Boolean).join(" · ")}
        aria-disabled={!agent} onClick={agent ? () => openDetail(agent) : undefined}
        data-node-attention={attention?.to}
      >
        <OperatorBand gate={gate} mergeDue={mergeDue} />
        {gate?.state === "passed" && gate.via ? <span className="text-caption text-muted-foreground" data-phone-node-gate-via={gate.via}>{t(`mission.gate.via.${gate.via}`)}{gate.said ? ` · “${gate.said}”` : ""}</span> : null}
        {node.worker_agent ? (
          <span className="flex min-w-0 items-center gap-xxs text-caption text-muted-foreground" data-node-run="worker">
            <span className="flex shrink-0" aria-hidden="true" data-node-worker-mark="true"><AgentMark kind={node.worker_agent} /></span>
            <span className="min-w-0 truncate">{agentProductName(node.worker_agent)}</span>
          </span>
        ) : null}
        {stopped ? <span className="truncate text-caption text-mission-stopped" data-node-work={node.work?.state}>{stopped}</span> : null}
        {asleep ? <span className="truncate text-caption text-muted-foreground" data-node-work="asleep">{asleep}</span> : null}
        {node.wait && waitLine ? <NodeWaitLine wait={node.wait} words={waitLine} /> : null}
        {/* A waiting step's foot says how its wait failed and whose turn that is, so the reason is not said twice. */}
        {detail && !waits ? <span className="truncate text-caption text-muted-foreground" title={detail} data-phone-node-reason="true">{detail}</span> : null}
        {waits ? <WaitCardLines card={waits} /> : branch || result || marks ? (
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
    </div>
  );
}
