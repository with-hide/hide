import { useMemo, useState } from "react";
import { AgentMark } from "../AgentMark";
import { agentProductName } from "../agentAdapters";
import { useMinuteClock } from "../components/elapsed";
import { OperatorBand } from "../components/mission-gate";
import { MissionGraphCard, MissionGraphReport } from "../components/mission-graph-card";
import { BranchLineView, ResultLineView, WaitingMark } from "../components/mission-line";
import { TooltipProvider } from "../components/ui/tooltip";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { MissionGraph } from "../MissionGraph";
import { MissionMapCanvas } from "../MissionMapCanvas";
import { branchLine, branchLineWords, resultLine, resultLineWords } from "../missionLine";
import { attentionReasonText, escalationWhy, gateWaitedLong, holdReason, nodeLook, openAttention } from "../missions";
import type { MissionMapFlow } from "../snapshot";
import { openDetail } from "./connection";
import { phoneLineNode, workerOf } from "./missionModel";
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
 * theirs now; its worker's mark and name; why it waits, when it does; and its
 * bottom line under a hairline, the desktop's branch line without the branch
 * name (the frame carries none) or a general step's preset icon and name, then
 * `내 차례` at its first row's end when the step waits on the operator for more than its merge. The
 * model, the time, the steps bar and a finished step's conclusion or verdict
 * are not in the frame and are not drawn (`docs/INSTALL.md`). A tap opens the
 * worker's conversation; the phone has no popover (D-38).
 */
function PhoneNode({ mission, node }: { mission: PhoneMission; node: PhoneMissionNode }) {
  const { t } = useInterfaceTranslation();
  const look = nodeLook(mission, node);
  const reason = holdReason(mission, node, workerOf);
  const attention = openAttention(mission, node);
  const gate = node.gate ?? null;
  const long = gateWaitedLong(gate, useMinuteClock());
  const agent = usePhone((s) => node.agent && agentOf(s.groups, node.agent) ? node.agent : null);
  // A merge the core says is the operator's now is the band's (D-53); one ahead or done is the person on the branch line.
  const mergeDue = node.merge?.state === "due";
  const line = useMemo(() => phoneLineNode(node, mission.nodes), [node, mission.nodes]);
  // An outside step a Mac before destinations sent with nothing it watches has no line: a lone mark would say nothing (B60).
  const branch = useMemo(() => line.kind === "external" && !line.external ? null : branchLine(line, mission.destination ?? null), [line, mission.destination]);
  const result = branch ? null : resultLine(line);
  // `내 차례` at the line's end says the step waits on the operator for something the band and line do not: a gate reached, a raised reason, a decision or the worker's turn.
  const turn = gate?.state === "open" || attention?.to === "operator" || reason?.kind === "decision" || reason?.kind === "worker";
  // `내 차례` stands at the right end of the line's first row as the desktop's marks do (D-56); a long wait's words give way first, the tag's own word never.
  const marks = turn ? (
    <span className="flex min-w-0 items-center justify-end gap-x-xs overflow-hidden" data-node-foot-end="true">
      <span className="flex items-center gap-x-xxs font-semibold text-mission-hold" data-node-gate-tag={gate?.state}>
        <span className="shrink-0">{t("agents.mark.turn")}</span>
        {long ? <span className="min-w-0 shrink-[10] truncate font-normal text-muted-foreground" data-node-gate-long="true"> · {t("mission.gate.long")}</span> : null}
      </span>
    </span>
  ) : null;
  const word = t(`mission.look.${look}`);
  const detail = reason?.kind === "attention"
    ? `${reason.attention.to === "conductor" ? `${t("mission.attention.to.conductor")} · ` : ""}${attentionReasonText(reason.attention.reason, reason.attention.pr, t)}`
    : reason?.kind === "worker" ? escalationWhy(reason.escalation, t)
      : reason?.kind === "decision" ? t("mobile.mission.waitsDecision") : null;
  return (
    <div data-dependency-node={node.key} className="flex">
      <MissionGraphCard
        node={node} look={look} word={word} yours={mergeDue || turn}
        aria-label={[
          node.title, word, node.worker_agent ? agentProductName(node.worker_agent) : null, detail,
          // The lines are drawn for the eye, so their words and states are said here (B28).
          branch ? `${t("mission.foot.where")}: ${branchLineWords(branch, t)}` : null,
          result ? resultLineWords(result, t) : null,
          gate ? `${t("mission.gate.title")} · ${gate.look}${gate.state === "passed" ? ` · ${t("mission.gate.passed")}` : gate.state === "open" ? ` · ${t("agents.mark.turn")}` : ""}${long ? ` · ${t("mission.gate.long")}` : ""}` : null,
          mergeDue && !branch?.person ? t("mission.merge.turn") : null,
        ].filter(Boolean).join(" · ")}
        aria-disabled={!agent} onClick={agent ? () => openDetail(agent) : undefined}
        data-node-attention={attention?.to}
      >
        <OperatorBand gate={gate} mergeDue={mergeDue} />
        {gate?.state === "passed" && gate.via ? <span className="text-caption text-muted-foreground" data-phone-node-gate-via={gate.via}>{t(`mission.gate.via.${gate.via}`)}{gate.said ? ` · “${gate.said}”` : ""}</span> : null}
        {/* A waiting step has no worker and says `⏳ 기다림` in its place, as the desktop's card does (PRD node-presets D-13). */}
        {node.kind === "external" ? (
          <span className="flex min-w-0 items-center gap-xxs text-caption text-muted-foreground" data-node-run="waiting"><WaitingMark /></span>
        ) : null}
        {node.worker_agent ? (
          <span className="flex min-w-0 items-center gap-xxs text-caption text-muted-foreground" data-node-run="worker">
            <span className="flex shrink-0" aria-hidden="true" data-node-worker-mark="true"><AgentMark kind={node.worker_agent} /></span>
            <span className="min-w-0 truncate">{agentProductName(node.worker_agent)}</span>
          </span>
        ) : null}
        {detail ? <span className="truncate text-caption text-muted-foreground" title={detail} data-phone-node-reason="true">{detail}</span> : null}
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
    </div>
  );
}
