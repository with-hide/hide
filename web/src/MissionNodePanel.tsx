import { ArrowRightIcon, ChevronRightIcon, GitBranchIcon, HistoryIcon, ScrollTextIcon, SquareTerminalIcon } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { agentMarkOf } from "./agentLogos";
import { CheckerPopover, useCheckerStatus, VerdictReason, VERDICT_TONE } from "./components/mission-checker";
import { CriterionMark } from "./components/criterion-mark";
import { MapPanel, MapPanelHead } from "./components/map-page";
import { MissionNodeMark } from "./components/mission-node-mark";
import { MissionPrMark } from "./components/mission-pr";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { formatDateTime } from "./i18n/format";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { CheckerRow, markWord, MissionDevice, RunText, useMissionDeviceName, useNodeRun, useNodeTimes, useWorkerRow } from "./missionParts";
import {
  attentionWords,
  cardLine,
  checkerState,
  criteriaTally,
  criterionName,
  criterionNote,
  eventWords,
  hideWords,
  landingLeft,
  lineOutrun,
  mainCommit,
  missionOpen,
  nodeLook,
  nodeQuestion,
  nodeSteps,
  nodeWork,
  nodeResumable,
  openAttention,
  ownCriteria,
  shortCommit,
} from "./missions";
import { AttentionChoices } from "./components/mission-attention";
import { PILL_TONE, ProgressCells } from "./components/mission-tone";
import { PlanChecklist, PlannedSteps } from "./components/plan-checklist";
import { planCells, planCount } from "./plans";
import { formatBytes } from "./i18n/format";
import { sessionOfAgent } from "./sessionWork";
import type { MessageKey } from "./i18n/catalogs";
import type { Mission, MissionChecker, MissionCriterion, MissionEvent, MissionNode, PlanView } from "./snapshot";
import { useShellStore } from "./store";

// The node the operator picked on a mission's map (PRD mission-map-git-criteria
// D-01, D-10, B31-B33), read top down: a head that stays while the panel
// scrolls (its mark, title, the ways to its worker's terminal and record, and
// one line of who works it, how long it took and its branch), what waits on
// someone, its worker's plan, what it made, its finish lines as one list, the
// nodes before and after it as one line, and its events folded.

export function MissionNodePanel({ mission, node, events, end, onSelect, onClose, actions }: { mission: Mission; node: MissionNode; events: MissionEvent[] | null; end: number; onSelect: (key: string) => void; onClose: () => void; actions: Actions }) {
  return (
    <MapPanel data={{ "data-mission-detail": node.key }}>
      <Head mission={mission} node={node} events={events} end={end} onClose={onClose} actions={actions} />
      <Attention mission={mission} node={node} actions={actions} />
      <Now mission={mission} node={node} events={events} />
      <Plan mission={mission} node={node} actions={actions} />
      <Made mission={mission} node={node} />
      <Criteria mission={mission} node={node} actions={actions} />
      <Links mission={mission} node={node} onSelect={onSelect} />
      <Ways mission={mission} node={node} actions={actions} />
      <HideRecord mission={mission} node={node} events={events} />
      <Events node={node} events={events} />
      <Brief key={node.key} mission={mission} node={node} actions={actions} />
    </MapPanel>
  );
}

/**
 * The panel's head, which stays at the top while the panel scrolls: the
 * node's mark (its state word on hover and in its name), its title, `터미널`
 * and `기록` beside ✕, and one line of facts: the worker's agent mark, model
 * and effort, how long it took, its attempt, how often a defect reopened it,
 * `↻ 재계획` with the replan's reason on hover, the worker's sleep or end, and
 * its branch.
 */
