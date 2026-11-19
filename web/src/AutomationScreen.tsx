import { CheckIcon, ChevronLeftIcon, FolderOpenIcon, MinusIcon, PauseIcon, PencilIcon, PlayIcon, PlusIcon, SquareIcon, Trash2Icon, XIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { agentProductName } from "./agentAdapters";
import { callAutomations } from "./automationCall";
import { AutomationForm } from "./AutomationForm";
import { settledPlace } from "./automationPlace";
import { lastResult, nextRun, nextRunText, recordText, scheduleText, whenText, type RecordTone } from "./automationWords";
import { useMinuteClock } from "./components/elapsed";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "./components/ui/alert-dialog";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { RowButton, RowOverlayButton } from "./components/ui/row-button";
import { Hint } from "./components/ui/tooltip";
import { hostBridge } from "./host";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage, type InterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { agentProps } from "./PaneGrid";
import { PaneView } from "./PaneView";
import { catalogWorkspaces, type Automation, type AutomationRecord, type AutomationRun, type AutomationsSection, type PaneRow, type SnapshotRest } from "./snapshot";
import { focusedPaneOf, useShellStore } from "./store";
import { useUiStore } from "./ui";

// The automation screen (PRD automations D-02, B1-B8, B11, B15-B18,
// B25-B27, B30, B38): this core's automations as a table and
// [+ 새 자동화]; a row opens its detail, whose run list opens a run's
// terminal in place. It is the page's own navigation, like the mission
// screen whose table and head it follows (design principle 5): where it
// stands is `AutomationPlace`. Every value is the core's `automations`
// section; a press is one window event, and the form's calls wait for the
// core's answer (`automationCall.ts`).

type Words = { t: ReturnType<typeof useInterfaceTranslation>["t"]; language: InterfaceLanguage; zone: string | undefined; now: number };

function useWords(): Words {
  const { t, i18n } = useInterfaceTranslation();
  const zone = useShellStore((s) => s.automations?.zone);
  const now = useMinuteClock();
  return { t, language: requireInterfaceLanguage(i18n.language), zone, now };
}

export function AutomationScreen({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const section = useShellStore((s) => s.automations);
  const place = useUiStore((s) => s.automations);
  const selected = section?.items.find((item) => item.id === place.selected) ?? null;
  const editing = place.form === "edit" ? selected : null;

  // An automation or run removed while it shows steps back without a word; one just answered waits for its section (B8).
  useEffect(() => {
    const settled = settledPlace(place, section);
    if (settled) useUiStore.getState().setAutomations(settled);
  }, [place, section]);

  const waiting = section === null || (place.arriving && !selected);
  const state = waiting ? "loading" : place.form ? "form" : selected ? (place.run !== null ? "run" : "detail") : "table";
  return (
    // The table follows the mission screen's, so it takes that screen's container and its one breakpoint (`missions-split`).
    <section className="@container/missions flex min-h-0 min-w-0 flex-1 flex-col bg-background text-foreground" aria-label={t("automationScreen.title")} data-automation-screen={state}>
      {waiting ? (
        <>
          <Head count={null} />
          <Skeleton />
        </>
      ) : place.form === "new" || (place.form === "edit" && editing) ? (
        <AutomationForm key={editing?.id ?? "new"} section={section} automation={editing} actions={actions} />
      ) : selected ? (
        <Detail key={selected.id} automation={selected} run={place.run} actions={actions} />
      ) : (
        <>
          <Head count={section.items.length} />
          {section.unreadable ? <UnreadableLine file={section.unreadable.file} /> : null}
          <Table section={section} actions={actions} />
        </>
      )}
    </section>
  );
}

/** `자동화 · N개` and [+ 새 자동화] (B2, B4). */
function Head({ count }: { count: number | null }) {
  const { t } = useInterfaceTranslation();
  return (
    <header className="flex shrink-0 items-center gap-md border-b border-border px-lg py-sm" data-automation-screen-head="true">
      <h1 className="flex min-w-0 flex-1 items-baseline gap-xs text-subhead">
        <span className="shrink-0 font-semibold text-foreground">{t("automationScreen.title")}</span>
        {count !== null && count > 0 ? <span className="text-body tabular-nums text-muted-foreground" data-automation-count={count}>· {t("automationScreen.count", { count })}</span> : null}
      </h1>
      {count !== null && count > 0 ? <NewButton /> : null}
    </header>
  );
}

function NewButton({ className }: { className?: string }) {
  const { t } = useInterfaceTranslation();
  return (
    <Button variant="secondary" size="sm" className={className} onClick={() => useUiStore.getState().setAutomations({ selected: null, run: null, form: "new" })} data-automation-new="true">
      <PlusIcon aria-hidden="true" />
      {t("automationScreen.new")}
    </Button>
  );
}

/** The saved book the core could not read and set aside, one line above the table until an automation is made. */
function UnreadableLine({ file }: { file: string }) {
  const { t } = useInterfaceTranslation();
  return <p className="shrink-0 px-lg pt-sm text-caption text-warning" title={file} data-automation-unreadable="true">{t("automationScreen.unreadable", { file: lastName(file) })}</p>;
}

/** Quiet rows before the core's automations first arrive: no count, no words. */
function Skeleton() {
  return (
    <div className="flex flex-col gap-sm px-lg py-md" aria-hidden="true" data-automation-screen-state="loading">
      {[0, 1, 2].map((row) => (
        <div key={row} className="flex items-center gap-md">
          <span className="h-(--size-badge-height) flex-1 rounded-xs bg-secondary" />
          <span className="h-(--size-badge-height) w-(--size-mission-table-project) rounded-xs bg-secondary" />
          <span className="h-(--size-badge-height) w-(--size-mission-table-project) rounded-xs bg-secondary" />
        </div>
      ))}
    </div>
  );
}

/** The table's columns; a screen too narrow for every column leaves out the schedule and the folder and agent, which the detail names (B3). */
const TABLE_COLUMNS = "grid grid-cols-[var(--size-icon-sm)_minmax(0,1fr)_var(--size-mission-table-project)_var(--size-mission-table-project)_var(--size-control-sm)] missions-split:grid-cols-[var(--size-icon-sm)_minmax(0,1fr)_var(--size-mission-table-project)_var(--size-mission-table-project)_var(--size-mission-table-project)_minmax(0,var(--size-mission-table-steps))_var(--size-control-sm)] items-center gap-x-md";

/**
 * The table (B2, B3, B38): each automation in the core's order, its unseen
 * dot, name, schedule, next run, last result, folder and agent, and ▶ to run
 * it now; a paused one dimmed. With none, the empty line and [+ 새 자동화].
 * ↑ and ↓ move between the rows, Enter opens one.
 */
function Table({ section, actions }: { section: AutomationsSection; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const box = useRef<HTMLDivElement>(null);
  // Back from a detail or the form, whose button is gone, the keys land on the first row or [+ 새 자동화] (B38).
  useEffect(() => {
    const active = document.activeElement;
    if (active && active !== document.body && active.isConnected) return;
    box.current?.querySelector<HTMLElement>("[data-automation-open], [data-automation-new]")?.focus();
  }, []);
  if (section.items.length === 0) {
    return (
      <div ref={box} className="flex flex-col items-start gap-sm px-lg py-xl" data-automation-screen-state="empty">
        <p className="text-body text-foreground">{t("automationScreen.empty")}</p>
        <NewButton />
        <p className="text-caption text-muted-foreground">{t("automationScreen.emptyTalk")}</p>
      </div>
    );
  }
  const step = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    const rows = Array.from(box.current?.querySelectorAll<HTMLElement>("[data-automation-open]") ?? []);
    const index = rows.findIndex((row) => row === document.activeElement);
    const next = rows[index < 0 ? 0 : index + (event.key === "ArrowDown" ? 1 : -1)];
    if (!next) return;
    event.preventDefault();
    next.focus();
  };
  return (
    <div ref={box} className="min-h-0 flex-1 overflow-y-auto pb-lg" onKeyDown={step} data-automation-screen-state="table">
      <div className={cn(TABLE_COLUMNS, "sticky top-0 z-10 border-b border-border/60 bg-background px-[calc(var(--spacing-lg)+var(--spacing-xs))] py-xs text-caption text-muted-foreground")} aria-hidden="true">
        <span />
        <span>{t("automationScreen.col.name")}</span>
        <span className="hidden missions-split:block">{t("automationScreen.col.schedule")}</span>
        <span>{t("automationScreen.col.next")}</span>
        <span>{t("automationScreen.col.last")}</span>
        <span className="hidden missions-split:block">{t("automationScreen.col.where")}</span>
        <span />
      </div>
      <div className="flex flex-col px-lg pt-xs">
        {section.items.map((automation) => <TableRow key={automation.id} automation={automation} actions={actions} />)}
      </div>
    </div>
  );
}

function TableRow({ automation, actions }: { automation: Automation; actions: Actions }) {
  const words = useWords();
  const { t } = words;
  const last = lastResult(automation);
  const lastWords = last ? recordText(last, words.now, words.zone, words.language, t) : null;
  return (
    <div className={cn("relative min-w-0 rounded-xs px-xs py-sm", TABLE_COLUMNS, automation.paused && "opacity-(--opacity-dimmed)")} data-automation-row={automation.id} data-automation-row-paused={automation.paused ? automation.paused.reason : undefined}>
      <RowOverlayButton aria-label={t("automationScreen.open", { name: automation.name })} onClick={() => actions.openAutomation(automation.id)} data-automation-open={automation.id} />
      <span className="pointer-events-none relative flex justify-center">
        {automation.unseen > 0 ? <span role="img" aria-label={t("automationScreen.unseen")} className="size-(--size-tab-status-dot) rounded-full bg-primary" data-automation-unseen={automation.unseen} /> : null}
      </span>
      <span className="pointer-events-none relative min-w-0 truncate text-body font-semibold text-foreground" title={automation.name}>{automation.name}</span>
      <span className="pointer-events-none relative hidden min-w-0 truncate text-caption text-subtle-foreground missions-split:block">{scheduleText(automation.reads_as, words.language, t)}</span>
      <span className="pointer-events-none relative min-w-0"><NextRunWords automation={automation} words={words} /></span>
      <span className="pointer-events-none relative min-w-0">{lastWords ? <RecordWords tone={lastWords.tone} text={lastWords.text} /> : null}</span>
      <span className="pointer-events-none relative hidden min-w-0 items-center gap-xs text-caption text-subtle-foreground missions-split:flex"><Where automation={automation} /></span>
      <span className="relative flex justify-end">
        <RunNowButton automation={automation} actions={actions} icon />
      </span>
    </div>
  );
}

/** `● 도는 중`, `⏸ 멈춤` or the next time (B3, B25). */
function NextRunWords({ automation, words }: { automation: Automation; words: Words }) {
  const next = nextRun(automation);
  const text = nextRunText(automation, words.now, words.zone, words.language, words.t);
  return (
    <span className={cn("flex min-w-0 items-center gap-xs text-caption", next.kind === "running" ? "text-agent-working" : "text-subtle-foreground")} data-automation-next={next.kind}>
      {next.kind === "running" ? <span aria-hidden="true" className="size-(--size-tab-status-dot) shrink-0 rounded-full bg-agent-working" /> : null}
      {next.kind === "paused" ? <PauseIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" /> : null}
      <span className="min-w-0 truncate">{text}</span>
    </span>
  );
}

const TONE_CLASS: Record<RecordTone, string> = {
  ok: "text-success",
  failed: "text-destructive",
  skipped: "text-muted-foreground",
  running: "text-agent-working",
  stopped: "text-muted-foreground",
};

/** A record's mark: ✓, ✗, –, ● or ■ (B3, B16, B20, B22). */
function ToneMark({ tone }: { tone: RecordTone }) {
  const className = cn("size-(--size-icon-sm) shrink-0", TONE_CLASS[tone]);
  switch (tone) {
    case "ok":
      return <CheckIcon aria-hidden="true" className={className} />;
    case "failed":
      return <XIcon aria-hidden="true" className={className} />;
    case "skipped":
      return <MinusIcon aria-hidden="true" className={className} />;
    case "running":
      return <span aria-hidden="true" className="flex size-(--size-icon-sm) shrink-0 items-center justify-center"><span className="size-(--size-tab-status-dot) rounded-full bg-agent-working" /></span>;
    case "stopped":
      // ■, filled, so it never reads as an empty checkbox.
      return <SquareIcon aria-hidden="true" className={cn(className, "fill-current")} />;
  }
}

function RecordWords({ tone, text }: { tone: RecordTone; text: string }) {
  return (
    <span className="flex min-w-0 items-center gap-xs text-caption text-subtle-foreground" data-automation-record-tone={tone}>
      <ToneMark tone={tone} />
      <span className={cn("min-w-0 truncate", tone === "failed" && "text-destructive")} title={text}>{text}</span>
    </span>
  );
}

/** `기본 폴더 · Claude`, `hide · Codex · 새 워크트리`. */
function Where({ automation }: { automation: Automation }) {
  const { t } = useInterfaceTranslation();
  const folder = automation.folder.default ? t("automationScreen.ownFolder") : lastName(automation.folder.path);
  return (
    <>
      <span className="min-w-0 truncate" title={automation.folder.path}>{folder}</span>
      <span aria-hidden="true">·</span>
      <AgentMark kind={automation.agent.kind} />
      <span className="shrink-0">{agentProductName(automation.agent.kind)}</span>
      {automation.folder.worktree ? <span className="shrink-0">· {t("automationScreen.worktree")}</span> : null}
    </>
  );
}

function lastName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/** ▶ 지금 실행 (B19): refused while a run goes, which the button says on hover. */
function RunNowButton({ automation, actions, icon = false }: { automation: Automation; actions: Actions; icon?: boolean }) {
  const { t } = useInterfaceTranslation();
  const label = icon ? t("automationScreen.runNowNamed", { name: automation.name }) : t("automationScreen.runNow");
  const button = (
    <Button
      variant={icon ? "row-action" : "secondary"}
      size={icon ? "icon-xs" : "sm"}
      aria-label={label}
      disabled={automation.running}
      onClick={() => actions.automation("run", automation.id)}
      data-automation-run-now={automation.id}
    >
      <PlayIcon aria-hidden="true" />
      {icon ? null : t("automationScreen.runNow")}
    </Button>
  );
  // A disabled button takes no pointer, so its hint hangs on the box around it.
  return <Hint label={automation.running ? t("automationScreen.runGoing") : label}><span className="flex">{button}</span></Hint>;
}

/**
 * An automation's detail (B11, B25-B27, B30): back to the table, its name
 * and actions; its task, schedule, next run, folder and agent; its runs. A
 * run opened from the list shows its terminal in place of the facts (B16,
 * B17). Opening it marks its runs seen (B11).
 */
function Detail({ automation, run, actions }: { automation: Automation; run: number | null; actions: Actions }) {
  const live = useShellStore((s) => s.connection === "live");
  const unseen = automation.unseen > 0;
  // Seen once the detail shows its runs, and again when a run ends while it shows (B11, B14).
  useEffect(() => {
    if (live && unseen) actions.automation("seen", automation.id);
  }, [actions, automation.id, unseen, live]);
  const shown = run === null ? null : automation.records.find((record): record is Extract<AutomationRecord, { kind: "run" }> => record.kind === "run" && record.number === run) ?? null;
  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col" aria-label={automation.name} data-automation-detail={automation.id}>
      {shown ? <RunView key={shown.number} automation={automation} run={shown} actions={actions} /> : run !== null ? <Skeleton /> : <Facts automation={automation} actions={actions} />}
    </section>
  );
}

function BackButton({ label, onBack }: { label: string; onBack: () => void }) {
  return (
    <Button variant="text" size="sm" className="-ml-xs gap-xxs px-xs" onClick={onBack} data-automation-back="true">
      <ChevronLeftIcon aria-hidden="true" />
      <span className="max-w-(--size-mission-table-project) truncate">{label}</span>
    </Button>
  );
}

function Facts({ automation, actions }: { automation: Automation; actions: Actions }) {
  const words = useWords();
  const { t } = words;
  const canOpenFolder = hostBridge() !== null && actions.onCoreMachine();
  const agent = [agentProductName(automation.agent.kind), automation.agent.model ?? t("automationScreen.detail.agentDefault"), automation.agent.effort].filter(Boolean).join(" · ");
  const back = useRef<HTMLDivElement>(null);
  // Opened from the table, the keys land in the detail, so Escape-less keyboard use goes on from its first action (B38).
  useEffect(() => {
    const active = document.activeElement;
    if (!active || active === document.body || !active.isConnected) back.current?.querySelector<HTMLElement>("button")?.focus();
  }, []);
  return (
    <>
      <div ref={back} className="flex shrink-0 flex-wrap items-center gap-x-sm gap-y-xs border-b border-border px-lg py-sm" data-automation-detail-head="true">
        <BackButton label={t("automationScreen.back")} onBack={() => useUiStore.getState().setAutomations({ selected: null, run: null })} />
        <h1 className="min-w-0 flex-1 truncate text-subhead font-semibold" title={automation.name}>{automation.name}</h1>
        <span className="flex shrink-0 flex-wrap items-center gap-xs">
          <RunNowButton automation={automation} actions={actions} />
          {automation.paused ? (
            <Button variant="secondary" size="sm" onClick={() => actions.automation("resume", automation.id)} data-automation-resume={automation.id}>
              <PlayIcon aria-hidden="true" />
              {t("automationScreen.resume")}
            </Button>
          ) : (
            <Button variant="secondary" size="sm" onClick={() => actions.automation("pause", automation.id)} data-automation-pause={automation.id}>
              <PauseIcon aria-hidden="true" />
              {t("automationScreen.pause")}
            </Button>
          )}
          <Button variant="secondary" size="sm" onClick={() => useUiStore.getState().setAutomations({ form: "edit" })} data-automation-edit={automation.id}>
            <PencilIcon aria-hidden="true" />
            {t("automationScreen.edit")}
          </Button>
          {canOpenFolder ? (
            <Button variant="secondary" size="sm" onClick={() => actions.openOwnedFile(automation.folder.path)} data-automation-open-folder={automation.id}>
              <FolderOpenIcon aria-hidden="true" />
              {t("automationScreen.openFolder")}
            </Button>
          ) : null}
          <RemoveButton automation={automation} actions={actions} />
        </span>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-lg py-md">
        <dl className="grid max-w-(--size-mission-panel-words) grid-cols-[max-content_minmax(0,1fr)] gap-x-lg gap-y-xs text-body" data-automation-facts="true">
          <Fact label={t("automationScreen.detail.task")}><span className="whitespace-pre-wrap text-foreground" data-automation-task="true">{automation.task}</span></Fact>
          <Fact label={t("automationScreen.detail.schedule")}>{scheduleText(automation.reads_as, words.language, t)}</Fact>
          <Fact label={t("automationScreen.detail.next")}><NextRunWords automation={automation} words={words} /></Fact>
          <Fact label={t("automationScreen.detail.folder")}>
            <span className="flex min-w-0 flex-col">
              <span>{automation.folder.default ? t("automationScreen.ownFolder") : lastName(automation.folder.path)}{automation.folder.worktree ? ` · ${t("automationScreen.worktree")}` : ""}</span>
              <span className="truncate font-mono text-caption text-muted-foreground" title={automation.folder.path} data-automation-folder-path="true">{automation.folder.path}</span>
            </span>
          </Fact>
          <Fact label={t("automationScreen.detail.agent")}>
            <span className="flex min-w-0 items-center gap-xs"><AgentMark kind={automation.agent.kind} /><span className="truncate">{agent}</span></span>
          </Fact>
        </dl>
        <h2 className="pt-lg pb-xs text-caption font-semibold text-muted-foreground">{t("automationScreen.detail.runs")}</h2>
        <Runs automation={automation} actions={actions} words={words} />
      </div>
    </>
  );
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-caption text-muted-foreground">{label}</dt>
      <dd className="min-w-0 text-subtle-foreground">{children}</dd>
    </>
  );
}

/**
 * The run list (B16-B18, B20-B22): newest first, each record's time, mark
 * and words, a manual or missed run tagged, a run talked to again or whose
 * conversation is gone said so. A run opens its terminal; a finished one
 * asks the core to open its conversation again (B17).
 */
function Runs({ automation, actions, words }: { automation: Automation; actions: Actions; words: Words }) {
  const { t } = words;
  const box = useRef<HTMLDivElement>(null);
  if (automation.records.length === 0) return <p className="text-caption text-muted-foreground" data-automation-runs="empty">{t("automationScreen.detail.noRuns")}</p>;
  const step = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    const rows = Array.from(box.current?.querySelectorAll<HTMLElement>("[data-automation-run-open]") ?? []);
    const index = rows.findIndex((row) => row === document.activeElement);
    const next = rows[index < 0 ? 0 : index + (event.key === "ArrowDown" ? 1 : -1)];
    if (!next) return;
    event.preventDefault();
    next.focus();
  };
  return (
    <div ref={box} className="flex max-w-(--size-task-table-wide) flex-col" onKeyDown={step} data-automation-runs={automation.records.length}>
      {automation.records.map((record, index) => <RunRow key={record.kind === "run" ? `run-${record.number}` : `${record.kind}-${index}`} automation={automation} record={record} actions={actions} words={words} />)}
    </div>
  );
}

