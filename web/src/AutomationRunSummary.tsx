import { useState } from "react";
import type { Actions } from "./actions";
import { agentProductName } from "./agentAdapters";
import { type Words, ToneMark, useWords } from "./automationMarks";
import { recordText, whenText, type RecordTone } from "./automationWords";
import { AgentLogo } from "./components/agent-logo";
import { Button } from "./components/ui/button";
import { RowButton } from "./components/ui/row-button";
import { Hint } from "./components/ui/tooltip";
import { cn } from "./lib/utils";
import type { Automation, AutomationRecord, AutomationRunResult } from "./snapshot";
import { useShellStore } from "./store";
import { ViewKindMark } from "./viewMark";

// A run's summary (operator decision 2026-11-21, card A; 2026-11-24): how the
// run ended, the agent's last answer, what its agent opened (결과), the task
// and the automation's last runs. It is the first, fixed tab of a run's desk
// (the `automation_run` card) and, while the run has no tab, the whole right
// side of the run screen with its result rows shown but not pressable.

type Run = Extract<AutomationRecord, { kind: "run" }>;

/** The answer card's edge, in the run's tone. */
const TONE_EDGE: Record<RecordTone, string> = {
  ok: "border-success",
  failed: "border-destructive",
  skipped: "border-border",
  running: "border-agent-working",
  stopped: "border-border",
};

/**
 * How the run ended in large, when it started, how and with which agent;
 * the agent's last answer; what its agent opened as 결과, each row a press
 * that brings that View in front on the run's desk (`pressable`); the task
 * folded to three lines; and the automation's last runs, each a press to
 * that run.
 */
export function RunSummary({ automation, run, going, actions, pressable }: { automation: Automation; run: Run; going: boolean; actions: Actions; pressable: boolean }) {
  const words = useWords();
  const { t } = words;
  const [taskOpen, setTaskOpen] = useState(false);
  const shown = recordText(run, words.now, words.zone, words.language, t);
  const headline = shown.tone === "ok" ? t("automationScreen.run.summaryDone", { time: shown.text }) : shown.text;
  const when = whenText(run.started_at_unix_ms, words.now, words.zone, words.language, t);
  const how = run.trigger === "manual" ? t("automationScreen.run.manual") : run.trigger === "missed" ? t("automationScreen.run.missed") : t("automationScreen.run.scheduled");
  const agent = [agentProductName(automation.agent.kind), automation.agent.model].filter(Boolean).join(" · ");
  const long = automation.task.length > 160 || automation.task.split("\n").length > 3;
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-lg overflow-y-auto px-lg py-lg" data-automation-run-summary={run.number}>
      <div className="flex items-center gap-sm" data-automation-run-summary-head={shown.tone}>
        <span className="flex size-(--size-control) shrink-0 items-center justify-center rounded-full bg-secondary"><ToneMark tone={shown.tone} /></span>
        <div className="flex min-w-0 flex-col gap-xxs">
          <span className={cn("truncate text-subhead font-semibold", shown.tone === "failed" && "text-destructive")} title={headline}>{headline}</span>
          <span className="flex min-w-0 flex-wrap items-center gap-x-md gap-y-xxs text-caption text-muted-foreground">
            <span className="tabular-nums">{when} · {how}</span>
            <span className="flex min-w-0 items-center gap-xxs"><AgentLogo agent={automation.agent.kind} label={agentProductName(automation.agent.kind)} /><span className="truncate">{agent}</span></span>
          </span>
        </div>
      </div>
      {going ? null : (
        <section className="flex flex-col gap-xs">
          <h3 className="text-micro text-muted-foreground">{t("automationScreen.run.summaryAnswer")}</h3>
          {run.answer ? (
            <p className={cn("whitespace-pre-wrap break-words rounded-r-md border-l-2 bg-secondary px-md py-sm text-body text-foreground", TONE_EDGE[shown.tone])} data-automation-run-answer="true">{run.answer}</p>
          ) : (
            <p className="text-caption text-muted-foreground">{t("automationScreen.run.summaryNoAnswer")}</p>
          )}
        </section>
      )}
      <RunResults automation={automation} run={run} going={going} pressable={pressable} actions={actions} />
      <section className="flex flex-col gap-xxs" data-automation-run-task={taskOpen ? "open" : "folded"}>
        <h3 className="text-micro text-muted-foreground">{t("automationScreen.run.summaryTask")}</h3>
        <p className={cn("whitespace-pre-wrap break-words text-caption text-subtle-foreground", !taskOpen && "line-clamp-3")}>{automation.task}</p>
        {long ? (
          <Button variant="text" size="inline" className="self-start text-caption" onClick={() => setTaskOpen(!taskOpen)} data-automation-run-task-toggle="true">
            {taskOpen ? t("automationScreen.run.taskFold") : t("automationScreen.run.taskUnfold")}
          </Button>
        ) : null}
      </section>
      <RunStrip automation={automation} current={run.number} actions={actions} words={words} />
    </div>
  );
}

