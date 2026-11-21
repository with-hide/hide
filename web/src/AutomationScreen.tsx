import { CheckIcon, ChevronLeftIcon, ChevronRightIcon, CircleAlertIcon, FolderOpenIcon, InboxIcon, MinusIcon, PackageIcon, PauseIcon, PencilIcon, PlayIcon, PlusIcon, RepeatIcon, SquareIcon, Trash2Icon, XIcon, type LucideIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { agentProductName } from "./agentAdapters";
import { callAutomations } from "./automationCall";
import { AUTOMATION_EXAMPLE_KEYS, AUTOMATION_EXAMPLES, shownOwnFolder, type AutomationExample } from "./automationDraft";
import { AutomationForm, EXAMPLE_WORDS } from "./AutomationForm";
import { settledPlace } from "./automationPlace";
import { viewRun } from "./automationRunView";
import { lastResult, pauseReasonText, recentSpendText, recordText, scheduleText, skippedFolded, spendText, whenText, type RecordTone } from "./automationWords";
import { AgentLogo } from "./components/agent-logo";
import { LookChip } from "./components/linked-view";
import { formatElapsed, useMinuteClock } from "./components/elapsed";
import { EmptyWelcome } from "./components/empty-welcome";
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
import { ViewAreas } from "./ViewAreas";
import { workspaceViewOf } from "./workspace";
import { catalogWorkspaces, type Automation, type AutomationRecord, type AutomationRun, type AutomationSpend, type AutomationsSection, type PaneRow, type SnapshotRest } from "./snapshot";
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
        <AutomationForm key={editing?.id ?? `new-${place.example ?? ""}`} section={section} automation={editing} example={place.example} actions={actions} />
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

/** `자동화` with its count, what the screen holds in one quiet line, and [+ 새 자동화] (B2, B4); with none, the empty screen says it instead. */
function Head({ count }: { count: number | null }) {
  const { t } = useInterfaceTranslation();
  return (
    <header className="flex shrink-0 items-center gap-md px-lg pt-lg pb-md" data-automation-screen-head="true">
      <div className="flex min-w-0 flex-1 flex-col gap-xxs">
        <h1 className="flex min-w-0 items-center gap-xs">
          <span className="truncate text-headline font-semibold text-foreground">{t("automationScreen.title")}</span>
          {count !== null && count > 0 ? <Badge className="tabular-nums" data-automation-count={count}>{count}</Badge> : null}
        </h1>
        <p className="truncate text-caption text-muted-foreground">{t("automationScreen.subtitle")}</p>
      </div>
      {count !== null && count > 0 ? <NewButton /> : null}
    </header>
  );
}

function NewButton({ primary = false }: { primary?: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <Button variant={primary ? "default" : "secondary"} size={primary ? "default" : "sm"} onClick={() => useUiStore.getState().setAutomations({ selected: null, run: null, form: "new" })} data-automation-new="true">
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

/** Quiet cards before the core's automations first arrive: no count, no words. */
function Skeleton() {
  return (
    <div className={cn(LIST_GRID, "px-lg py-md")} aria-hidden="true" data-automation-screen-state="loading">
      {[0, 1, 2].map((card) => (
        <div key={card} className="flex flex-col gap-sm rounded-lg border border-border p-md">
          <span className="h-(--size-badge-height) w-(--size-mission-table-project) rounded-xs bg-secondary" />
          <span className="h-(--size-badge-height) w-(--size-mission-table-steps) rounded-xs bg-secondary" />
          <span className="h-(--size-badge-height) rounded-xs bg-secondary" />
          <span className="h-(--size-badge-height) rounded-xs bg-secondary" />
        </div>
      ))}
    </div>
  );
}

/** The list's cards, as many to a line as the screen holds at their least width. */
const LIST_GRID = "grid grid-cols-[repeat(auto-fill,minmax(var(--size-automation-card),1fr))] gap-md";

/** Each starting point's mark on the empty screen. */
const EXAMPLE_ICONS = { issues: InboxIcon, deps: PackageIcon, ci: CircleAlertIcon } as const satisfies Record<AutomationExample, LucideIcon>;

/**
 * The list (B2, B3, B38): each automation in the core's order as a card of
 * the agent's logo, its name with the unseen dot and ▶ to run it now, its
 * next run over its schedule, its task in two lines, and its last results
 * beside its folder; a paused one dimmed. With none, Hide AI's owl says what
 * an automation does, with [+ 새 자동화] and three starting points. The
 * arrows move between the cards in order, Enter opens one.
 */
function Table({ section, actions }: { section: AutomationsSection; actions: Actions }) {
  const box = useRef<HTMLDivElement>(null);
  // Back from a detail or the form, whose button is gone, the keys land on the first row or [+ 새 자동화] (B38).
  useEffect(() => {
    const active = document.activeElement;
    if (active && active !== document.body && active.isConnected) return;
    box.current?.querySelector<HTMLElement>("[data-automation-open], [data-automation-new]")?.focus();
  }, []);
  if (section.items.length === 0) {
    return (
      <div ref={box} className="flex min-h-0 flex-1 flex-col overflow-y-auto">
        <Empty />
      </div>
    );
  }
  const step = (event: KeyboardEvent<HTMLDivElement>) => {
    const forward = event.key === "ArrowDown" || event.key === "ArrowRight";
    if (!forward && event.key !== "ArrowUp" && event.key !== "ArrowLeft") return;
    const cards = Array.from(box.current?.querySelectorAll<HTMLElement>("[data-automation-open]") ?? []);
    const index = cards.findIndex((card) => card === document.activeElement);
    const next = cards[index < 0 ? 0 : index + (forward ? 1 : -1)];
    if (!next) return;
    event.preventDefault();
    next.focus();
  };
  return (
    <div ref={box} className="min-h-0 flex-1 overflow-y-auto px-lg pb-lg" onKeyDown={step} data-automation-screen-state="table">
      <div className={LIST_GRID}>
        {section.items.map((automation) => <AutomationCard key={automation.id} automation={automation} actions={actions} />)}
      </div>
    </div>
  );
}

/** The empty screen (B2): the owl and what an automation does, [+ 새 자동화], three starting points the form opens on, and that an agent can make one when asked. */
function Empty() {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  return (
    <EmptyWelcome pose="ai" title={t("automationScreen.empty")} detail={t("automationScreen.emptyDetail")} data-automation-screen-state="empty">
      <NewButton primary />
      <div className="flex w-full max-w-(--size-task-table-wide) flex-col gap-xs pt-md text-left">
        <span className="px-xs text-caption text-muted-foreground">{t("automationScreen.examples")}</span>
        <div className="flex flex-col divide-y divide-border overflow-hidden rounded-lg border border-border">
          {AUTOMATION_EXAMPLE_KEYS.map((key) => {
            const Icon = EXAMPLE_ICONS[key];
            const name = t(EXAMPLE_WORDS[key].name);
            return (
              <RowButton key={key} className="gap-md rounded-none px-md py-sm" aria-label={t("automationScreen.exampleOpen", { name })}
                onClick={() => useUiStore.getState().setAutomations({ selected: null, run: null, form: "new", example: key })} data-automation-example={key}>
                <span aria-hidden="true" className="flex size-(--size-control-lg) shrink-0 items-center justify-center rounded-md bg-secondary text-subtle-foreground"><Icon className="size-(--size-icon)" /></span>
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate text-body font-medium text-foreground">{name}</span>
                  <span className="truncate text-caption text-muted-foreground">{t(EXAMPLE_WORDS[key].task)}</span>
                </span>
                <span className="flex shrink-0 items-center gap-xxs text-caption text-subtle-foreground"><RepeatIcon aria-hidden="true" className="size-(--size-icon-sm)" />{scheduleText(AUTOMATION_EXAMPLES[key], language, t)}</span>
              </RowButton>
            );
          })}
        </div>
      </div>
      <p className="max-w-(--size-task-table-wide) break-keep text-caption text-muted-foreground">{t("automationScreen.emptyTalk")}</p>
    </EmptyWelcome>
  );
}

function AutomationCard({ automation, actions }: { automation: Automation; actions: Actions }) {
  const words = useWords();
  const { t } = words;
  const agent = agentProductName(automation.agent.kind);
  return (
    <div className={cn("relative flex min-w-0 flex-col gap-sm rounded-lg border border-border bg-card p-md", automation.paused?.reason === "operator" && "opacity-(--opacity-dimmed)")} data-automation-row={automation.id} data-automation-row-paused={automation.paused ? automation.paused.reason : undefined}>
      <RowOverlayButton className="rounded-lg" aria-label={t("automationScreen.open", { name: automation.name })} onClick={() => actions.openAutomation(automation.id)} data-automation-open={automation.id} />
      <span className="pointer-events-none relative flex min-w-0 items-center gap-xs">
        <AgentLogo agent={automation.agent.kind} label={agent} />
        <span className="min-w-0 truncate text-body font-semibold text-foreground" title={automation.name}>{automation.name}</span>
        {automation.unseen > 0 ? <span role="img" aria-label={t("automationScreen.unseen")} className="size-(--size-tab-status-dot) shrink-0 rounded-full bg-primary" data-automation-unseen={automation.unseen} /> : null}
        <StateBadge automation={automation} />
        <span className="pointer-events-auto ml-auto flex shrink-0"><RunNowButton automation={automation} actions={actions} icon /></span>
      </span>
      <span className="pointer-events-none relative flex min-w-0 flex-col gap-xxs">
        <span className="sr-only">{t("automationScreen.col.next")}</span>
        <NextTime automation={automation} words={words} large />
        <span className="flex min-w-0 items-center gap-xs text-caption text-subtle-foreground">
          <RepeatIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
          <span className="min-w-0 truncate">{scheduleText(automation.reads_as, words.language, t)}</span>
        </span>
      </span>
      <span className="pointer-events-none relative line-clamp-2 break-words text-caption text-muted-foreground" title={automation.task}>{automation.task}</span>
      <span className="pointer-events-none relative mt-auto flex min-w-0 items-center gap-sm border-t border-border pt-sm text-caption text-subtle-foreground">
        <span className="sr-only">{t("automationScreen.recent")}</span>
        <span className="flex min-w-0 flex-1"><RecentRuns automation={automation} words={words} /></span>
        <span className="flex min-w-0 shrink items-center gap-xs text-muted-foreground"><Where automation={automation} /><span className="sr-only">, {agent}</span></span>
      </span>
    </div>
  );
}

/** What the automation is doing, by its name: a run going, paused by the operator, or stopped for a person with why (B3, B23, B25); one that waits for its time says nothing. */
function StateBadge({ automation }: { automation: Automation }) {
  const { t } = useInterfaceTranslation();
  if (automation.running) {
    return (
      <Badge variant="outline" className="text-agent-working" data-automation-state="running">
        <span aria-hidden="true" className="size-(--size-tab-status-dot) rounded-full bg-agent-working" />
        {t("automation.next.running")}
      </Badge>
    );
  }
  if (!automation.paused) return null;
  if (automation.paused.reason === "operator") return <Badge data-automation-state="paused"><PauseIcon aria-hidden="true" />{t("automation.next.paused")}</Badge>;
  return <Badge variant="outline" className="text-destructive" data-automation-state="stopped">{pauseReasonText(automation.paused.reason, t)}</Badge>;
}

/** `내일 09:00 · 15시간 뒤`, the time `large` on a card; a paused automation or a schedule that never runs again has no next time (B3, B25). */
function NextTime({ automation, words, large = false }: { automation: Automation; words: Words; large?: boolean }) {
  const at = automation.paused ? null : automation.next_run_at_unix_ms;
  if (at == null) return <span className={cn("text-muted-foreground", large ? "text-subhead" : "text-caption")} data-automation-next="none">{automation.paused ? "—" : words.t("automation.next.none")}</span>;
  return (
    <span className="flex min-w-0 items-baseline gap-xs text-caption" data-automation-next="at">
      <span className={cn("shrink-0 tabular-nums text-foreground", large && "text-subhead font-semibold")}>{whenText(at, words.now, words.zone, words.language, words.t)}</span>
      {/* The clock moves by the minute, so the last minute, and a time the core is about to take, read as soon rather than a count that stands still. */}
      <span className="min-w-0 truncate text-muted-foreground">{at - words.now < 60_000 ? words.t("automationScreen.soon") : words.t("automationScreen.inTime", { time: formatElapsed(words.language, at - words.now) })}</span>
    </span>
  );
}

/** How many of the newest results the list draws as marks. */
const RECENT_MARKS = 5;

/** The newest results as marks, oldest first (✓ ✗ – ■, ● while one goes), then the newest one's words (B3). */
function RecentRuns({ automation, words }: { automation: Automation; words: Words }) {
  const recent = automation.records.filter((record) => record.kind !== "missed").slice(0, RECENT_MARKS);
  if (recent.length === 0) return <span className="text-caption text-muted-foreground">{words.t("automationScreen.noRunYet")}</span>;
  const last = lastResult(automation);
  const lastWords = last ? recordText(last, words.now, words.zone, words.language, words.t) : null;
  return (
    <span className="flex min-w-0 items-center gap-xs" data-automation-recent={recent.length}>
      <span aria-hidden="true" className="flex shrink-0 items-center">
        {[...recent].reverse().map((record, index) => <ToneMark key={index} tone={recordText(record, words.now, words.zone, words.language, words.t).tone} />)}
      </span>
      {lastWords ? <span className={cn("min-w-0 truncate text-caption", lastWords.tone === "failed" ? "text-destructive" : "text-subtle-foreground")} title={lastWords.text}>{lastWords.text}</span> : null}
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

/** `기본 폴더`, `hide · 새 워크트리`: where a run works; the row's logo names the agent. */
function Where({ automation }: { automation: Automation }) {
  const { t } = useInterfaceTranslation();
  const folder = automation.folder.default ? t("automationScreen.ownFolder") : lastName(automation.folder.path);
  return (
    <>
      <span className="max-w-(--size-mission-table-project) shrink-0 truncate" title={automation.folder.path}>{folder}</span>
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
  const runCount = automation.records.filter((record) => record.kind === "run").length;
  // Its own folder reads `~/hide/automations/<이름>-<id>` as the form named it, so the name at its end is not cut (B5).
  const ownRoot = useShellStore((s) => s.automations?.own_folder_root);
  const back = useRef<HTMLDivElement>(null);
  // Opened from the table, the keys land in the detail, so Escape-less keyboard use goes on from its first action (B38).
  useEffect(() => {
    const active = document.activeElement;
    if (!active || active === document.body || !active.isConnected) back.current?.querySelector<HTMLElement>("button")?.focus();
  }, []);
  return (
    <>
      <div ref={back} className="flex shrink-0 flex-col gap-md px-lg pt-sm pb-md" data-automation-detail-head="true">
        <div className="flex flex-wrap items-center gap-x-sm gap-y-xs">
          <BackButton label={t("automationScreen.back")} onBack={() => useUiStore.getState().setAutomations({ selected: null, run: null })} />
          <span className="flex min-w-0 flex-1 flex-wrap items-center justify-end gap-xs">
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
        <div className="flex min-w-0 items-start gap-md">
          <span className="flex pt-xxs"><AgentLogo agent={automation.agent.kind} label={agentProductName(automation.agent.kind)} /></span>
          <div className="flex min-w-0 flex-1 flex-col gap-xs">
            <h1 className="flex min-w-0 items-center gap-xs">
              <span className="min-w-0 truncate text-headline font-semibold text-foreground" title={automation.name}>{automation.name}</span>
              <StateBadge automation={automation} />
            </h1>
            <p className="max-w-(--size-editor-measure) whitespace-pre-wrap text-body text-subtle-foreground" data-automation-task="true">{automation.task}</p>
            {automation.recent_spend ? <p className="text-caption tabular-nums text-muted-foreground" data-automation-recent-spend={automation.recent_spend.runs}>{recentSpendText(automation.recent_spend, words.language, t)}</p> : null}
          </div>
        </div>
      </div>
      <div className="flex min-h-0 flex-1 flex-col gap-lg overflow-y-auto px-lg pb-lg">
        <dl className="grid shrink-0 grid-cols-2 gap-px overflow-hidden rounded-lg border border-border bg-border missions-split:grid-cols-4" data-automation-facts="true">
          <Fact label={t("automationScreen.detail.schedule")}>
            <span className="flex min-w-0 items-center gap-xs"><RepeatIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" /><span className="truncate">{scheduleText(automation.reads_as, words.language, t)}</span></span>
          </Fact>
          <Fact label={t("automationScreen.detail.next")}><NextTime automation={automation} words={words} /></Fact>
          <Fact label={t("automationScreen.detail.folder")}>
            <span className="flex min-w-0 flex-col">
              <span className="truncate">{automation.folder.default ? t("automationScreen.ownFolder") : lastName(automation.folder.path)}{automation.folder.worktree ? ` · ${t("automationScreen.worktree")}` : ""}</span>
              <span className="truncate font-mono text-micro text-muted-foreground" title={automation.folder.path} data-automation-folder-path="true">{shownOwnFolder(ownRoot, automation.folder.path)}</span>
            </span>
          </Fact>
          <Fact label={t("automationScreen.detail.agent")}>
            <span className="flex min-w-0 items-center gap-xs"><AgentMark kind={automation.agent.kind} /><span className="truncate">{agent}</span></span>
          </Fact>
        </dl>
        <section className="flex min-w-0 flex-col gap-xs" aria-label={t("automationScreen.detail.runs")}>
          <h2 className="flex items-center gap-xs text-subhead font-semibold text-foreground">
            {t("automationScreen.detail.runs")}
            {runCount > 0 ? <Badge className="tabular-nums">{runCount}</Badge> : null}
          </h2>
          <Runs automation={automation} actions={actions} words={words} />
        </section>
      </div>
    </>
  );
}

/** One fact of the detail's card: its quiet name over its value. */
function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-xxs bg-background px-md py-sm">
      <dt className="text-micro text-muted-foreground">{label}</dt>
      <dd className="min-w-0 text-body text-foreground">{children}</dd>
    </div>
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
  if (automation.records.length === 0) return <p className="rounded-lg border border-dashed border-border px-md py-md text-caption text-muted-foreground" data-automation-runs="empty">{t("automationScreen.detail.noRuns")}</p>;
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
    <div ref={box} className="flex flex-col divide-y divide-border overflow-hidden rounded-lg border border-border" onKeyDown={step} data-automation-runs={automation.records.length}>
      {automation.records.map((record, index) => <RunRow key={record.kind === "run" ? `run-${record.number}` : `${record.kind}-${index}`} automation={automation} record={record} actions={actions} words={words} />)}
    </div>
  );
}

function recordWhen(record: AutomationRecord): number | null {
  if (record.kind === "run") return record.due_unix_ms ?? record.started_at_unix_ms;
  if (record.kind === "skipped") return skippedFolded(record) ? null : record.at_unix_ms;
  return null;
}

function RunRow({ automation, record, actions, words }: { automation: Automation; record: AutomationRecord; actions: Actions; words: Words }) {
  const { t } = words;
  const when = recordWhen(record);
  const whenWords = when === null ? null : whenText(when, words.now, words.zone, words.language, t);
  const shown = recordText(record, words.now, words.zone, words.language, t);
  // What the run's agent opened that the operator has not looked at (operator decision 2026-11-21).
  const paneId = record.kind === "run" ? record.pane_id : undefined;
  const look = useShellStore((s) => (paneId ? s.agents.find((row) => row.pane_id === paneId)?.look : undefined));
  const body = (
    <>
      {whenWords ? <span className="w-(--size-mission-table-project) shrink-0 truncate text-caption tabular-nums text-subtle-foreground">{whenWords}</span> : null}
      <span className="flex min-w-0 flex-1 items-center gap-xs">
        <RecordWords tone={shown.tone} text={shown.text} />
        {record.kind === "run" ? <RunTags run={record} /> : null}
        {look && paneId ? <LookChip look={look} onOpen={() => actions.openAgent(paneId, look)} /> : null}
      </span>
      {record.kind === "run" && record.spend ? <RunSpend spend={record.spend} words={words} /> : null}
    </>
  );
  const data = { "data-automation-record": record.kind, "data-automation-record-number": record.kind === "run" ? record.number : undefined, "data-automation-run-state": record.kind === "run" ? record.state : undefined, "data-automation-run-reason": record.kind === "run" ? record.reason : undefined };
  if (record.kind !== "run") return <div className="flex min-w-0 items-center gap-md px-md py-sm" {...data}>{body}</div>;
  // The run's view opens a finished run's conversation again as it arrives (B17).
  const open = () => actions.openAutomation(automation.id, record.number);
  return (
    <div className="relative flex min-w-0" {...data}>
      <RowOverlayButton className="rounded-none" aria-label={t("automationScreen.run.open", { when: whenWords ?? "" })} onClick={open} data-automation-run-open={record.number} />
      <div className="pointer-events-none relative flex min-w-0 flex-1 items-center gap-md px-md py-sm">
        {body}
        <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
      </div>
    </div>
  );
}

/** What a run's conversation spent, `토큰 12.3K`, once the ledger read it (B39). */
function RunSpend({ spend, words }: { spend: AutomationSpend; words: Words }) {
  return <span className="shrink-0 text-caption tabular-nums text-muted-foreground" data-automation-run-spend={spend.tokens}>{spendText(spend, words.language, words.t)}</span>;
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
  return placeOf(rest, paneId)?.pane ?? null;
}

/** The pane a run's terminal is and the tab that holds it, whose desk keeps what the run's agent opened. */
function placeOf(rest: SnapshotRest | null, paneId: string | undefined): { pane: PaneRow; tabId: string | null } | null {
  if (!paneId) return null;
  for (const workspace of catalogWorkspaces(rest)) {
    for (const checkout of workspace.checkouts) {
      for (const tab of checkout.tabs) {
        const pane = tab.panes.find((row) => row.id === paneId);
        if (pane) return { pane, tabId: tab.id ?? null };
      }
    }
  }
  return null;
}

/**
 * Whether the desk in front is the run's tab's and holds something: the
 * window moved the core's focus to the run's pane (`showAutomationRunPane`),
 * so the front Workspace's desk is the one its agent opened Views on.
 */
function useRunDesk(tabId: string | null): boolean {
  return useShellStore((s) => {
    const view = workspaceViewOf(s.rest);
    return tabId !== null && view?.tray.tab_id === tabId && (view.layout?.display_count ?? 0) > 0;
  });
}

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
  const tabId = useShellStore((s) => placeOf(s.rest, run.pane_id)?.tabId ?? null);
  const desk = useRunDesk(tabId);
  const opening = useRunOpening(automation.id, run.number);
  // The run's pane takes the core's focus as it arrives, so its desk is the one in front; however the screen is
  // left, the leaving names the pane the operator was on, which the core gives the focus back to unless a press
  // since moved it elsewhere (operator decision 2026-11-21).
  const paneId = pane?.id ?? null;
  const before = useRef<string | null>(null);
  useEffect(() => {
    if (!live || !paneId) return;
    const rest = useShellStore.getState().rest;
    const focused = rest ? focusedPaneOf(rest) : null;
    if (before.current === null && focused !== paneId) before.current = focused;
    actions.showAutomationRunPane(paneId);
  }, [actions, live, paneId]);
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
  useEffect(() => (live ? viewRun((on) => actions.automationRunViewing(automation.id, run.number, on, on ? null : before.current)) : undefined), [actions, automation.id, run.number, live]);
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
        {run.spend ? <RunSpend spend={run.spend} words={words} /> : null}
        {going ? (
          <Button variant="secondary" size="sm" onClick={() => actions.automationRunStop(automation.id, run.number)} data-automation-run-stop={run.number}>
            <SquareIcon aria-hidden="true" className="fill-current" />
            {t("automationScreen.run.stop")}
          </Button>
        ) : null}
      </div>
      {/* Pair view's shape from the start: the terminal on the left, on the right what the run's agent opened, else the run's summary (operator decision 2026-11-21). */}
      <div className="flex min-h-0 min-w-0 flex-1" data-automation-run-body={run.number}>
        <div className="flex min-h-0 min-w-0 flex-1 basis-0 flex-col" data-automation-run-side="terminal">{body}</div>
        <div className="flex min-h-0 min-w-0 flex-1 basis-0 flex-col border-l border-border bg-card" role="region" aria-label={desk ? t("panes.lane.documents") : t("automationScreen.run.summary")} data-automation-run-side={desk ? "desk" : "summary"} data-view-area={desk ? "true" : undefined}>
          {desk ? <ViewAreas actions={actions} /> : <RunSummary automation={automation} run={run} going={going} actions={actions} />}
        </div>
      </div>
    </>
  );
}

/** The answer card's edge, in the run's tone. */
const TONE_EDGE: Record<RecordTone, string> = {
  ok: "border-success",
  failed: "border-destructive",
  skipped: "border-border",
  running: "border-agent-working",
  stopped: "border-border",
};

/**
 * The right side while the run's agent opened nothing (operator decision
 * 2026-11-21, card A): how the run ended in large, when it started, how and
 * with which agent; the agent's last answer as the body; the task folded to
 * three lines; and the automation's last runs, each a press to that run.
 */
function RunSummary({ automation, run, going, actions }: { automation: Automation; run: Extract<AutomationRecord, { kind: "run" }>; going: boolean; actions: Actions }) {
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
      <section className="flex flex-col gap-xs">
        <h3 className="text-micro text-muted-foreground">{t("automationScreen.run.summaryAnswer")}</h3>
        {going ? (
          <p className="text-caption text-muted-foreground">{t("automationScreen.run.summaryGoing")}</p>
        ) : run.answer ? (
          <p className={cn("whitespace-pre-wrap break-words rounded-r-md border-l-2 bg-secondary px-md py-sm text-body text-foreground", TONE_EDGE[shown.tone])} data-automation-run-answer="true">{run.answer}</p>
        ) : (
          <p className="text-caption text-muted-foreground">{t("automationScreen.run.summaryNoAnswer")}</p>
        )}
      </section>
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
      <PaneView pane={pane} transport={transport} focused={focused} scale={scale} actions={actions} {...props} owned />
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