function Head({ mission, node, events, end, onClose, actions }: { mission: Mission; node: MissionNode; events: MissionEvent[] | null; end: number; onClose: () => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const look = nodeLook(mission, node);
  const worker = useNodeRun(node);
  const device = useMissionDeviceName(node.machine);
  const times = useNodeTimes(mission, node, events, end);
  const facts = [
    ...(times?.took ? [t("mission.elapsed.total", { elapsed: times.took })] : []),
    ...(node.attempts >= 2 ? [t("mission.badge.attempts", { count: node.attempts })] : []),
    ...(node.reopened > 0 ? [t("mission.badge.reopened", { count: node.reopened })] : []),
  ];
  const replan = node.change && missionOpen(mission) ? `${t("mission.changed")} · ${node.change}` : null;
  // The worker's life beside its model (B4): asleep or ended, as `hide agent show` says it.
  const life = node.agent_runtime === "asleep" || node.agent_runtime === "ended" ? t(node.agent_runtime === "asleep" ? "mission.worker.asleep" : "mission.worker.ended") : null;
  const branch = node.branch ?? (node.external && "branch" in node.external ? node.external.branch : null);
  const kind = KIND_SHORT[node.kind];
  const parts: ReactNode[] = [
    ...(kind ? [<span key="kind" className="shrink-0 font-semibold text-subtle-foreground" data-panel-kind={node.kind}>{t(kind)}</span>] : []),
    ...(worker ? [<span key="run" className="flex min-w-0 items-center gap-xxs">{worker.agent ? <AgentMark kind={worker.agent} /> : null}<RunText run={worker.run} /></span>] : []),
    ...(device ? [<MissionDevice key="device" name={device} />] : []),
    ...facts.map((fact) => <span key={fact} className="shrink-0 tabular-nums" data-panel-fact="true">{fact}</span>),
    ...(replan
      ? [
          <Hint key="replan" label={replan}>
            <span tabIndex={0} aria-label={replan} className="flex shrink-0 items-center gap-xxs rounded-xs text-mission-hold outline-none focus-visible:ring-1 focus-visible:ring-ring" data-panel-replan="true">
              <HistoryIcon aria-hidden="true" className="size-(--size-icon-sm)" />
              {t("mission.panel.replanned")}
            </span>
          </Hint>,
        ]
      : []),
    ...(life ? [<span key="life" className="shrink-0" data-panel-worker-life={node.agent_runtime}>{life}</span>] : []),
    ...(branch
      ? [
          <span key="branch" className="flex min-w-0 items-center gap-xxs font-mono" title={branch}>
            <GitBranchIcon aria-hidden="true" className="size-(--size-pr-icon) shrink-0" />
            <span className="min-w-0 truncate" data-panel-branch={branch}>{branch}</span>
          </span>,
        ]
      : []),
  ];
  return (
    <MapPanelHead
      mark={<MissionNodeMark look={look} label={markWord(node, look, t)} still={(nodeWork(mission, node)?.state ?? "working") !== "working"} />}
      title={node.title}
      ways={<WorkerWays mission={mission} node={node} actions={actions} />}
      close={t("mission.panel.close")}
      onClose={onClose}
      facts={parts.length > 0 ? parts : null}
    />
  );
}

/** A node's kind as the head's first fact; a change, the usual kind, says nothing. */
const KIND_SHORT: Record<MissionNode["kind"], MessageKey | null> = {
  change: null,
  research: "mission.kind.research",
  verify: "mission.panel.kindVerify",
  external: "mission.kind.external",
};

/**
 * The head's ways out (B31): the node's current pull request as its PR mark
 * (state icon and `#n`), which opens its PR card, then `터미널` (the worker's
 * terminal while its row is shown) and `기록`, their icons alone in a panel
 * narrower than `--size-mission-panel-words` so a long title keeps its room;
 * an outside node has none.
 */
function WorkerWays({ mission, node, actions }: { mission: Mission; node: MissionNode; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const row = useWorkerRow(node);
  // Derived outside the selector: it builds a new session each call, which a selector must not return.
  const rest = useShellStore((s) => s.rest);
  const session = useMemo(() => (row ? sessionOfAgent(row) : null), [row, rest]);
  if (node.kind === "external") return null;
  const card = node.pr != null ? `${t("mission.prCard")} · #${node.pr}` : null;
  return (
    <>
      {node.pr != null && card ? (
        <Hint label={card}>
          <Button size="sm" variant="secondary" className="shrink-0 px-xs" aria-label={card} disabled={!node.pr_url} onClick={() => node.pr_url && actions.openSessionPullRequest({ workspace_id: mission.workspace_id, url: node.pr_url, number: node.pr! }, "here")} data-node-pr-card={node.pr}>
            <MissionPrMark number={node.pr} state={node.pr_state} stale={mission.github_stale} />
          </Button>
        </Hint>
      ) : null}
      <Hint label={t("mission.toTerminal")}>
        <Button size="sm" variant="secondary" className="shrink-0 px-xs mission-panel-words:px-sm" aria-label={t("mission.toTerminal")} disabled={!node.pane_id || !row} onClick={() => node.pane_id && actions.openAgent(node.pane_id)} data-node-terminal="true">
          <SquareTerminalIcon aria-hidden="true" className="mission-panel-words:hidden" />
          <span className="hidden mission-panel-words:inline">{t("mission.panel.terminal")}</span>
        </Button>
      </Hint>
      <Hint label={t("mission.record")}>
        <Button size="sm" variant="secondary" className="shrink-0 px-xs mission-panel-words:px-sm" aria-label={t("mission.record")} disabled={!session} onClick={() => session && actions.openChildRecord(session)} data-node-record="true">
          <ScrollTextIcon aria-hidden="true" className="mission-panel-words:hidden" />
          <span className="hidden mission-panel-words:inline">{t("mission.record")}</span>
        </Button>
      </Hint>
    </>
  );
}

function Part({ title, children, part }: { title: ReactNode; children: ReactNode; part: string }) {
  return (
    <section className="flex min-w-0 flex-col gap-xs" data-panel-part={part}>
      <h4 className="text-caption text-muted-foreground">{title}</h4>
      {children}
    </section>
  );
}

/**
 * What waits on someone to look at the node (PRD mission-node-simple B2,
 * B15-B17): who looks, why, and the operator's choices for that reason, which
 * the operator may take before the Conductor does; each settles it with one
 * event.
 */
function Attention({ mission, node, actions }: { mission: Mission; node: MissionNode; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const words = attentionWords(node, t);
  if (!words || !node.attention || !openAttention(mission, node)) return null;
  return (
    <Part title={t("mission.attention")} part="attention">
      <p className="min-w-0 break-words border-l-2 border-mission-hold pl-sm" data-panel-attention={node.attention.reason} data-panel-attention-to={node.attention.to}>
        <span className="font-semibold text-mission-hold">! {words.who}</span>
        <span className="text-muted-foreground"> · </span>
        {words.why}
      </p>
      <AttentionChoices key={`${node.key}-${node.attention.since_unix_ms}`} mission={mission} node={node} attention={node.attention} actions={actions} />
    </Part>
  );
}

/**
 * Where the node is now (B4, B31), under no heading of its own: the
 * decision's question and choices while it names the node, its checkers at
 * work, else what it waits on; and the Conductor's last line beside a
 * question, finished work and a dropped or cancelled node too, dimmed once
 * Hide's facts moved past it (PRD mission-node-simple B4). A worker's plan
 * says where it is, so beside one the Conductor's line is left to the record.
 */
function Now({ mission, node, events }: { mission: Mission; node: MissionNode; events: MissionEvent[] | null }) {
  const { t } = useInterfaceTranslation();
  const look = nodeLook(mission, node);
  const ended = look === "cancelled" || look === "dropped";
  const question = ended ? null : nodeQuestion(mission, node);
  const options = question ? (mission.question?.options ?? []) : [];
  const checks = ended || question || look === "done" ? [] : node.criteria.filter((line) => (line.kind === "review" || line.kind === "verify") && line.state === "open" && (line.checkers?.length ?? 0) > 0);
  // Beside a question, finished work or an ended node only the Conductor's own line is left to say; a done research node's line is its conclusion, which the result shows.
  const own = node.line && !node.plan && !(node.kind === "research" && look === "done") ? { text: node.line, attention: false } : null;
  const line = ended || question || look === "done" || node.plan ? own : cardLine(mission, node, null, t);
  if (!question && !line && checks.length === 0) return null;
  // The Conductor's line is its own word; once Hide's facts moved past it, it stays readable but dimmed (B4).
  const behind = line !== null && line.text === node.line && lineOutrun(node, events);
  return (
    <section className="flex min-w-0 flex-col gap-xs" data-panel-part="now">
      {question ? <p className="min-w-0 break-words border-l-2 border-mission-hold pl-sm" data-panel-question="true">{question}</p> : null}
      {options.length > 0 ? (
        <ul className="flex min-w-0 flex-wrap gap-xs text-caption" data-panel-options={options.length}>
          {options.map((option, index) => (
            <li key={index} className="min-w-0 rounded-xs bg-muted px-xs">
              <span className="break-words">{option}</span>
              {mission.question?.recommend === index + 1 ? <span className="ml-xxs text-mission-hold">{t("mission.banner.recommended")}</span> : null}
            </li>
          ))}
        </ul>
      ) : null}
      {line ? (
        <p className={cn("min-w-0 break-words", line.attention && "text-mission-failed", behind && "text-muted-foreground opacity-(--opacity-dimmed)")} title={behind ? t("mission.line.behind") : undefined} data-panel-line-behind={behind || undefined} data-panel-conductor-line={line.text === node.line || undefined}>
          {line.text}
        </p>
      ) : null}
      {checks.map((check) => (
        <div key={check.key} className="flex min-w-0 flex-col gap-xxs text-caption" data-panel-checkers={check.kind}>
          {(check.checkers ?? []).map((checker, index) => <CheckerRow key={index} mission={mission} kind={check.kind as "review" | "verify"} checker={checker} />)}
        </div>
      ))}
    </section>
  );
}

/**
 * How the node's work goes, `작업 방식`: its worker's own plan once it exists
 * (PRD hide-plan D-17, D-18), `n/m`, a cell per step and `크게 ›`, which opens
 * the plan's map in the 보기 slot as the pane chip's does, over the
 * checklist, with `바뀜` on a step the worker changed from the Conductor's;
 * else the steps the Conductor planned, as a list of steps not begun. A
 * finished, dropped or cancelled node keeps the plan its worker left, folded
 * under `펼치기`.
 */
function Plan({ mission, node, actions }: { mission: Mission; node: MissionNode; actions: Actions }) {
  const look = nodeLook(mission, node);
  const work = nodeSteps(node);
  if (!work) return null;
  const ended = look === "done" || look === "dropped" || look === "cancelled";
  // Picking another node gives a new fold, so each finished node opens folded.
  return node.plan ? (
    <PlanPart key={node.key} plan={node.plan} planned={node.steps} folded={ended} actions={actions} />
  ) : (
    <PlannedPart steps={node.steps} />
  );
}

/** The Conductor's steps before the worker has put its own plan up: a list of steps not begun, under the count. */
function PlannedPart({ steps }: { steps: readonly string[] }) {
  const { t } = useInterfaceTranslation();
  return (
    <section className="flex min-w-0 flex-col gap-xs" data-panel-part="plan" data-panel-plan="planned">
      <div className="flex min-w-0 items-center gap-xs text-caption text-muted-foreground">
        <h4 className="shrink-0">{t("mission.panel.approach")}</h4>
        <span className="shrink-0" data-panel-plan-count="planned">{t("mission.steps.planned", { count: steps.length })}</span>
      </div>
      <div className="min-w-0 text-caption"><PlannedSteps steps={steps} /></div>
    </section>
  );
}

function PlanPart({ plan, planned, folded, actions }: { plan: PlanView; planned: readonly string[]; folded: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(!folded);
  return (
    <section className="flex min-w-0 flex-col gap-xs" data-panel-part="plan" data-panel-plan={open ? "open" : "folded"}>
      <div className="flex min-w-0 items-center gap-xs text-caption text-muted-foreground">
        <h4 className="shrink-0">{t("mission.panel.approach")}</h4>
        <ProgressCells tones={planCells(plan)} size="chip" />
        <span className="shrink-0 font-mono tabular-nums" data-panel-plan-count="true">{planCount(plan)}</span>
        {folded ? (
          <Button variant="text" size="inline" className="text-caption" aria-expanded={open} onClick={() => setOpen(!open)} data-panel-plan-toggle="true">
            {t(open ? "mission.report.collapse" : "mission.report.expand")}
            <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon-sm) transition-transform", open && "rotate-90")} />
          </Button>
        ) : null}
        <Button variant="text" size="inline" className="ml-auto shrink-0 text-caption" onClick={() => actions.openPlan(plan.session, "here")} data-panel-plan-large="true">
          {t("plan.large")}
          <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />
        </Button>
      </div>
      {open ? <div className="min-w-0 text-caption"><PlanChecklist plan={plan} planned={planned} /></div> : null}
    </section>
  );
}

/**
 * What the node made (B31): its earlier pull requests with their states and
 * the base its current one stacks on, the CI line, its commit on main, a
 * research conclusion or a verify node's verdict; its current pull request
 * and its branch are the head's. Nothing the record does not hold.
 */
function Made({ mission, node }: { mission: Mission; node: MissionNode }) {
  const { t } = useInterfaceTranslation();
  const ci = node.criteria.find((line) => line.kind === "ci");
  const rows: [string, ReactNode][] = [];
  const outside = node.external && "pr" in node.external ? [{ number: node.external.pr, state: node.pr_state, url: node.pr_url }] : [];
  // The current pull request is the head's button; the earlier ones and the base it stacks on stay here. An outside node has no head button.
  const prs = node.prs.length > 0 ? node.prs.filter((pr) => node.kind === "external" || pr.number !== node.pr) : outside;
  if (prs.length > 0 || node.base_pr != null) {
    rows.push([
      t("mission.field.pr"),
      <span key="pr" className="flex min-w-0 flex-wrap items-center gap-xs" data-panel-prs={prs.map((pr) => pr.number).join(" ")}>
        {prs.map((pr) => <MissionPrMark key={pr.number} number={pr.number} state={pr.state} stale={mission.github_stale} />)}
        {node.base_pr != null ? <span className="text-caption text-muted-foreground" data-panel-base={node.base_pr}>{t("mission.panel.base", { number: String(node.base_pr) })}</span> : null}
      </span>,
    ]);
  }
  if (ci) rows.push([t("mission.panel.ci"), <LineState key="ci" mission={mission} line={ci} />]);
  const commit = mainCommit(node);
  // What a finished change still waits on before main, as the core decides it (B27): the card leaves it to the panel (D-07).
  const left = landingLeft(node);
  if (commit) rows.push(["main", <span key="commit" className="font-mono text-caption" title={commit} data-panel-commit={commit}>{t("mission.result.main", { commit: shortCommit(commit) })}</span>]);
  else if (left) rows.push(["main", <span key="left" className="font-semibold text-mission-hold" data-panel-landing={left}>{t(left === "merge" ? "mission.landing.merge" : "mission.landing.join")}</span>]);
  if (node.kind === "research" && node.status === "done" && node.line) rows.push([t("mission.result.conclusion"), <span key="c" className="break-words" data-panel-conclusion="true">{node.line}</span>]);
  if (node.kind === "verify" && node.verdict) {
    rows.push([
      t("mission.result.verify"),
      <span key="v" className={cn("font-mono font-semibold", node.verdict === "fail" && "text-mission-failed")} data-panel-verdict={node.verdict}>
        {t(node.verdict === "pass" ? "mission.result.pass" : "mission.result.fail")}
        {node.checks ? ` ${node.checks.passed}/${node.checks.total}` : ""}
      </span>,
    ]);
  }
  if (rows.length === 0) return null;
  return (
    <Part title={t("mission.panel.result")} part="result">
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-md gap-y-xxs text-caption">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-muted-foreground">{label}</dt>
            <dd className="min-w-0">{value}</dd>
          </div>
        ))}
      </dl>
    </Part>
  );
}

