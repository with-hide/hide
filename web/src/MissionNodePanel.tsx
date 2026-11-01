import { ChevronRightIcon, GitBranchIcon, XIcon } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { agentMarkOf } from "./agentLogos";
import { CheckerPopover, useCheckerStatus, VerdictReason, VERDICT_TONE } from "./components/mission-checker";
import { CriterionMark } from "./components/criterion-mark";
import { MissionNodeMark } from "./components/mission-node-mark";
import { MissionPrMark } from "./components/mission-pr";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { formatDateTime } from "./i18n/format";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { CheckerRow, lookWord, RunText, useNodeTimes, useRunWords, useWorkerRow } from "./missionParts";
import {
  cardLine,
  checkerState,
  criteriaTally,
  criterionName,
  criterionNote,
  eventWords,
  heldQuestion,
  kindWord,
  mainCommit,
  nodeLook,
  ownCriteria,
  shortCommit,
} from "./missions";
import { PlanChecklist } from "./components/plan-checklist";
import { planNowLine } from "./plans";
import { sessionOfAgent } from "./sessionWork";
import type { MessageKey } from "./i18n/catalogs";
import type { Mission, MissionChecker, MissionCriterion, MissionEvent, MissionNode } from "./snapshot";
import { useShellStore } from "./store";

// The node the operator picked on a mission's map (PRD mission-map-git-criteria
// D-01, D-10, B31-B33), read top down: its head (title, state, kind, model and
// effort, how long it took), where it is now, what it made, its finish lines
// as one list, what it waits on and what waits on it, the ways to its pull
// request, terminal and record, and its events folded.

