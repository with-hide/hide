import { useMemo, useState } from "react";
import { AgentMark } from "../AgentMark";
import { agentProductName } from "../agentAdapters";
import { useMinuteClock } from "../components/elapsed";
import { GateStrip } from "../components/mission-gate";
import { MissionGraphCard, MissionGraphReport } from "../components/mission-graph-card";
import { MissionPrMark } from "../components/mission-pr";
import { TooltipProvider } from "../components/ui/tooltip";
import { useInterfaceTranslation } from "../i18n/translator";
import { MissionGraph } from "../MissionGraph";
import { MissionMapCanvas } from "../MissionMapCanvas";
import { attentionReasonText, escalationWhy, gateWaitedLong, holdReason, nodeLook, openAttention, stageWord } from "../missions";
import type { MissionMapFlow } from "../snapshot";
import { openDetail } from "./connection";
import { workerOf } from "./missionModel";
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
  const focus = nodes.filter((node) => node.status === "working" || holdReason(mission, node, workerOf) !== null).map((node) => node.key);
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
        <div className="flex h-[50svh] min-h-(--size-image-min-height) min-w-0 flex-col" data-phone-mission-map="true">
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

function PhoneNode({ mission, node }: { mission: PhoneMission; node: PhoneMissionNode }) {
  const { t } = useInterfaceTranslation();
  const look = nodeLook(mission, node);
  const reason = holdReason(mission, node, workerOf);
  const attention = openAttention(mission, node);
  const gate = node.gate;
  const long = gateWaitedLong(gate, useMinuteClock());
  const agent = usePhone((s) => node.agent && agentOf(s.groups, node.agent) ? node.agent : null);
  const yours = gate?.state === "open" || attention?.to === "operator" || reason?.kind === "decision" || reason?.kind === "worker";
  const word = t(`mission.look.${look}`);
  const detail = reason?.kind === "attention"
    ? `${reason.attention.to === "conductor" ? `${t("mission.attention.to.conductor")} · ` : ""}${attentionReasonText(reason.attention.reason, reason.attention.pr, t)}`
    : reason?.kind === "worker" ? escalationWhy(reason.escalation, t)
      : reason?.kind === "decision" ? t("mobile.mission.waitsDecision")
        : node.status === "planned" ? t("mission.steps.notStarted")
          : node.status === "working" ? t(stageWord(node)) : null;
  return (
    <div data-dependency-node={node.key} className="flex">
      <MissionGraphCard
        node={node} look={look} word={word} yours={yours}
        aria-label={[node.title, word, detail, node.worker_agent ? agentProductName(node.worker_agent) : null].filter(Boolean).join(" · ")}
        aria-disabled={!agent} onClick={agent ? () => openDetail(agent) : undefined}
        data-node-attention={attention?.to}
      >
        {detail ? <span className="truncate text-caption text-muted-foreground" title={detail} data-phone-node-reason="true">{detail}</span> : null}
        {node.worker_agent ? <span className="flex items-center gap-xxs text-caption text-muted-foreground"><AgentMark kind={node.worker_agent} />{agentProductName(node.worker_agent)}</span> : null}
        {gate ? <GateStrip gate={gate} /> : null}
        {gate?.state === "passed" && gate.via ? <span className="text-caption text-muted-foreground" data-phone-node-gate-via={gate.via}>{t(`mission.gate.via.${gate.via}`)}{gate.said ? ` · “${gate.said}”` : ""}</span> : null}
        {node.pr !== null || yours || long ? (
          <span className="flex items-center gap-xs text-caption text-muted-foreground">
            {node.pr !== null ? <MissionPrMark number={node.pr} state={node.pr_state} stale={mission.stale} /> : null}
            {yours ? <span className="font-semibold text-mission-hold" data-node-gate-tag={gate?.state}>{t("agents.mark.turn")}</span> : null}
            {long ? <span data-node-gate-long="true">· {t("mission.gate.long")}</span> : null}
          </span>
        ) : null}
      </MissionGraphCard>
    </div>
  );
}