/** The automation's last five runs, oldest first, each its mark and a press to it; the run shown is ringed, and `5회 중 4번 끝남` sums them. */
function RunStrip({ automation, current, actions, words }: { automation: Automation; current: number; actions: Actions; words: Words }) {
  const { t } = words;
  const runs = automation.records.filter((record): record is Extract<AutomationRecord, { kind: "run" }> => record.kind === "run").slice(0, 5).reverse();
  if (runs.length < 2) return null;
  const done = runs.filter((row) => row.state === "succeeded").length;
  return (
    <section className="mt-auto flex flex-col gap-xs" data-automation-run-recent={runs.length}>
      <h3 className="text-micro text-muted-foreground">{t("automationScreen.run.recent")}</h3>
      <div className="flex items-center gap-xs">
        {runs.map((row) => {
          const line = recordText(row, words.now, words.zone, words.language, t);
          const label = `${whenText(row.due_unix_ms ?? row.started_at_unix_ms, words.now, words.zone, words.language, t)} · ${line.text}`;
          return (
            <Hint key={row.number} label={label}>
              <Button variant="outline" size="icon-xs" aria-label={label} aria-current={row.number === current ? "true" : undefined} className={cn(row.number === current && "ring-2 ring-ring")} onClick={() => actions.openAutomation(automation.id, row.number)} data-automation-run-recent-item={row.number}>
                <ToneMark tone={line.tone} />
              </Button>
            </Hint>
          );
        })}
        <span className="ml-xs text-caption text-muted-foreground tabular-nums">{t("automationScreen.run.recentDone", { count: runs.length, done })}</span>
      </div>
    </section>
  );
}

/**
 * What the run's agent opened for the operator, newest first: a file, a diff
 * or a page, each a press that brings that View in front on the run's desk,
 * opening it again when its tab was closed (`pressable`; without a tab the
 * rows are shown only). A run that goes and opened nothing yet keeps the
 * heading with one quiet line; a finished one that opened nothing draws no
 * section. A row carries no 볼 것 mark and no note of a closed tab: the tabs
 * beside the summary already say what is unseen (design principle 13).
 */
function RunResults({ automation, run, going, pressable, actions }: { automation: Automation; run: Run; going: boolean; pressable: boolean; actions: Actions }) {
  const { t } = useWords();
  const results = run.results ?? [];
  if (results.length === 0 && !going) return null;
  return (
    <section className="flex flex-col gap-xxs" data-automation-run-results={results.length}>
      <h3 className="text-micro text-muted-foreground">{t("automationScreen.run.results")}</h3>
      {results.length === 0 ? (
        <p className="text-caption text-muted-foreground" data-automation-run-results-empty="true">{t("automationScreen.run.resultsEmpty")}</p>
      ) : (
        <ul className="flex flex-col">
          {results.map((result) => <ResultRow key={result.key} result={result} pressable={pressable} onOpen={() => actions.automationRunResultOpen(automation.id, run.number, result.key)} />)}
        </ul>
      )}
    </section>
  );
}

function ResultRow({ result, pressable, onOpen }: { result: AutomationRunResult; pressable: boolean; onOpen: () => void }) {
  const body = (
    <>
      <span className="flex size-(--size-icon) shrink-0 items-center justify-center"><ViewKindMark kind={result.kind} label={result.label} /></span>
      <span className="min-w-0 flex-1 truncate text-caption text-foreground" title={result.label}>{result.label}</span>
    </>
  );
  return (
    <li data-automation-run-result={result.key} data-automation-run-result-kind={result.kind}>
      {pressable ? (
        <RowButton onClick={onOpen} data-automation-run-result-open={result.key}>{body}</RowButton>
      ) : (
        <div className="flex w-full min-w-0 items-center gap-xs px-xs py-xs">{body}</div>
      )}
    </li>
  );
}

/**
 * The summary as a card on the run's desk (`automation_run`): the run it
 * names from the automations section, its rows pressable. A run the section
 * no longer holds draws nothing.
 */
export function RunSummaryCard({ automation, run, actions }: { automation: string; run: number; actions: Actions }) {
  const item = useShellStore((s) => s.automations?.items.find((row) => row.id === automation) ?? null);
  const record = item?.records.find((row): row is Run => row.kind === "run" && row.number === run) ?? null;
  if (!item || !record) return null;
  return <RunSummary automation={item} run={record} going={record.state === "running" || record.state === "starting"} actions={actions} pressable />;
}
