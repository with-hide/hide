import type { ReactNode } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { formatDateTime } from "../i18n/format";
import { requireInterfaceLanguage } from "../i18n/locale";
import { planNowLine } from "../plans";
import type { TFunction } from "i18next";
import { attentionReasonText, cardLine, escalationWhy, holdReason, missionOpen, missionPulls, missionResultCounts, skipWords, stageWord, summaryGroups, type HoldReason } from "../missions";
import { useWorkerEscalation } from "../missionWorkers";
import type { Mission, MissionAttentionTo, MissionNode, MissionSkip } from "../snapshot";
import { formatElapsed, useElapsed } from "./elapsed";
import { MissionPrMark } from "./mission-pr";
import { Button } from "./ui/button";

// The map's right column while no node is picked (PRD mission-tool D-13,
// B19; PRD mission-node-simple B5): an open mission's nodes in progress
// (each with its line, a node waiting on the Conductor saying so first) and
// done, each counted, leaving what waits on the operator to 내 차례 alone; its pull requests and the workers at
// work of the most allowed; a closed one's time and date, its result pull
// requests, and the workers and checkers it took; open or closed, the lines
// its Conductor skipped on its own (`Conductor가 정한 것`). Every value is the
// record's; a row with nothing in it is not drawn. A node named here picks it
// on the map.

export function MissionSummary({ mission, onSelect }: { mission: Mission; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const rows = missionOpen(mission) ? <OpenRows mission={mission} onSelect={onSelect} /> : <ClosedRows mission={mission} />;
  return (
    <section aria-label={t("mission.summary")} className="flex min-w-0 flex-col px-md py-sm" data-mission-side="summary">
      <dl className="flex min-w-0 flex-col gap-md text-body">
        {rows}
        <DecidedRow mission={mission} onSelect={onSelect} />
      </dl>
    </section>
  );
}

/**
 * `Conductor가 정한 것 N`: the lines the Conductor skipped on its own, with
 * no decision of the operator's, oldest first (the principle of 2026-11-11,
 * hide-private docs/missions.md), each the node, `<line> 넘김`, why, who and
 * how long ago. For reading after the fact: nothing here asks anything.
 */
function DecidedRow({ mission, onSelect }: { mission: Mission; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const skips = mission.skipped_alone ?? [];
  if (skips.length === 0) return null;
  return (
    <SummaryRow name={t("mission.decided", { count: skips.length })} field="decided">
      {skips.map((skip) => <SkipLine key={`${skip.node}:${skip.line}`} mission={mission} skip={skip} onSelect={onSelect} />)}
    </SummaryRow>
  );
}

function SkipLine({ mission, skip, onSelect }: { mission: Mission; skip: MissionSkip; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const ago = useElapsed(skip.at_unix_ms);
  const node = mission.nodes.find((one) => one.key === skip.node) ?? null;
  const what = `${skipWords(skip, t)} · ${skip.reason}`;
  return (
    <span className="flex min-w-0 flex-col" data-mission-skip={`${skip.node}:${skip.line}`}>
      {node ? <NodeName node={node} onSelect={onSelect} /> : <span className="truncate">{skip.title}</span>}
      <span className="truncate text-caption text-muted-foreground" title={what}>{what}</span>
      <span className="truncate text-caption text-subtle-foreground">{[skip.by, ago].filter(Boolean).join(" · ")}</span>
    </span>
  );
}

/** One row of a work page's side column: a dim name over its values (a plan's map draws the same rows). */
export function SummaryRow({ name, field, children }: { name: string; field: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-xxs" data-mission-summary-row={field}>
      <dt className="text-caption text-muted-foreground">{name}</dt>
      <dd className="flex min-w-0 flex-col gap-xxs">{children}</dd>
    </div>
  );
}

function OpenRows({ mission, onSelect }: { mission: Mission; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const escalationOf = useWorkerEscalation();
  const { attention, working, done } = summaryGroups(mission, escalationOf);
  const pulls = missionPulls(mission);
  // What waits on the operator stands in 내 차례 above the map and nowhere else here; what waits on the Conductor is work in progress, said in its line.
  const held = attention.flatMap((node) => {
    const reason = holdReason(mission, node, escalationOf);
    const words = reason ? holdWords(reason, t) : null;
    return words?.to === "operator" ? [] : [{ node, words }];
  });
  const going = [...held, ...working.map((node) => ({ node, words: null }))];
  return (
    <>
      {going.length > 0 ? (
        <SummaryRow name={t("mission.summary.working", { count: going.length })} field="working">
          {going.map(({ node, words }) => {
            const line = words ? `${words.who} · ${words.why}` : (cardLine(mission, node, node.plan ? planNowLine(node.plan, t) : null, t)?.text ?? t(stageWord(node)));
            return (
              <span key={node.key} className="flex min-w-0 flex-col">
                <NodeName node={node} onSelect={onSelect} />
                <span className="truncate text-caption text-muted-foreground" title={line} data-mission-summary-line={node.key} data-mission-summary-attention={words?.to}>{line}</span>
              </span>
            );
          })}
        </SummaryRow>
      ) : null}
      {done.length > 0 ? (
        <SummaryRow name={t("mission.summary.done", { count: done.length })} field="done">
          <span className="flex min-w-0 flex-wrap gap-x-sm gap-y-xxs">
            {done.map((node) => <NodeName key={node.key} node={node} onSelect={onSelect} />)}
          </span>
        </SummaryRow>
      ) : null}
      {pulls.length > 0 ? <SummaryRow name={t("mission.field.pr")} field="pr"><Pulls mission={mission} nodes={pulls} /></SummaryRow> : null}
      <SummaryRow name={t("mission.summary.workers")} field="workers">
        <span className="tabular-nums">{t("mission.summary.running", { running: String(mission.running), cap: String(mission.cap) })}</span>
      </SummaryRow>
    </>
  );
}

/**
 * Who a held node waits on and why, as `내 차례` words it: a raised node's
 * own reason, a raised worker's line or cause, and the decision; the last
 * two are the operator's turn.
 */
function holdWords(reason: HoldReason, t: TFunction<"translation">): { to: MissionAttentionTo; who: string; why: string } {
  switch (reason.kind) {
    case "attention":
      return { to: reason.attention.to, who: t(`mission.attention.to.${reason.attention.to}`), why: attentionReasonText(reason.attention.reason, reason.attention.pr, t) };
    case "worker":
      return { to: "operator", who: t("mission.attention.to.operator"), why: escalationWhy(reason.escalation, t) };
    case "decision":
      return { to: "operator", who: t("mission.attention.to.operator"), why: t("missionScreen.pill.decision") };
  }
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
      {took ? <SummaryRow name={t("mission.summary.took")} field="took"><span className="tabular-nums">{took}</span></SummaryRow> : null}
      {pulls.length > 0 ? <SummaryRow name={t("mission.panel.result")} field="result"><Pulls mission={mission} nodes={pulls} /></SummaryRow> : null}
      {agents ? (
        <SummaryRow name={t("mission.summary.workers")} field="agents">
          <span className="tabular-nums">{agents.checkers > 0 ? t("mission.summary.agents", { workers: String(agents.workers), checkers: String(agents.checkers) }) : String(agents.workers)}</span>
        </SummaryRow>
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