function recordWhen(record: AutomationRecord): number | null {
  if (record.kind === "run") return record.due_unix_ms ?? record.started_at_unix_ms;
  if (record.kind === "skipped") return record.at_unix_ms;
  return null;
}

function RunRow({ automation, record, actions, words }: { automation: Automation; record: AutomationRecord; actions: Actions; words: Words }) {
  const { t } = words;
  const when = recordWhen(record);
  const whenWords = when === null ? null : whenText(when, words.now, words.zone, words.language, t);
  const shown = recordText(record, words.now, words.zone, words.language, t);
  const body = (
    <>
      {whenWords ? <span className="w-(--size-mission-table-project) shrink-0 truncate text-caption tabular-nums text-subtle-foreground">{whenWords}</span> : null}
      <span className="flex min-w-0 flex-1 items-center gap-xs">
        <RecordWords tone={shown.tone} text={shown.text} />
        {record.kind === "run" ? <RunTags run={record} /> : null}
      </span>
    </>
  );
  const data = { "data-automation-record": record.kind, "data-automation-record-number": record.kind === "run" ? record.number : undefined, "data-automation-run-state": record.kind === "run" ? record.state : undefined, "data-automation-run-reason": record.kind === "run" ? record.reason : undefined };
  if (record.kind !== "run") return <div className="flex min-w-0 items-center gap-sm px-xs py-xs" {...data}>{body}</div>;
  // The run's view opens a finished run's conversation again as it arrives (B17).
  const open = () => actions.openAutomation(automation.id, record.number);
  return (
    <RowButton className="flex min-w-0 items-center gap-sm px-xs py-xs" aria-label={t("automationScreen.run.open", { when: whenWords ?? "" })} onClick={open} data-automation-run-open={record.number} {...data}>
      {body}
    </RowButton>
  );
}