/** A line's mark and its note, as the criteria list and the CI row show it. */
function LineState({ mission, line }: { mission: Mission; line: MissionCriterion }) {
  const { t } = useInterfaceTranslation();
  const note = criterionNote(line, mission, t);
  return (
    <span className="flex min-w-0 items-baseline gap-xs">
      <CriterionMark state={line.state} />
      {note ? <span className={cn("min-w-0 break-words", line.state === "skipped" ? "text-mission-hold" : "text-muted-foreground")}>{note}</span> : null}
    </span>
  );
}

/** Who decides a finish line, as the small tag before its words. */
const TAG: Record<MissionCriterion["kind"], MessageKey> = {
  own: "mission.criteria.tag.own",
  defect: "mission.criteria.tag.own",
  check: "mission.criteria.tag.check",
  ci: "mission.criteria.tag.ci",
  review: "mission.criteria.tag.review",
  verify: "mission.criteria.tag.verify",
};

/**
 * The finish lines (B32) as one list in the order the record keeps them:
 * each line is ✓ ○ ⤳ ✕, a small tag of who decides it (`일`, `검사`, `CI`,
 * `리뷰`, `검증`) and its words on one line, the whole text and its evidence
 * on hover; a review or verification ends in a small chip per checker that
 * opens its answer and the way to its terminal or record. A skipped or failed
 * line carries its verdict under it, the short reason first and the full
 * text folded. A mission from before criteria shows its sentence (B23).
 */
