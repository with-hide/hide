import { ChevronRightIcon, GitBranchIcon, XIcon } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { CriterionMark } from "./components/criterion-mark";
import { MissionNodeMark } from "./components/mission-node-mark";
import { PrMark } from "./components/pr-mark";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { formatDateTime } from "./i18n/format";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { CheckerRow, lookWord, RunText, useNodeTimes, useRunWords, useWorkerRow } from "./missionParts";
import {
  cardLine,
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
import { PlanChecklist } from "./PlanChecklist";
import { planNowLine } from "./plans";
import { sessionOfAgent } from "./sessionWork";
import type { Mission, MissionCriterion, MissionEvent, MissionNode } from "./snapshot";
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
      <Criteria mission={mission} node={node} />
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
  if (look === "done" || look === "landed" || look === "cancelled" || look === "dropped") return null;
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
        {node.pr_state ? <PrMark state={node.pr_state} number={pr} stale={mission.github_stale} /> : <span className="font-mono">#{pr}</span>}
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

/**
 * The finish lines as one list (B32): ✓ ○ ⤳ ✕ and each line's words, a
 * review or verification named by its checkers, a skipped line's reason, a
 * rerun that passed. A mission from before criteria shows its sentence (B23).
 */
function Criteria({ mission, node }: { mission: Mission; node: MissionNode }) {
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
      <ul className="flex min-w-0 flex-col gap-xxs" data-panel-criteria={node.criteria.map((line) => `${line.key}:${line.state}`).join(" ")}>
        {node.criteria.map((line) => {
          const note = criterionNote(line, mission, t);
          return (
            <li key={line.key} className="grid min-w-0 grid-cols-[var(--size-agent-mark)_minmax(0,1fr)] gap-xs" title={line.evidence} data-panel-line={line.key} data-line-state={line.state}>
              <CriterionMark state={line.state} />
              <span className="min-w-0 break-words">
                {criterionName(line, t)}
                {note ? <span className={cn("ml-xs text-caption", line.state === "skipped" ? "text-mission-hold" : "text-muted-foreground")} data-line-note="true">{note}</span> : null}
              </span>
            </li>
          );
        })}
      </ul>
    </Part>
  );
}

/** What the node waits on and what waits on it, by name with a pull request's number dim beside it; pressing one picks it (B33). */
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
            {pr != null ? <span className="shrink-0 font-mono text-muted-foreground">#{pr}</span> : null}
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
  const session = useMemo(() => (row ? sessionOfAgent(row, rest) : null), [row, rest]);
  if (node.kind === "external") return null;
  return (
    <div className="flex flex-wrap gap-xs" data-panel-part="ways">
      {node.pr != null ? (
        <Button size="sm" disabled={!node.pr_url} onClick={() => node.pr_url && actions.openSessionPullRequest({ workspace_id: mission.workspace_id, url: node.pr_url, number: node.pr! })} data-node-pr-card="true">{t("mission.prCard")}</Button>
      ) : null}
      <Button size="sm" variant={node.pr != null ? "secondary" : "default"} disabled={!node.pane_id || !row} onClick={() => node.pane_id && actions.openAgent(node.pane_id)} data-node-terminal="true">{t("mission.goTerminal")}</Button>
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