/** `수동`, `놓친 회차`, `이어서 대화 중`, `대화를 찾을 수 없음` (B17-B20). */
function RunTags({ run }: { run: AutomationRun }) {
  const { t } = useInterfaceTranslation();
  const tags: [string, string][] = [];
  if (run.trigger === "manual") tags.push(["manual", t("automationScreen.run.manual")]);
  if (run.trigger === "missed") tags.push(["missed", t("automationScreen.run.missed")]);
  if (run.opened) tags.push(["opened", t("automationScreen.run.opened")]);
  if (run.conversation_missing) tags.push(["missing", t("automationScreen.run.conversationMissing")]);
  return (
    <>
      {tags.map(([kind, text]) => (
        <Badge key={kind} variant="outline" className={kind === "opened" ? "text-agent-working" : undefined} data-automation-run-tag={kind}>{text}</Badge>
      ))}
    </>
  );
}

/** The pane a run's terminal is, wherever its tab is; none until the core has it. */
function paneRowOf(rest: SnapshotRest | null, paneId: string | undefined): PaneRow | null {
  if (!paneId) return null;
  for (const workspace of catalogWorkspaces(rest)) {
    for (const checkout of workspace.checkouts) {
      for (const tab of checkout.tabs) {
        const pane = tab.panes.find((row) => row.id === paneId);
        if (pane) return pane;
      }
    }
  }
  return null;
}