function Criteria({ mission, node, actions }: { mission: Mission; node: MissionNode; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  if (mission.exempt) {
    const before = ownCriteria(node);
    return before ? (
      <Part title={t("mission.panel.oldCriteria")} part="criteria">
        <p className="min-w-0 break-words" data-panel-old-criteria="true">{before}</p>
      </Part>
    ) : null;
  }
  const tally = criteriaTally(mission, node);
  if (!tally) return null;
  return (
    <Part title={<>{t("mission.field.criteria")} <span className="ml-xs font-mono tabular-nums">{tally.passed}/{tally.total}</span></>} part="criteria">
      <ul className="flex min-w-0 flex-col gap-xxs" data-panel-criteria={node.criteria.map((line) => `${line.key}:${line.state}`).join(" ")}>
        {node.criteria.map((line) => <CriterionLine key={line.key} mission={mission} line={line} actions={actions} />)}
      </ul>
    </Part>
  );
}

function CriterionLine({ mission, line, actions }: { mission: Mission; line: MissionCriterion; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const name = criterionName(line, t);
  // A skipped line's reason stands in its verdict, not after its words.
  const note = line.state === "skipped" ? null : criterionNote(line, mission, t);
  const checks = line.kind === "review" || line.kind === "verify" ? { kind: line.kind, checkers: line.checkers ?? [] } : null;
  const whole = [name, note, line.evidence].filter(Boolean).join("\n\n");
  return (
    <li className="grid min-w-0 grid-cols-[var(--size-agent-mark)_minmax(0,1fr)] items-center gap-x-xs gap-y-xxs">
      <div className="contents" data-panel-line={line.key} data-line-state={line.state} data-line-kind={line.kind}>
        <CriterionMark state={line.state} />
        <span className="flex min-w-0 items-center gap-xs">
          <Badge variant="outline" data-line-tag={line.kind}>{t(TAG[line.kind])}</Badge>
          <span className="min-w-0 flex-1 truncate" title={whole}>{name}</span>
          {/* The note truncates too, at half the line, so the line's own words keep their place. */}
          {note ? <span className="min-w-0 max-w-1/2 truncate text-caption text-muted-foreground" data-line-note="true">{note}</span> : null}
          {checks && checks.checkers.length > 0 ? (
            <span className="flex shrink-0 items-center gap-xxs" data-panel-checker-chips={checks.checkers.length}>
              {checks.checkers.map((checker, index) => <CheckerChip key={index} mission={mission} kind={checks.kind} checker={checker} actions={actions} />)}
            </span>
          ) : null}
        </span>
      </div>
      {line.state === "skipped" && line.skip ? (
        <Verdict tone="skipped" word={t("mission.verdict.skipped")}>
          <p className="min-w-0 break-words text-foreground" data-verdict-summary="true">{criterionNote(line, mission, t)}</p>
        </Verdict>
      ) : null}
      {line.state === "fail" && !checks ? (
        <Verdict tone="fail" word={t("mission.verdict.failed")}>
          <VerdictReason summary={line.summary} evidence={line.evidence} />
        </Verdict>
      ) : null}
      {checks
        ? checks.checkers.filter((checker) => checker.state === "fail").map((checker, index) => (
            <Verdict key={index} tone="fail" word={t("mission.verdict.rejected")} who={checker.worker.agent}>
              <VerdictReason summary={checker.summary} evidence={checker.evidence} />
            </Verdict>
          ))
        : null}
    </li>
  );
}

/** A skipped or rejected line's verdict, under its words: its word in its colour, who gave it, and why. */
function Verdict({ tone, word, who, children }: { tone: "fail" | "skipped"; word: string; who?: string; children: ReactNode }) {
  return (
    <div className={cn("col-start-2 flex min-w-0 flex-col gap-xxs rounded-xs border-l-2 bg-secondary px-sm py-xs text-caption", tone === "fail" ? "border-mission-failed" : "border-mission-hold")} data-panel-verdict={tone}>
      <span className="flex min-w-0 items-center gap-xs">
        <span className={cn("shrink-0 rounded-full px-xs text-micro font-semibold", tone === "fail" ? PILL_TONE.failed : PILL_TONE.hold)}>{word}</span>
        {who ? <span className="flex min-w-0 items-center gap-xxs text-muted-foreground"><AgentMark kind={who} /><span className="truncate">{agentMarkOf(who)?.label ?? who}</span></span> : null}
      </span>
      {children}
    </div>
  );
}

/** A checker of a review or verification as a small chip, its agent's mark and its answer, its name on hover; pressing it opens its popover (B17, B32). */
function CheckerChip({ mission, kind, checker, actions }: { mission: Mission; kind: "review" | "verify"; checker: MissionChecker; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const state = checkerState(checker);
  const status = useCheckerStatus(kind, checker);
  const name = agentMarkOf(checker.worker.agent)?.label ?? checker.worker.agent;
  const word = state === "fail" ? t("mission.verdict.rejected") : status;
  return (
    <CheckerPopover mission={mission} kind={kind} checker={checker} actions={actions} hint={`${name} · ${status}`}>
      <Button variant="secondary" size="sm" className="h-(--size-badge-height) gap-xxs rounded-full px-xs text-micro" aria-label={`${name} · ${word}`} data-panel-checker={checker.worker.agent} data-checker-state={state}>
        <AgentMark kind={checker.worker.agent} />
        <span className={cn("font-semibold", VERDICT_TONE[state])}>{word}</span>
      </Button>
    </CheckerPopover>
  );
}

/**
 * The nodes it waits on, this node and the nodes that wait on it, as one
 * line of flow (PRD mission-tool B18): each by name with its pull request's
 * mark, its accessible name saying which side it is on; pressing one picks
 * it (B33). A node with neither has no line.
 */
function Links({ mission, node, onSelect }: { mission: Mission; node: MissionNode; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const before = node.depends_on.flatMap((key) => mission.nodes.find((other) => other.key === key) ?? []);
  const after = mission.nodes.filter((other) => other.depends_on.includes(node.key));
  if (before.length === 0 && after.length === 0) return null;
  const arrow = <ArrowRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />;
  const pills = (nodes: MissionNode[], which: "before" | "after") => (
    <span className="flex min-w-0 flex-wrap items-center gap-xs" data-panel-links={which}>
      {nodes.map((other) => {
        const pr = other.pr ?? (other.external && "pr" in other.external ? other.external.pr : null);
        return (
          <button key={other.key} type="button" onClick={() => onSelect(other.key)} title={other.title} aria-label={`${t(which === "before" ? "mission.panel.waitsOn" : "mission.panel.waitedBy")} · ${other.title}`} className="flex min-w-0 max-w-full items-center gap-xxs rounded-xs border border-border px-xs text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring" data-panel-link={other.key}>
            <span className="min-w-0 truncate">{other.title}</span>
            {pr != null ? <MissionPrMark number={pr} state={other.pr_state} stale={mission.github_stale} /> : null}
          </button>
        );
      })}
    </span>
  );
  return (
    <Part title={t("mission.panel.links")} part="links">
      <div className="flex min-w-0 flex-wrap items-center gap-xs text-caption" data-panel-flow="true">
        {before.length > 0 ? <>{pills(before, "before")}{arrow}</> : null}
        <span className="shrink-0 rounded-xs border border-mission-working px-xs text-foreground" data-panel-flow-self="true">{t("mission.panel.thisNode")}</span>
        {after.length > 0 ? <>{arrow}{pills(after, "after")}</> : null}
      </div>
    </Part>
  );
}

/** `다시 진행` on a done or dropped node (B16); the pull request, terminal and record are the head's. */
function Ways({ mission, node, actions }: { mission: Mission; node: MissionNode; actions: Actions }) {
  if (node.kind === "external" || !nodeResumable(mission, node)) return null;
  return (
    <div className="flex flex-wrap gap-xs" data-panel-part="ways">
      <ResumeNode mission={mission} node={node} actions={actions} />
    </div>
  );
}

/** 다시 진행 on a done or dropped node (B16): one event, and the button stays off until the node moves, so a second press sends nothing. */
function ResumeNode({ mission, node, actions }: { mission: Mission; node: MissionNode; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [sent, setSent] = useState(false);
  return (
    <Button size="sm" variant="secondary" disabled={sent} onClick={() => { setSent(true); actions.settleMissionNode(mission.id, node.key, "resume"); }} data-node-resume="true">
      {t("mission.operator.resume")}
    </Button>
  );
}

/** What Hide did to the node and what it raised, newest last, from its events, else the last mark the record keeps (B3, B4). */
function HideRecord({ mission, node, events }: { mission: Mission; node: MissionNode; events: MissionEvent[] | null }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const own = events?.filter((event) => event.node === node.key && HIDE_KINDS.has(event.kind)) ?? [];
  const mark = hideWords(mission, node, t);
  if (own.length === 0 && !mark) return null;
  const time = (at: number) => formatDateTime(language, at, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });
  return (
    <Part title={t("mission.hide.title")} part="hide">
      {own.length > 0 ? (
        <ol className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-md gap-y-xxs text-caption" data-panel-hide-events={own.length}>
          {own.map((event, index) => {
            const words = eventWords(event, t);
            return (
              <li key={index} className="contents" data-panel-hide-event={event.kind}>
                <span className="text-muted-foreground tabular-nums">{time(event.at_unix_ms)}</span>
                <span className="min-w-0 break-words">
                  <span className="font-semibold">{words.what}</span>
                  {words.line ? <span className="text-muted-foreground"> · {words.line}</span> : null}
                </span>
              </li>
            );
          })}
        </ol>
      ) : (
        <p className="min-w-0 break-words text-caption" data-panel-hide-mark={node.hide?.action}>{t("mission.hide.mark", { what: mark ?? "" })}</p>
      )}
    </Part>
  );
}

/** The events Hide's record lists: what Hide did, what it raised and what the operator settled (D-15). */
const HIDE_KINDS: ReadonlySet<MissionEvent["kind"]> = new Set(["hide", "hide_closed", "raised", "raised_to_operator", "resolved"]);

/** The node's events in order, folded under their count (B31). */
function Events({ node, events }: { node: MissionNode; events: MissionEvent[] | null }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const [open, setOpen] = useState(false);
  const own = events?.filter((event) => event.node === node.key) ?? [];
  if (own.length === 0) return null;
  const time = (at: number) => formatDateTime(language, at, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });
  return (
    <section className="flex min-w-0 flex-col gap-xs" data-panel-part="events" data-panel-events={open ? "open" : "folded"}>
      <button type="button" aria-expanded={open} onClick={() => setOpen(!open)} className="flex w-fit items-center gap-xxs rounded-xs text-caption text-muted-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring" data-panel-events-toggle="true">
        {t("mission.panel.events", { count: own.length })}
        <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon-sm) transition-transform", open && "rotate-90")} />
      </button>
      {open ? (
        <ol className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-md gap-y-xxs text-caption" data-flow-events={own.length}>
          {own.map((event, index) => {
            const words = eventWords(event, t);
            return (
              <li key={index} className="contents" data-panel-event={event.kind}>
                <span className="text-muted-foreground tabular-nums">{time(event.at_unix_ms)}</span>
                <span className="min-w-0 break-words">
                  <span className="font-semibold">{words.what}</span>
                  {words.line ? <span className="text-muted-foreground"> · {words.line}</span> : null}
                </span>
              </li>
            );
          })}
        </ol>
      ) : null}
    </section>
  );
}

/**
 * The brief its worker was started on, the panel's last part and folded
 * until the operator opens it: `지시 원문 · 2.4 KB`. The map carries only the
 * size, so opening asks the core for the text, which arrives for this node
 * alone. Nothing is drawn for a node whose worker was never started.
 */
function Brief({ mission, node, actions }: { mission: Mission; node: MissionNode; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const brief = useShellStore((s) => s.rest?.mission_brief);
  if (node.brief_bytes == null) return null;
  const text = open && brief?.mission === mission.id && brief.node === node.key ? brief.text : null;
  const size = formatBytes(requireInterfaceLanguage(i18n.language), node.brief_bytes);
  return (
    <section className="flex min-w-0 flex-col gap-xs border-t border-border pt-sm" data-panel-part="brief" data-panel-brief={open ? "open" : "folded"}>
      <Button
        variant="text"
        size="inline"
        className="w-fit text-caption"
        aria-expanded={open}
        onClick={() => {
          if (!open) actions.readMissionBrief(mission.id, node.key);
          setOpen(!open);
        }}
        data-panel-brief-toggle="true"
      >
        <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon-sm) transition-transform", open && "rotate-90")} />
        {t("mission.brief.title")}
        <span className="text-muted-foreground" data-panel-brief-size="true">· {size}</span>
      </Button>
      {text ? <pre className="max-h-(--size-relationship-list-max) min-w-0 overflow-auto whitespace-pre-wrap break-words rounded-xs border border-border bg-background p-sm font-mono text-caption text-subtle-foreground" data-panel-brief-text="true">{text}</pre> : null}
    </section>
  );
}
