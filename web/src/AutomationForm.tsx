import { ChevronLeftIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { callAutomations, type SchedulePreview } from "./automationCall";
import { addCall, draftOf, draftSchedule, editCall, emptyField, exampleDraft, refusalField, refusalKey, shownOwnFolder, type AutomationExample, type FormDraft, type ScheduleDraft, type ScheduleKind } from "./automationDraft";
import { scheduleText, whenText, WEEKDAYS, weekdayText } from "./automationWords";
import { AgentPicker } from "./components/agent-picker";
import { useMinuteClock } from "./components/elapsed";
import { Button } from "./components/ui/button";
import { Checkbox } from "./components/ui/checkbox";
import { Input } from "./components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./components/ui/select";
import { Textarea } from "./components/ui/textarea";
import { ToggleGroup, ToggleGroupItem } from "./components/ui/toggle-group";
import { selectKind } from "./agentPicker";
import { startItems } from "./agentProfiles";
import { effortsOf } from "./conductorSettings";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { listedWorkspaces, localDeviceId, type Automation, type AutomationSchedule, type AutomationsSection, type AutomationWeekday, type Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// The automation form (PRD automations B4-B8, B26): name, task, the agent
// as an agent start lists it, the folder, the schedule with what it reads as
// and when it runs next. The schedule is read by the core as it is typed
// (`preview`); a value the core refuses is one red line under its field and
// [만들기] waits until it reads. A make opens the new automation's detail,
// with its manual run's terminal when it was asked to run now (B8).

const OWN_FOLDER = "__own";
const DEFAULT_EFFORT = "__default";
const KINDS: readonly ScheduleKind[] = ["daily", "weekdays", "weekly", "every_minutes", "every_hours", "cron"];
const KIND_KEY = {
  daily: "automationScreen.form.kind.daily",
  weekdays: "automationScreen.form.kind.weekdays",
  weekly: "automationScreen.form.kind.weekly",
  every_minutes: "automationScreen.form.kind.everyMinutes",
  every_hours: "automationScreen.form.kind.everyHours",
  cron: "automationScreen.form.kind.cron",
} as const satisfies Record<ScheduleKind, string>;
/** The words of each starting point the empty screen offers (B2); the screen's list and the form read the same ones. */
export const EXAMPLE_WORDS = {
  issues: { name: "automationScreen.example.issues.name", task: "automationScreen.example.issues.task" },
  deps: { name: "automationScreen.example.deps.name", task: "automationScreen.example.deps.task" },
  ci: { name: "automationScreen.example.ci.name", task: "automationScreen.example.ci.task" },
} as const satisfies Record<AutomationExample, { name: string; task: string }>;
/**
 * The minutes the form offers, a copy of `EVERY_MINUTES` in
 * `hide-core/src/automations/schedule.rs` (D-06), which neither the preview
 * nor the section names; a value the core stops offering is refused under the
 * schedule (`schedule_minutes_out_of_range`).
 */
const MINUTES = [5, 10, 15, 30];
const HOURS = Array.from({ length: 12 }, (_, index) => index + 1);
/** How long typing waits before the core reads the schedule, so a cron line is read once it is written, not per key. */
const PREVIEW_WAIT_MS = 200;

type Refusal = { field: ReturnType<typeof refusalField>; code: string };
type Preview = { state: "reading" } | { state: "read"; value: SchedulePreview } | { state: "refused"; code: string };

/** This machine's projects a run may work in: listed, not temporary (B5). */
function useFolders(): Workspace[] {
  const workspaces = useShellStore((s) => s.rest?.navigator?.workspaces);
  const local = useShellStore((s) => localDeviceId(s.rest));
  return useMemo(() => listedWorkspaces(workspaces ?? []).filter((workspace) => workspace.device_id === local && !workspace.temporary), [workspaces, local]);
}

export function AutomationForm({ section, automation, example = null, actions }: { section: AutomationsSection; automation: Automation | null; example?: AutomationExample | null; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const [draft, setDraft] = useState<FormDraft>(() => (example && !automation
    ? exampleDraft(example, section.last_choice, { name: t(EXAMPLE_WORDS[example].name), task: t(EXAMPLE_WORDS[example].task) })
    : draftOf(automation, section.last_choice)));
  const [refusal, setRefusal] = useState<Refusal | null>(null);
  const [sending, setSending] = useState(false);
  const [touched, setTouched] = useState({ name: false, task: false });
  // An answer that lands after the form closed (cancel, back, another screen) moves nothing (B8).
  const open = useRef(true);
  useEffect(() => {
    open.current = true;
    return () => {
      open.current = false;
    };
  }, []);
  const { preview, ownFolder } = usePreview(draft.schedule, draft.name);
  const folders = useFolders();
  const chosen = draft.folder === null ? null : folders.find((workspace) => workspace.path === draft.folder) ?? null;
  const git = chosen?.is_git === true;
  const empty = emptyField(draft);
  const blocked = empty !== null || preview.state === "refused" || sending;
  const change = (patch: Partial<FormDraft>) => {
    setDraft((current) => ({ ...current, ...patch }));
    setRefusal(null);
  };
  const changeSchedule = (patch: Partial<ScheduleDraft>) => change({ schedule: { ...draft.schedule, ...patch } });
  const close = () => useUiStore.getState().setAutomations(automation ? { form: null } : { selected: null, run: null, form: null });

  const submit = async (runNow: boolean) => {
    if (blocked) return;
    setSending(true);
    if (automation) {
      const call = editCall(automation, draft, git);
      const answer = call ? await callAutomations(call) : null;
      if (!open.current) return;
      setSending(false);
      if (answer && !answer.ok) return setRefusal({ field: refusalField(answer.code), code: answer.code });
      useUiStore.getState().setAutomations({ form: null });
      return;
    }
    const answer = await callAutomations(addCall(draft, git, runNow));
    if (!open.current) return;
    setSending(false);
    if (!answer.ok) return setRefusal({ field: refusalField(answer.code), code: answer.code });
    // The new automation's detail, with the terminal of the run it started, held until the section has them (B8).
    useUiStore.getState().setAutomations({ selected: answer.value.id, run: answer.value.run ?? null, form: null, arriving: true });
  };

  // An empty name or task is one red line under its field once the field was left (B7); [만들기] already waits.
  const missing = (name: "name" | "task") => (touched[name] && !draft[name].trim() ? (name === "name" ? "automation_name_missing" : "automation_task_missing") : null);
  const field = (name: Refusal["field"]) => {
    if (refusal?.field === name) return <RefusalLine code={refusal.code} field={name} />;
    const empty = name === "name" || name === "task" ? missing(name) : null;
    return empty ? <RefusalLine code={empty} field={name} /> : null;
  };
  const scheduleRefusal = refusal?.field === "schedule" ? refusal.code : preview.state === "refused" ? preview.code : null;
  return (
    <>
      <div className="flex shrink-0 items-center gap-sm border-b border-border px-lg py-sm" data-automation-form-head="true">
        <Button variant="text" size="sm" className="-ml-xs gap-xxs px-xs" onClick={close} data-automation-back="true">
          <ChevronLeftIcon aria-hidden="true" />
          <span className="max-w-(--size-mission-table-project) truncate">{automation ? automation.name : t("automationScreen.back")}</span>
        </Button>
        <h1 className="min-w-0 flex-1 truncate text-subhead font-semibold">{automation ? t("automationScreen.form.editTitle", { name: automation.name }) : t("automationScreen.form.newTitle")}</h1>
      </div>
      <form
        className="min-h-0 flex-1 overflow-y-auto px-lg py-md"
        onSubmit={(event) => {
          event.preventDefault();
          void submit(false);
        }}
        data-automation-form={automation ? "edit" : "new"}
      >
        <div className="grid max-w-(--size-mission-panel-words) grid-cols-[max-content_minmax(0,1fr)] items-start gap-x-lg gap-y-md">
          <Field label={t("automationScreen.form.name")} htmlFor="automation-name">
            <Input id="automation-name" autoFocus value={draft.name} maxLength={80} placeholder={t("automationScreen.form.namePlaceholder")} onChange={(event) => change({ name: event.target.value })} onBlur={() => setTouched((now) => ({ ...now, name: true }))} aria-invalid={refusal?.field === "name" || missing("name") !== null || undefined} data-automation-field="name" />
            {field("name")}
          </Field>
          <Field label={t("automationScreen.form.task")} htmlFor="automation-task">
            <Textarea id="automation-task" rows={4} value={draft.task} placeholder={t("automationScreen.form.taskPlaceholder")} onChange={(event) => change({ task: event.target.value })} onBlur={() => setTouched((now) => ({ ...now, task: true }))} aria-invalid={refusal?.field === "task" || missing("task") !== null || undefined} data-automation-field="task" />
            {field("task")}
          </Field>
          <Field label={t("automationScreen.form.agent")}>
            <AgentFields draft={draft} change={change} actions={actions} />
            <p className="text-caption text-muted-foreground">{t("automationScreen.form.agentHint")}</p>
            {field("agent")}
          </Field>
          <Field label={t("automationScreen.form.folder")}>
            <Select value={draft.folder ?? OWN_FOLDER} onValueChange={(value) => change({ folder: value === OWN_FOLDER ? null : value, worktree: false })}>
              <SelectTrigger aria-label={t("automationScreen.form.folder")} className="w-full min-w-0" data-automation-field="folder"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value={OWN_FOLDER}>{t("automationScreen.form.folderNone")}</SelectItem>
                {draft.folder !== null && !chosen ? <SelectItem value={draft.folder}>{draft.folder}</SelectItem> : null}
                {folders.map((workspace) => <SelectItem key={workspace.id} value={workspace.path} description={workspace.path}>{workspace.label}</SelectItem>)}
              </SelectContent>
            </Select>
            {draft.folder === null && ownFolder ? (
              <p className="truncate font-mono text-caption text-muted-foreground" data-automation-own-folder="true">{shownOwnFolder(section.own_folder_root, ownFolder)}</p>
            ) : null}
            {git ? (
              <label className="flex items-center gap-xs text-body text-subtle-foreground">
                <Checkbox checked={draft.worktree} onCheckedChange={(checked) => change({ worktree: checked === true })} data-automation-field="worktree" />
                {t("automationScreen.form.worktree")}
              </label>
            ) : null}
            {field("folder")}
          </Field>
          <Field label={t("automationScreen.form.schedule")}>
            <ScheduleFields schedule={draft.schedule} change={changeSchedule} language={language} />
            {scheduleRefusal ? <RefusalLine code={scheduleRefusal} field="schedule" /> : <PreviewLine preview={preview} />}
          </Field>
        </div>
        {refusal?.field === "form" ? <div className="pt-md"><RefusalLine code={refusal.code} field="form" /></div> : null}
        <div className="flex max-w-(--size-mission-panel-words) flex-wrap justify-end gap-sm pt-lg">
          <Button type="button" variant="text" size="sm" onClick={close} data-automation-cancel="true">{t("automationScreen.form.cancel")}</Button>
          {automation ? null : (
            <Button type="button" variant="secondary" size="sm" disabled={blocked} onClick={() => void submit(true)} data-automation-create-run="true">{t("automationScreen.form.createRun")}</Button>
          )}
          <Button type="submit" size="sm" disabled={blocked} data-automation-create={automation ? undefined : "true"} data-automation-save={automation ? "true" : undefined}>
            {automation ? t("automationScreen.form.save") : t("automationScreen.form.create")}
          </Button>
        </div>
      </form>
    </>
  );
}

function Field({ label, htmlFor, children }: { label: string; htmlFor?: string; children: ReactNode }) {
  return (
    <>
      <label htmlFor={htmlFor} className="pt-xs text-caption text-muted-foreground">{label}</label>
      <div className="flex min-w-0 flex-col gap-xs">{children}</div>
    </>
  );
}

function RefusalLine({ code, field }: { code: string; field: string }) {
  const { t } = useInterfaceTranslation();
  return <p role="alert" className="text-caption text-destructive" data-automation-refusal={code} data-automation-refusal-field={field}>{t(refusalKey(code))}</p>;
}

/** `평일 09:00 · 다음: 목 11-20 09:00 · 이 기계 시간대` (B6). */
function PreviewLine({ preview }: { preview: Preview }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const now = useMinuteClock();
  if (preview.state !== "read") return <p className="text-caption text-muted-foreground" aria-hidden="true">&nbsp;</p>;
  const { value } = preview;
  const reads = scheduleText(value.reads_as, language, t);
  const text = value.next_run_at_unix_ms === null
    ? t("automationScreen.form.previewNever", { reads })
    : t("automationScreen.form.preview", { reads, next: whenText(value.next_run_at_unix_ms, now, value.zone ?? undefined, language, t) });
  return <p className="text-caption text-subtle-foreground" data-automation-preview="true">{text}</p>;
}

/**
 * The core's reading of the schedule and the name as they are typed (B5-B7);
 * a newer draft's answer replaces an older one. The schedule's reading is
 * kept while only the name moves, so its line, a refusal too, stays as a name
 * is typed; the own folder is the last one the core named, kept while the next
 * is read or the schedule is refused, so the folder line does not come and go
 * with each key.
 */
function usePreview(schedule: ScheduleDraft, name: string): { preview: Preview; ownFolder: string | null } {
  const live = useShellStore((s) => s.connection === "live");
  const scheduleKey = JSON.stringify(draftSchedule(schedule));
  const key = JSON.stringify({ schedule: draftSchedule(schedule), name: name.trim() });
  const [preview, setPreview] = useState<{ scheduleKey: string; value: Preview }>({ scheduleKey: "", value: { state: "reading" } });
  const [ownFolder, setOwnFolder] = useState<string | null>(null);
  const latest = useRef(key);
  latest.current = key;
  useEffect(() => {
    if (!live) return undefined;
    const timer = setTimeout(() => {
      const draft = JSON.parse(key) as { schedule: AutomationSchedule; name: string };
      void callAutomations({ call: "preview", schedule: draft.schedule, name: draft.name }).then((answer) => {
        if (latest.current !== key) return;
        // A call the socket dropped says nothing about the schedule; the next connection reads it again.
        if (!answer.ok && answer.code === "socket_closed") return;
        setPreview({ scheduleKey: JSON.stringify(draft.schedule), value: answer.ok ? { state: "read", value: answer.value } : { state: "refused", code: answer.code } });
        if (answer.ok) setOwnFolder(answer.value.own_folder);
      });
    }, PREVIEW_WAIT_MS);
    return () => clearTimeout(timer);
  }, [key, live]);
  return { preview: preview.scheduleKey === scheduleKey ? preview.value : { state: "reading" }, ownFolder };
}

/** The agent with its profile and model as an agent start picks them, and its effort (B4, D-07). */
function AgentFields({ draft, change, actions }: { draft: FormDraft; change: (patch: Partial<FormDraft>) => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((s) => s.rest);
  const usage = useShellStore((s) => s.usage);
  const menu = useMemo(() => startItems(rest, localDeviceId(rest), usage), [rest, usage]);
  const efforts = effortsOf(draft.agent.kind);
  const start = () => useShellStore.getState().rest?.ui_state?.agent_start;
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-xs" data-automation-field="agent" data-automation-agent={draft.agent.kind}>
      <AgentPicker
        actions={actions}
        value={{ kind: draft.agent.kind, model: draft.agent.model }}
        onChange={(next) => {
          if (next.kind === "terminal") return;
          const same = next.kind === draft.agent.kind;
          change({ agent: { kind: next.kind, profile: same ? draft.agent.profile : "default", model: next.model, effort: same ? draft.agent.effort : null } });
        }}
        items={{
          list: menu.items,
          profile: draft.agent.profile,
          showLeft: menu.showLeft,
          onItem: (item) => change({ agent: { kind: item.kind, profile: item.profile, model: item.kind === draft.agent.kind ? draft.agent.model : selectKind(item.kind, start()).model, effort: item.kind === draft.agent.kind ? draft.agent.effort : null } }),
        }}
      />
      {efforts.length > 0 ? (
        <Select value={draft.agent.effort ?? DEFAULT_EFFORT} onValueChange={(value) => change({ agent: { ...draft.agent, effort: value === DEFAULT_EFFORT ? null : value } })}>
          <SelectTrigger aria-label={t("automationScreen.form.effort")} className="w-auto shrink-0" data-automation-field="effort"><SelectValue /></SelectTrigger>
          <SelectContent>
            <SelectItem value={DEFAULT_EFFORT}>{`${t("automationScreen.form.effort")} · ${t("automationScreen.form.effortDefault")}`}</SelectItem>
            {efforts.map((effort) => <SelectItem key={effort} value={effort}>{effort}</SelectItem>)}
          </SelectContent>
        </Select>
      ) : null}
    </div>
  );
}

/** 매일 / 평일 / 매주 with its days / N시간마다 with its interval, each with its time; N분마다 from each hour's start; or a cron line (B6). */
function ScheduleFields({ schedule, change, language }: { schedule: ScheduleDraft; change: (patch: Partial<ScheduleDraft>) => void; language: ReturnType<typeof requireInterfaceLanguage> }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex min-w-0 flex-col gap-xs">
      <div className="flex min-w-0 flex-wrap items-center gap-xs">
        <Select value={schedule.kind} onValueChange={(value) => change({ kind: value as ScheduleKind })}>
          <SelectTrigger aria-label={t("automationScreen.form.schedule")} className="w-auto shrink-0" data-automation-field="schedule-kind"><SelectValue /></SelectTrigger>
          <SelectContent>
            {KINDS.map((kind) => <SelectItem key={kind} value={kind} data-automation-schedule-kind={kind}>{t(KIND_KEY[kind])}</SelectItem>)}
          </SelectContent>
        </Select>
        {schedule.kind === "every_minutes" ? (
          <Select value={String(schedule.minutes)} onValueChange={(value) => change({ minutes: Number(value) })}>
            <SelectTrigger aria-label={t("automationScreen.form.interval")} className="w-auto shrink-0" data-automation-field="minutes"><SelectValue /></SelectTrigger>
            <SelectContent>
              {MINUTES.map((minutes) => <SelectItem key={minutes} value={String(minutes)} data-automation-minutes={minutes}>{t("automationScreen.form.minutesValue", { minutes })}</SelectItem>)}
            </SelectContent>
          </Select>
        ) : null}
        {schedule.kind === "every_hours" ? (
          <Select value={String(schedule.hours)} onValueChange={(value) => change({ hours: Number(value) })}>
            <SelectTrigger aria-label={t("automationScreen.form.interval")} className="w-auto shrink-0" data-automation-field="hours"><SelectValue /></SelectTrigger>
            <SelectContent>
              {HOURS.map((hours) => <SelectItem key={hours} value={String(hours)}>{t("automationScreen.form.hoursValue", { count: hours })}</SelectItem>)}
            </SelectContent>
          </Select>
        ) : null}
        {schedule.kind === "every_minutes" ? null : schedule.kind === "cron" ? (
          <Input mono className="min-w-0 flex-1" value={schedule.expr} placeholder={t("automationScreen.form.cronPlaceholder")} aria-label={t("automationScreen.form.kind.cron")} onChange={(event) => change({ expr: event.target.value })} data-automation-field="cron" />
        ) : (
          <Input type="time" step={60} className="w-auto shrink-0" value={schedule.time} aria-label={t("automationScreen.form.time")} onChange={(event) => change({ time: event.target.value })} data-automation-field="time" />
        )}
      </div>
      {schedule.kind === "weekly" ? (
        <ToggleGroup type="multiple" className={cn("flex-wrap self-start")} value={schedule.days} onValueChange={(days) => change({ days: days as AutomationWeekday[] })} aria-label={t("automationScreen.form.days")} data-automation-field="days">
          {WEEKDAYS.map((day) => <ToggleGroupItem key={day} size="sm" value={day} data-automation-day={day}>{weekdayText(language, day)}</ToggleGroupItem>)}
        </ToggleGroup>
      ) : null}
    </div>
  );
}