/** How often a showing run terminal tells the core again that it shows: the core holds a viewing for 30 minutes (`VIEWING_HOLD_MS`). */
const VIEWING_REPEAT_MS = 5 * 60_000;

/**
 * A run's terminal in the automation screen (B16-B18): the run going, live
 * and typed into, with [■ 멈추기]; a finished one's conversation opened
 * again, until it sleeps by Hide's sleep rules, when a press opens it again.
 * While it shows, the core keeps it open (`automation_run_viewing`, B15).
 */
function RunView({ automation, run, actions }: { automation: Automation; run: Extract<AutomationRecord, { kind: "run" }>; actions: Actions }) {
  const words = useWords();
  const { t } = words;
  const live = useShellStore((s) => s.connection === "live");
  const going = run.state === "running" || run.state === "starting";
  const pane = useShellStore((s) => paneRowOf(s.rest, run.pane_id));
  const opening = useRunOpening(automation.id, run.number);
  // Arriving at a finished run whose terminal is closed is a press on it, which opens its conversation again (B17);
  // once here, a terminal that slept waits for [다시 열기] (B18).
  const openable = !going && !pane && !run.pane_id && !run.opened && run.resumable && !run.conversation_missing;
  const arrived = useRef(false);
  useEffect(() => {
    if (arrived.current || !live) return;
    arrived.current = true;
    if (openable) opening.open();
  }, [live, openable, opening]);
  // Said again well inside the core's hold, so a terminal watched longer than it stays open (B15).
  useEffect(() => {
    if (!live) return undefined;
    const say = () => actions.automationRunViewing(automation.id, run.number, true);
    say();
    const again = window.setInterval(say, VIEWING_REPEAT_MS);
    return () => {
      window.clearInterval(again);
      actions.automationRunViewing(automation.id, run.number, false);
    };
  }, [actions, automation.id, run.number, live]);
  const shown = recordText(run, words.now, words.zone, words.language, t);
  const when = whenText(run.due_unix_ms ?? run.started_at_unix_ms, words.now, words.zone, words.language, t);
  let body: ReactNode;
  if (pane) body = <RunTerminal pane={pane} actions={actions} />;
  else if (run.conversation_missing || (!going && !run.resumable)) body = <Line data="unavailable">{run.conversation_missing ? t("automationScreen.run.conversationMissing") : t("automationScreen.run.noConversation")}</Line>;
  else if (going || run.opened || opening.state.kind === "asking") body = <Line data="opening">{t("automationScreen.run.opening")}</Line>;
  else {
    const refused = opening.state.kind === "refused" ? opening.state.code : null;
    body = (
      <div className="flex flex-col items-start gap-sm px-lg py-md" data-automation-run-terminal="closed" data-automation-run-refusal={refused ?? undefined}>
        <p className={cn("text-caption", refused ? "text-destructive" : "text-muted-foreground")} role={refused ? "alert" : undefined}>
          {refused === null ? t("automationScreen.run.closed") : refused === "automation_opening" ? t("automationScreen.run.openingAnother") : t("automationScreen.run.openRefused")}
        </p>
        <Button variant="secondary" size="sm" onClick={opening.open} data-automation-run-reopen={run.number}>{t("automationScreen.run.reopen")}</Button>
      </div>
    );
  }
  return (
    <>
      <div className="flex shrink-0 flex-wrap items-center gap-x-sm gap-y-xs border-b border-border px-lg py-sm" data-automation-run-head={run.number} data-automation-run-state={run.state}>
        <BackButton label={automation.name} onBack={() => useUiStore.getState().setAutomations({ run: null })} />
        <h1 className="shrink-0 text-subhead font-semibold">{t("automationScreen.run.title", { when })}</h1>
        <span className="flex min-w-0 flex-1 items-center gap-xs">
          <RecordWords tone={shown.tone} text={shown.text} />
          <RunTags run={run} />
        </span>
        {going ? (
          <Button variant="secondary" size="sm" onClick={() => actions.automationRunStop(automation.id, run.number)} data-automation-run-stop={run.number}>
            <SquareIcon aria-hidden="true" className="fill-current" />
            {t("automationScreen.run.stop")}
          </Button>
        ) : null}
      </div>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-automation-run-body={run.number}>{body}</div>
    </>
  );
}

