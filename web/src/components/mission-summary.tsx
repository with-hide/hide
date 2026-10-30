import type { ReactNode } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { formatDateTime } from "../i18n/format";
import { requireInterfaceLanguage } from "../i18n/locale";
import { planNowLine } from "../plans";
import { cardLine, liveNodes, missionOpen, missionPulls, missionResultCounts, nextNodes, stageWord } from "../missions";
import type { Mission, MissionNode } from "../snapshot";
import { formatElapsed } from "./elapsed";
import { MissionPrMark } from "./mission-pr";

// The map's right column while no node is picked (PRD mission-tool D-13,
// B19): an open mission's 지금 (each node a worker is at, with its line),
// 다음 (what can start next), its pull requests and the workers at work of
// the most allowed; a closed one's time and date, its result pull requests,
// and the workers and checkers it took. Every value is the record's; a row
// with nothing in it is not drawn. A node named here picks it on the map.

export function MissionSummary({ mission, onSelect }: { mission: Mission; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const rows = missionOpen(mission) ? <OpenRows mission={mission} onSelect={onSelect} /> : <ClosedRows mission={mission} />;
  return (
    <section aria-label={t("mission.summary")} className="flex min-w-0 flex-col px-md py-sm" data-mission-side="summary">
      <dl className="flex min-w-0 flex-col gap-md text-body">{rows}</dl>
    </section>
  );
}

function Row({ name, field, children }: { name: string; field: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-xxs" data-mission-summary-row={field}>
      <dt className="text-caption text-muted-foreground">{name}</dt>
      <dd className="flex min-w-0 flex-col gap-xxs">{children}</dd>
    </div>
  );
}

function OpenRows({ mission, onSelect }: { mission: Mission; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const live = liveNodes(mission);
  const next = nextNodes(mission);
  const pulls = missionPulls(mission);
  return (
    <>
      {live.length > 0 ? (
        <Row name={t("mission.field.now")} field="now">
          {live.map((node) => {
            const line = cardLine(mission, node, node.plan ? planNowLine(node.plan, t) : null, t)?.text ?? t(stageWord(node));
            return (
              <span key={node.key} className="flex min-w-0 flex-col">
                <NodeName node={node} onSelect={onSelect} />
                <span className="truncate text-caption text-muted-foreground" title={line} data-mission-summary-line={node.key}>{line}</span>
              </span>
            );
          })}
        </Row>
      ) : null}
      {next.length > 0 ? (
        <Row name={t("mission.summary.next")} field="next">
          {next.map((node) => <NodeName key={node.key} node={node} onSelect={onSelect} />)}
        </Row>
      ) : null}
      {pulls.length > 0 ? <Row name={t("mission.field.pr")} field="pr"><Pulls mission={mission} nodes={pulls} /></Row> : null}
      <Row name={t("mission.summary.workers")} field="workers">
        <span className="tabular-nums">{t("mission.summary.running", { running: String(mission.running), cap: String(mission.cap) })}</span>
      </Row>
    </>
  );
}

function ClosedRows({ mission }: { mission: Mission }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const pulls = missionPulls(mission);
  const agents = missionResultCounts(mission).agents;
  const start = mission.started_at_unix_ms;
  const end = mission.closed_at_unix_ms;
  const took = start != null && end != null ? [formatElapsed(language, end - start), formatDateTime(language, end, { month: "2-digit", day: "2-digit" })].join(" · ") : null;
  return (
    <>
      {took ? <Row name={t("mission.summary.took")} field="took"><span className="tabular-nums">{took}</span></Row> : null}
      {pulls.length > 0 ? <Row name={t("mission.panel.result")} field="result"><Pulls mission={mission} nodes={pulls} /></Row> : null}
      {agents ? (
        <Row name={t("mission.summary.workers")} field="agents">
          <span className="tabular-nums">{agents.checkers > 0 ? t("mission.summary.agents", { workers: String(agents.workers), checkers: String(agents.checkers) }) : String(agents.workers)}</span>
        </Row>
      ) : null}
    </>
  );
}

function NodeName({ node, onSelect }: { node: MissionNode; onSelect: (key: string) => void }) {
  return (
    <button type="button" onClick={() => onSelect(node.key)} title={node.title} className="min-w-0 truncate rounded-xs text-left outline-none hover:underline focus-visible:ring-1 focus-visible:ring-ring" data-mission-summary-node={node.key}>
      {node.title}
    </button>
  );
}

function Pulls({ mission, nodes }: { mission: Mission; nodes: MissionNode[] }) {
  return (
    <span className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs">
      {nodes.map((node) => <MissionPrMark key={node.key} number={node.pr!} state={node.pr_state} stale={mission.github_stale} title={node.title} data-mission-summary-pr={node.pr!} />)}
    </span>
  );
}