export function MissionNodePanel({ mission, node, events, end, onSelect, onClose, actions }: { mission: Mission; node: MissionNode; events: MissionEvent[] | null; end: number; onSelect: (key: string) => void; onClose: () => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const look = nodeLook(mission, node);
  const worker = useRunWords(mission, node.pane_id, node.machine, node.worker);
  const times = useNodeTimes(mission, node, events, end);
  const facts = [
    ...(times?.took ? [t("mission.elapsed.total", { elapsed: times.took })] : []),
    ...(times?.held ? [t("mission.time.held", { elapsed: times.held })] : []),
    ...(node.attempts >= 2 ? [t("mission.badge.attempts", { count: node.attempts })] : []),
    ...(node.reopened > 0 ? [t("mission.badge.reopened", { count: node.reopened })] : []),
  ];
  return (
    <div className="flex min-w-0 flex-col gap-md px-md py-sm text-body" data-mission-detail={node.key}>
      <section className="flex min-w-0 flex-col gap-xxs" data-panel-part="head">
        <div className="flex min-w-0 items-start gap-xs">
          <MissionNodeMark look={look} label={lookWord(look, t)} className="mt-xxs" />
          <h3 className="min-w-0 flex-1 break-words text-subhead font-semibold" data-panel-title="true">{node.title}</h3>
          <Hint label={t("mission.panel.close")}>
            <Button variant="ghost" size="icon-sm" className="-mr-xs shrink-0 text-muted-foreground" aria-label={t("mission.panel.close")} onClick={onClose} data-panel-close="true">
              <XIcon />
            </Button>
          </Hint>
        </div>
        <p className="flex min-w-0 flex-wrap items-center gap-x-xs text-caption text-muted-foreground" data-panel-meta="true">
          <span>{lookWord(look, t)}</span>
          <span aria-hidden="true">·</span>
          <span>{t(kindWord(node.kind))}</span>
          {worker ? (
            <>
              <span aria-hidden="true">·</span>
              <span className="flex min-w-0 items-center gap-xxs"><AgentMark kind={node.worker?.agent} /><RunText run={worker} /></span>
            </>
          ) : null}
        </p>
        {facts.length > 0 ? <p className="text-caption text-muted-foreground tabular-nums" data-panel-facts="true">{facts.join(" · ")}</p> : null}
      </section>
      <Now mission={mission} node={node} />
      <Made mission={mission} node={node} />
      <Criteria mission={mission} node={node} actions={actions} />
      <Links mission={mission} node={node} onSelect={onSelect} />
      <Ways mission={mission} node={node} actions={actions} />
      <Events node={node} events={events} />
    </div>
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
 * Where the node is now (B31): the decision's question and choices while it
 * holds the node, its worker's own plan while it works, its checkers while in
 * review, else what it waits on or why it failed; finished work has no now.
 */
function Now({ mission, node }: { mission: Mission; node: MissionNode }) {
  const { t } = useInterfaceTranslation();
  const look = nodeLook(mission, node);
  if (look === "done" || look === "cancelled" || look === "dropped") return null;
  const question = heldQuestion(mission, node);
  if (look === "hold" && question) {
    const options = mission.question?.options ?? [];
    return (
      <Part title={t("mission.field.now")} part="now">
        <p className="min-w-0 break-words border-l-2 border-mission-hold pl-sm" data-panel-question="true">{question}</p>
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
      </Part>
    );
  }
  if (look === "working" && node.plan) {
    return (
      <Part title={t("mission.field.now")} part="now">
        {planNowLine(node.plan, t) ? <p className="min-w-0 break-words">{planNowLine(node.plan, t)}</p> : null}
        <div className="min-w-0 text-caption" data-panel-plan="true"><PlanChecklist plan={node.plan} /></div>
      </Part>
    );
  }
  const checks = look === "review" ? node.criteria.filter((line) => (line.kind === "review" || line.kind === "verify") && (line.checkers?.length ?? 0) > 0) : [];
  const line = cardLine(mission, node, null, t);
  if (!line && checks.length === 0) return null;
  return (
    <Part title={t("mission.field.now")} part="now">
      {line ? <p className={cn("min-w-0 break-words", line.attention && "text-mission-failed")}>{line.text}</p> : null}
      {checks.map((check) => (
        <div key={check.key} className="flex min-w-0 flex-col gap-xxs text-caption" data-panel-checkers={check.kind}>
          {(check.checkers ?? []).map((checker, index) => <CheckerRow key={index} mission={mission} kind={check.kind as "review" | "verify"} checker={checker} />)}
        </div>
      ))}
    </Part>
  );
}

/**
 * What the node made (B31): its pull request with its state, the CI line and
 * the base it stacks on, its branch, its commit on main, a research
 * conclusion or a verify node's verdict. Nothing the record does not hold.
 */
function Made({ mission, node }: { mission: Mission; node: MissionNode }) {
  const { t } = useInterfaceTranslation();
  const ci = node.criteria.find((line) => line.kind === "ci");
  const rows: [string, ReactNode][] = [];
  const pr = node.pr ?? (node.external && "pr" in node.external ? node.external.pr : null);
  const branch = node.branch ?? (node.external && "branch" in node.external ? node.external.branch : null);
  if (pr != null) {
    rows.push([
      t("mission.field.pr"),
      <span key="pr" className="flex min-w-0 flex-wrap items-center gap-xs">
        <MissionPrMark number={pr} state={node.pr_state} stale={mission.github_stale} />
        {node.base_pr != null ? <span className="text-caption text-muted-foreground" data-panel-base={node.base_pr}>{t("mission.panel.base", { number: String(node.base_pr) })}</span> : null}
      </span>,
    ]);
  }
  if (ci) rows.push([t("mission.panel.ci"), <LineState key="ci" mission={mission} line={ci} />]);
  if (branch) {
    rows.push([
      t("mission.panel.branch"),
      <span key="branch" className="flex min-w-0 items-start gap-xxs font-mono text-caption">
        <GitBranchIcon aria-hidden="true" className="mt-xxs size-(--size-pr-icon) shrink-0 text-muted-foreground" />
        <span className="min-w-0 break-all" data-panel-branch={branch}>{branch}</span>
      </span>,
    ]);
  }
  const commit = mainCommit(node);
  if (commit) rows.push(["main", <span key="commit" className="font-mono text-caption" title={commit} data-panel-commit={commit}>{t("mission.result.main", { commit: shortCommit(commit) })}</span>]);
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

const GROUPS: { key: string; kinds: MissionCriterion["kind"][]; title: MessageKey }[] = [
  { key: "own", kinds: ["own", "defect"], title: "mission.criteria.group.own" },
  { key: "auto", kinds: ["check", "ci"], title: "mission.criteria.group.auto" },
  { key: "agents", kinds: ["review", "verify"], title: "mission.criteria.group.agents" },
];

/**
 * The finish lines (B32), grouped by who decides them: the work's own lines
 * and defects, the automatic checks, and the other agents' review or
 * verification. Each line is ✓ ○ ⤳ ✕ and its words; a skipped or failed line
 * carries its verdict under it, the short reason first and the full text
 * folded, and a review or verification names each checker by a chip that
 * opens its answer and the way to its terminal or record. A mission from
 * before criteria shows its sentence (B23).
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
    <Part title={`${t("mission.field.criteria")} · ${tally.passed}/${tally.total}`} part="criteria">
      <div className="flex min-w-0 flex-col gap-sm" data-panel-criteria={node.criteria.map((line) => `${line.key}:${line.state}`).join(" ")}>
        {GROUPS.map((group) => {
          const lines = node.criteria.filter((line) => group.kinds.includes(line.kind));
          if (lines.length === 0) return null;
          return (
            <div key={group.key} className="flex min-w-0 flex-col gap-xxs" data-criteria-group={group.key}>
              <h5 className="text-micro text-subtle-foreground">{t(group.title)}</h5>
              <ul className="flex min-w-0 flex-col gap-xs">
                {lines.map((line) => <CriterionLine key={line.key} mission={mission} line={line} actions={actions} />)}
              </ul>
            </div>
          );
        })}
      </div>
    </Part>
  );
}

function CriterionLine({ mission, line, actions }: { mission: Mission; line: MissionCriterion; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const name = criterionName(line, t);
  // A skipped line's reason stands in its verdict, not after its words.
  const note = line.state === "skipped" ? null : criterionNote(line, mission, t);
  const checks = line.kind === "review" || line.kind === "verify" ? { kind: line.kind, checkers: line.checkers ?? [] } : null;
  return (
    <li className="grid min-w-0 grid-cols-[var(--size-agent-mark)_minmax(0,1fr)] gap-x-xs gap-y-xxs">
      <div className="contents" data-panel-line={line.key} data-line-state={line.state}>
        <CriterionMark state={line.state} />
        <span className="min-w-0">
          <span className="line-clamp-2 break-words" title={line.evidence ? `${name}\n\n${line.evidence}` : name}>{name}</span>
          {note ? <span className="text-caption text-muted-foreground" data-line-note="true">{note}</span> : null}
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
      {checks && checks.checkers.length > 0 ? (
        <div className="col-start-2 flex min-w-0 flex-col gap-xs" data-panel-checker-chips={checks.checkers.length}>
          <div className="flex min-w-0 flex-wrap gap-xs">
            {checks.checkers.map((checker, index) => <CheckerChip key={index} mission={mission} kind={checks.kind} checker={checker} actions={actions} />)}
          </div>
          {checks.checkers.filter((checker) => checker.state === "fail").map((checker, index) => (
            <Verdict key={index} tone="fail" word={t("mission.verdict.rejected")} who={checker.worker.agent}>
              <VerdictReason summary={checker.summary} evidence={checker.evidence} />
            </Verdict>
          ))}
        </div>
      ) : null}
    </li>
  );
}

/** A skipped or rejected line's verdict, under its words: its word in its colour, who gave it, and why. */
function Verdict({ tone, word, who, children }: { tone: "fail" | "skipped"; word: string; who?: string; children: ReactNode }) {
  return (
    <div className={cn("col-start-2 flex min-w-0 flex-col gap-xxs rounded-xs border-l-2 bg-secondary px-sm py-xs text-caption", tone === "fail" ? "border-mission-failed" : "border-mission-hold")} data-panel-verdict={tone}>
      <span className="flex min-w-0 items-center gap-xs">
        <span className={cn("shrink-0 rounded-full px-xs text-micro font-semibold", tone === "fail" ? "bg-mission-failed/15 text-mission-failed" : "bg-mission-hold/15 text-mission-hold")}>{word}</span>
        {who ? <span className="flex min-w-0 items-center gap-xxs text-muted-foreground"><AgentMark kind={who} /><span className="truncate">{agentMarkOf(who)?.label ?? who}</span></span> : null}
      </span>
      {children}
    </div>
  );
}

/** A checker of a review or verification by its agent and its answer; pressing it opens its popover (B17, B32). */
function CheckerChip({ mission, kind, checker, actions }: { mission: Mission; kind: "review" | "verify"; checker: MissionChecker; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const state = checkerState(checker);
  const status = useCheckerStatus(kind, checker);
  const name = agentMarkOf(checker.worker.agent)?.label ?? checker.worker.agent;
  return (
    <CheckerPopover mission={mission} kind={kind} checker={checker} actions={actions} hint={`${name} · ${status}`}>
      <Button variant="secondary" size="sm" className="h-auto gap-xxs px-xs py-xxs text-caption" data-panel-checker={checker.worker.agent} data-checker-state={state}>
        <AgentMark kind={checker.worker.agent} />
        <span>{name}</span>
        <span className={cn("rounded-full px-xs text-micro font-semibold", VERDICT_TONE[state])}>{state === "fail" ? t("mission.verdict.rejected") : status}</span>
        <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) text-muted-foreground" />
      </Button>
    </CheckerPopover>
  );
}

/** What the node waits on and what waits on it, by name with its pull request's mark beside it (PRD mission-tool B18); pressing one picks it (B33). */
function Links({ mission, node, onSelect }: { mission: Mission; node: MissionNode; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const before = node.depends_on.flatMap((key) => mission.nodes.find((other) => other.key === key) ?? []);
  const after = mission.nodes.filter((other) => other.depends_on.includes(node.key));
  const list = (nodes: MissionNode[], which: string) => (
    <div className="flex min-w-0 flex-wrap items-center gap-xs text-caption" data-panel-links={which}>
      <span className="text-muted-foreground">{t(which === "before" ? "mission.panel.waitsOn" : "mission.panel.waitedBy")}</span>
      {nodes.length === 0 ? <span className="text-muted-foreground">{t("mission.panel.none")}</span> : null}
      {nodes.map((other) => {
        const pr = other.pr ?? (other.external && "pr" in other.external ? other.external.pr : null);
        return (
          <button key={other.key} type="button" onClick={() => onSelect(other.key)} title={other.title} className="flex min-w-0 max-w-full items-center gap-xxs rounded-xs border border-border px-xs outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" data-panel-link={other.key}>
            <span className="min-w-0 truncate">{other.title}</span>
            {pr != null ? <MissionPrMark number={pr} state={other.pr_state} stale={mission.github_stale} /> : null}
          </button>
        );
      })}
    </div>
  );
  return (
    <Part title={t("mission.panel.links")} part="links">
      {list(before, "before")}
      {list(after, "after")}
    </Part>
  );
}

/** The ways out of the panel (B31): the PR card when the node has a pull request, its worker's terminal and its record. */
function Ways({ mission, node, actions }: { mission: Mission; node: MissionNode; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const row = useWorkerRow(node);
  // Derived outside the selector: it builds a new session each call, which a selector must not return.
  const rest = useShellStore((s) => s.rest);
  const session = useMemo(() => (row ? sessionOfAgent(row) : null), [row, rest]);
  if (node.kind === "external") return null;
  return (
    <div className="flex flex-wrap gap-xs" data-panel-part="ways">
      {node.pr != null ? (
        <Button variant="secondary" size="sm" disabled={!node.pr_url} onClick={() => node.pr_url && actions.openSessionPullRequest({ workspace_id: mission.workspace_id, url: node.pr_url, number: node.pr! })} data-node-pr-card="true">{t("mission.prCard")}</Button>
      ) : null}
      <Button size="sm" variant="secondary" disabled={!node.pane_id || !row} onClick={() => node.pane_id && actions.openAgent(node.pane_id)} data-node-terminal="true">{t("mission.toTerminal")}</Button>
      <Button size="sm" variant="secondary" disabled={!session} onClick={() => session && actions.openChildRecord(session)} data-node-record="true">{t("mission.record")}</Button>
    </div>
  );
}

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