/**
 * Asks the core to open a finished run's conversation again (B17, B18) and
 * waits for its answer: one opening of an automation starts at a time, so a
 * second is refused `automation_opening`, which the run view says. While the
 * call waits the view reads as opening; the section then says what opened.
 */
function useRunOpening(id: string, run: number): { state: RunOpeningState; open: () => void } {
  const [state, setState] = useState<RunOpeningState>({ kind: "idle" });
  const open = useCallback(() => {
    setState({ kind: "asking" });
    void callAutomations({ call: "run_open", id, run }).then((answer) => {
      // A dropped socket says nothing about the run; the next press asks again.
      setState(answer.ok || answer.code === "socket_closed" ? { kind: "idle" } : { kind: "refused", code: answer.code });
    });
  }, [id, run]);
  return useMemo(() => ({ state, open }), [state, open]);
}

type RunOpeningState = { kind: "idle" } | { kind: "asking" } | { kind: "refused"; code: string };

function Line({ data, children }: { data: string; children: ReactNode }) {
  return <p className="px-lg py-md text-caption text-muted-foreground" data-automation-run-terminal={data}>{children}</p>;
}

/** The run's pane as the canvas draws one, its header, marks and sleep included; the run owns its life, so the head has no menu and no ✕. */
function RunTerminal({ pane, actions }: { pane: PaneRow; actions: Actions }) {
  const transport = useShellStore((s) => s.rest?.terminal?.panes?.find((row) => row.pane_id === pane.id));
  const scale = useShellStore((s) => s.rest?.ui_state?.pane_text_scales?.[pane.id] ?? 1);
  const agent = useShellStore((s) => s.agents.find((row) => row.pane_id === pane.id));
  const focused = useShellStore((s) => (s.rest ? focusedPaneOf(s.rest) === pane.id : false));
  const props = useMemo(() => agentProps(agent), [agent]);
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-automation-run-terminal="pane" data-automation-run-pane={pane.id}>
      <PaneView pane={pane} transport={transport} focused={focused} scale={scale} actions={actions} {...props} paneCount={1} zoomed={false} owned />
    </div>
  );
}

/**
 * Delete (B27, design principle 6): a confirm naming what goes and what
 * stays, with the run going said; no button is the default.
 */
function RemoveButton({ automation, actions }: { automation: Automation; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const runs = automation.records.filter((record) => record.kind === "run").length;
  const going = automation.running || automation.records.some((record) => record.kind === "run" && record.opened);
  return (
    <>
      <Button variant="ghost" size="sm" onClick={() => setOpen(true)} data-automation-remove={automation.id}>
        <Trash2Icon aria-hidden="true" />
        {t("automationScreen.remove")}
      </Button>
      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogContent data-automation-remove-confirm={automation.id}>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("automationScreen.removeConfirm.title", { name: automation.name, count: runs })}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("automationScreen.removeConfirm.body")}
              {going ? ` ${t("automationScreen.removeConfirm.running")}` : ""}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel data-automation-remove-cancel="true">{t("automationScreen.removeConfirm.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                actions.automation("remove", automation.id);
                useUiStore.getState().setAutomations({ selected: null, run: null, form: null });
              }}
              data-automation-remove-yes="true"
            >
              {t("automationScreen.removeConfirm.confirm")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
