import { useMemo, useState } from "react";
import { AgentMark } from "../AgentMark";
import { agentProductName } from "../agentAdapters";
import { useMinuteClock } from "../components/elapsed";
import { HumanStrip, mergeWords } from "../components/mission-gate";
import { MissionGraphCard, MissionGraphReport } from "../components/mission-graph-card";
import { MissionPrMark } from "../components/mission-pr";
import { StationLine } from "../components/mission-stations";
import { TooltipProvider } from "../components/ui/tooltip";
import { useInterfaceTranslation } from "../i18n/translator";
import { MissionGraph } from "../MissionGraph";
import { MissionMapCanvas } from "../MissionMapCanvas";
import { attentionReasonText, escalationWhy, gateWaitedLong, holdReason, missionOpen, nodeLook, openAttention, stageWord } from "../missions";
import { nodeStations, stationWords } from "../missionStations";
import type { MissionMapFlow } from "../snapshot";
import { openDetail } from "./connection";
import { phoneMergeOverridden, phoneStationNode, workerOf } from "./missionModel";
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
 * A step's card on the phone: the desktop's frame and title, why it waits,
 * its worker, the strip of the human steps its plan has (the gate's and a
 * merge its plan gives the operator, PRD mission-destination D-45), and its
 * last line: for a step that changes code the desktop's stations to the
 * branch its result lands in (D-13, D-38), then `내 차례`. A tap opens the
 * worker's conversation; the phone has no popover (D-38, B32).
 */
function PhoneNode({ mission, node }: { mission: PhoneMission; node: PhoneMissionNode }) {
  const { t } = useInterfaceTranslation();
  const look = nodeLook(mission, node);
  const reason = holdReason(mission, node, workerOf);
  const attention = openAttention(mission, node);
  const gate = node.gate ?? null;
  const long = gateWaitedLong(gate, useMinuteClock());
  const agent = usePhone((s) => node.agent && agentOf(s.groups, node.agent) ? node.agent : null);
  const due = node.merge?.state === "due";
  const merging = phoneMergeOverridden(mission, node);
  const open = missionOpen(mission);
  const stations = useMemo(() => nodeStations(phoneStationNode(node), mission.destination ?? null, open), [node, mission.destination, open]);
  const yours = due || gate?.state === "open" || attention?.to === "operator" || reason?.kind === "decision" || reason?.kind === "worker";
  const word = t(`mission.look.${look}`);
  // A merge that is the operator's now names its pull request here when no `👤` strip says it, as from a Mac before scopes reached the phone.
  const detail = reason?.kind === "attention"
    ? `${reason.attention.to === "conductor" ? `${t("mission.attention.to.conductor")} · ` : ""}${attentionReasonText(reason.attention.reason, reason.attention.pr, t)}`
    : reason?.kind === "worker" ? escalationWhy(reason.escalation, t)
      : reason?.kind === "decision" ? t("mobile.mission.waitsDecision")
        : due ? (merging ? null : mergeWords(node, t))
          : node.status === "planned" ? t("mission.steps.notStarted")
            : node.status === "working" ? t(stageWord(node)) : null;
  return (
    <div data-dependency-node={node.key} className="flex">
      <MissionGraphCard
        node={node} look={look} word={word} yours={yours}
        aria-label={[
          node.title, word, detail, node.worker_agent ? agentProductName(node.worker_agent) : null,
          // The stations are drawn as marks, so their words and states are said here (B28).
          stations ? `${t("mission.station.line")}: ${stationWords(stations, t)}` : null,
          merging ? mergeWords(node, t) : null,
        ].filter(Boolean).join(" · ")}
        aria-disabled={!agent} onClick={agent ? () => openDetail(agent) : undefined}
        data-node-attention={attention?.to}
      >
        {detail ? <span className="truncate text-caption text-muted-foreground" title={detail} data-phone-node-reason="true">{detail}</span> : null}
        {node.worker_agent ? <span className="flex items-center gap-xxs text-caption text-muted-foreground"><AgentMark kind={node.worker_agent} />{agentProductName(node.worker_agent)}</span> : null}
        <HumanStrip gate={gate} merge={merging} node={node} />
        {gate?.state === "passed" && gate.via ? <span className="text-caption text-muted-foreground" data-phone-node-gate-via={gate.via}>{t(`mission.gate.via.${gate.via}`)}{gate.said ? ` · “${gate.said}”` : ""}</span> : null}
        {stations || node.pr !== null || yours || long ? (
          <span className="flex min-w-0 items-center gap-xs text-caption text-muted-foreground" data-node-row="true">
            {/* The stations take the row and their last name truncates, `내 차례` keeping its place at the right (B30); a step with none, such as an outside one from a Mac before them, keeps its pull request's mark. */}
            {stations ? <StationLine stations={stations} className="flex-1" /> : node.pr !== null ? <MissionPrMark number={node.pr} state={node.pr_state} stale={mission.stale} /> : null}
            {yours ? <span className="shrink-0 font-semibold text-mission-hold" data-node-gate-tag={gate?.state}>{t("agents.mark.turn")}</span> : null}
            {long ? <span className="shrink-0" data-node-gate-long="true">· {t("mission.gate.long")}</span> : null}
          </span>
        ) : null}
      </MissionGraphCard>
    </div>
  );
}
