import { ChevronRightIcon, GitBranchIcon } from "lucide-react";
import type { TFunction } from "i18next";
import type { ReactNode } from "react";
import { AgentMark } from "./AgentMark";
import { formatElapsed } from "./components/elapsed";
import { PrMark } from "./components/pr-mark";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { checkerState, launchSession, nodeTimes, runWords, shortCommit, type NodeLook, type NodeResult } from "./missions";
import { allAgents, agentsByPane } from "./paneAgents";
import type { Mission, MissionChecker, MissionEvent, MissionNode, MissionWorker } from "./snapshot";
import { useShellStore } from "./store";

// The parts a mission's map, its node panel and the Conductor pane's popover
// draw alike: a node's result, its worker's live row, a look's word, a node's
// times and one checker of a review or verification.

/**
 * What a node left, by its kind, as the Conductor pane's popover lists it
 * (PRD mission-plan-visibility B11): `⑂ #n` in its pull request's state
 * colour, a direct change's branch or its commit on main, `결론 ▸` for
 * research or a verify node's verdict or line.
 */
export function ResultTail({ result, stale, full = false }: { result: NodeResult; stale: boolean; full?: boolean }) {
  const { t } = useInterfaceTranslation();
  switch (result.kind) {
    case "pr":
      return result.state ? <PrMark state={result.state} number={result.number} stale={stale} data-node-pr={result.number} /> : <span className="shrink-0 font-mono text-caption text-subtle-foreground" data-node-pr={result.number}>#{result.number}</span>;
    case "commit":
      return <span className="shrink-0 font-mono text-caption text-mission-done" title={result.commit} data-node-commit={result.commit}>{t("mission.result.main", { commit: shortCommit(result.commit) })}</span>;
    case "verdict":
      return (
        <span className={cn("shrink-0 font-mono text-caption font-semibold", result.verdict === "pass" ? "text-mission-done" : "text-mission-failed")} data-node-verdict={result.verdict}>
          {t(result.verdict === "pass" ? "mission.result.pass" : "mission.result.fail")}
          {result.checks ? ` ${result.checks.passed}/${result.checks.total}` : ""}
        </span>
      );
    case "branch":
      return (
        <span className="flex min-w-0 items-center gap-xxs text-caption text-subtle-foreground" data-node-branch={result.branch}>
          <GitBranchIcon aria-hidden="true" className="size-(--size-pr-icon) shrink-0" />
          <span className="truncate font-mono" title={result.branch}>{result.branch}</span>
        </span>
      );
    case "conclusion":
      return full ? (
        <span className="min-w-0 break-words text-caption" data-node-conclusion="full">{t("mission.result.conclusion")}: {result.line}</span>
      ) : (
        <span className="flex shrink-0 items-center text-caption text-foreground" title={result.line} data-node-conclusion="folded">
          {t("mission.result.conclusion")}
          <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />
        </span>
      );
    case "verify":
      return <span className={cn("min-w-0 text-caption text-muted-foreground", full ? "break-words" : "truncate")} title={result.line} data-node-verify="true">{result.line}</span>;
  }
}

/** The worker's live row, the one the panel's terminal and record follow. */
export function useWorkerRow(node: MissionNode) {
  return useShellStore((s) => {
    if (!node.pane_id) return null;
    const rows = agentsByPane(allAgents(s.agents, s.rest?.status?.remote));
    return rows.get(node.pane_id) ?? null;
  });
}

/** The look's word, the same on the card's mark, its hover and its accessible name (B26). */
export function lookWord(look: NodeLook, t: TFunction<"translation">): string {
  return t(`mission.look.${look}`);
}

/** How long a node took and how long a decision held it, from its events (B28); nothing moves while its device does not answer (B35). */
export function useNodeTimes(mission: Mission, node: MissionNode, events: MissionEvent[] | null, end: number) {
  const { i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const times = events ? nodeTimes(node, events, end) : null;
  if (!times || (times.running && mission.stale)) return null;
  return { took: times.took != null && times.took > 0 ? formatElapsed(language, times.took) : null, held: times.held > 0 ? formatElapsed(language, times.held) : null };
}

const CHECKER_TONE = { pass: "text-muted-foreground", fail: "text-mission-failed", running: "text-mission-working", waiting: "text-muted-foreground" } as const;

/** One checker: its agent's face, the model and effort it runs, and where its check is. */
export function CheckerRow({ mission, kind, checker }: { mission: Mission; kind: "review" | "verify"; checker: MissionChecker }) {
  const { t } = useInterfaceTranslation();
  const state = checkerState(checker);
  const run = useRunWords(mission, checker.pane_id, checker.machine, checker.worker);
  const status = state === "pass" || state === "fail" ? t(`mission.checker.${state}`) : t(`mission.checker.${state}.${kind}`);
  return (
    <span className="flex min-w-0 items-center gap-xs" data-node-checker={checker.worker.agent} data-checker-state={state}>
      <AgentMark kind={checker.worker.agent} />
      {run ? <RunText run={run} className="flex-1" /> : <span className="flex-1" />}
      <span className={cn("shrink-0", CHECKER_TONE[state])}>
        {state === "pass" ? "✓ " : state === "fail" ? "✕ " : ""}
        {status}
      </span>
    </span>
  );
}

/** What runs in a pane: its words, the mark a plan's words carry, and the two as one line for a hover or a name. */
export type RunLabel = { text: string; words: string; mark: string | null; from: "launch" | "plan" };

/**
 * What runs in a pane as Hide launched it, else what the plan asked marked
 * planned (B29); `from` says which, for a reader that tells them apart.
 */
export function useRunWords(mission: Mission, paneId: string | null | undefined, machine: string | null | undefined, plan: MissionWorker | null | undefined): RunLabel | null {
  const { t } = useInterfaceTranslation();
  const launch = useShellStore((s) => launchSession(s.rest, mission, paneId, machine));
  const run = runWords(launch, plan, { auto: t("mission.auto"), defaultModel: t("conductor.defaultModel") });
  if (!run) return null;
  return run.planned
    ? { text: t("mission.planned", { words: run.words }), words: run.words, mark: t("mission.plannedMark"), from: "plan" }
    : { text: run.words, words: run.words, mark: null, from: "launch" };
}

/** A run's words, which truncate, before its planned mark, which never does, so a planned model never reads as launched. */
export function RunText({ run, className, wordsClassName, children }: { run: RunLabel; className?: string; wordsClassName?: string; children?: ReactNode }) {
  return (
    <span className={cn("flex min-w-0 items-center gap-xxs", className)} title={run.text} data-run={run.from}>
      {/* A grid track that starts at nothing: the words ask no width to fit in, yet take their whole width when there is room. */}
      <span className={cn("grid min-w-0 grid-cols-[minmax(0,1fr)]", wordsClassName)} data-run-words="true"><span className="truncate">{run.words}</span></span>
      {run.mark ? <span className="shrink-0" data-run-planned="true">{run.mark}</span> : null}
      {children}
    </span>
  );
}
