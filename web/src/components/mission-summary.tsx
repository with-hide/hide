import type { ReactNode } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { formatDateTime } from "../i18n/format";
import { requireInterfaceLanguage } from "../i18n/locale";
import { planNowLine } from "../plans";
import { attentionWords, cardLine, missionOpen, missionPulls, missionResultCounts, stageWord, summaryGroups } from "../missions";
import type { Mission, MissionNode } from "../snapshot";
import { formatElapsed } from "./elapsed";
import { MissionPrMark } from "./mission-pr";
import { Button } from "./ui/button";

// The map's right column while no node is picked (PRD mission-tool D-13,
// B19; PRD mission-node-simple B5): an open mission's nodes by the three
// groups the map reads, 확인 필요 first (each with who looks), 진행 (each
// with its line) and 끝, each counted; its pull requests and the workers at
// work of the most allowed; a closed one's time and date, its result pull
// requests, and the workers and checkers it took. Every value is the
// record's; a row with nothing in it is not drawn. A node named here picks it
// on the map.

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
  const { attention, working, done } = summaryGroups(mission);
  const pulls = missionPulls(mission);
  return (
    <>
      {attention.length > 0 ? (
        <Row name={t("mission.summary.attention", { count: attention.length })} field="attention">
          {attention.map((node) => {
            const words = attentionWords(node, t);
            return (
              <span key={node.key} className="flex min-w-0 flex-col">
                <NodeName node={node} onSelect={onSelect} />
                {words ? <span className="truncate text-caption text-mission-hold" title={words.why} data-mission-summary-attention={node.attention?.to}>! {words.who} · {words.why}</span> : null}
              </span>
            );
          })}
        </Row>
      ) : null}
      {working.length > 0 ? (
        <Row name={t("mission.summary.working", { count: working.length })} field="working">
          {working.map((node) => {
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
      {done.length > 0 ? (
        <Row name={t("mission.summary.done", { count: done.length })} field="done">
          <span className="flex min-w-0 flex-wrap gap-x-sm gap-y-xxs">
            {done.map((node) => <NodeName key={node.key} node={node} onSelect={onSelect} />)}
          </span>
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
    <Button variant="text-link" size="inline" onClick={() => onSelect(node.key)} title={node.title} className="block min-w-0 shrink truncate text-left text-inherit" data-mission-summary-node={node.key}>
      {node.title}
    </Button>
  );
}

function Pulls({ mission, nodes }: { mission: Mission; nodes: ReturnType<typeof missionPulls> }) {
  return (
    <span className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs">
      {nodes.map(({ node, pr }) => <MissionPrMark key={pr.number} number={pr.number} state={pr.state} stale={mission.github_stale} title={node.title} data-mission-summary-pr={pr.number} />)}
    </span>
  );
}
