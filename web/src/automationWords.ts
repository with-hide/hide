import type { TFunction } from "i18next";
import { agentProductName } from "./agentAdapters";
import { formatElapsed } from "./components/elapsed";
import type { InterfaceLanguage } from "./i18n/locale";
import type { Automation, AutomationPauseReason, AutomationRecord, AutomationRun, AutomationSchedule, AutomationSpend, AutomationWeekday } from "./snapshot";
import { compactTokens } from "./usageHistory";

// The words of an automation (PRD automations B3, B6, B11, B16-B23, B34),
// one set for the desktop screen and the phone's tab: what a schedule reads
// as, when it runs next, each record's line and why it stopped. Every value
// is the core's; this module only words it, in the zone the core reads its
// schedules in (the machine that runs them, D-06).

type Translate = TFunction<"translation">;

/** The days in the order the form and a schedule's words list them. */
export const WEEKDAYS: readonly AutomationWeekday[] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/** A Sunday, so a weekday's short name comes from the language rather than a list kept per language. */
const A_SUNDAY = Date.UTC(2024, 0, 7, 12);
const DAY_INDEX: Record<AutomationWeekday, number> = { sun: 0, mon: 1, tue: 2, wed: 3, thu: 4, fri: 5, sat: 6 };

/** `월`, `Mon`: the day's short name in `language`. */
export function weekdayText(language: InterfaceLanguage, day: AutomationWeekday): string {
  return new Intl.DateTimeFormat(language, { weekday: "short", timeZone: "UTC" }).format(A_SUNDAY + DAY_INDEX[day] * 86_400_000);
}

/** `매일 09:00`, `평일 09:00`, `매주 월·수 09:00`, `15분마다`, `3시간마다 · 09:00 기준`, or the cron line itself when it equals no choice (B3, B6). */
export function scheduleText(schedule: AutomationSchedule, language: InterfaceLanguage, t: Translate): string {
  switch (schedule.kind) {
    case "daily":
      return t("automation.schedule.daily", { time: schedule.time });
    case "weekdays":
      return t("automation.schedule.weekdays", { time: schedule.time });
    case "weekly": {
      const days = WEEKDAYS.filter((day) => schedule.days.includes(day)).map((day) => weekdayText(language, day));
      return t("automation.schedule.weekly", { days: days.join("·"), time: schedule.time });
    }
    case "every_minutes":
      return t("automation.schedule.everyMinutes", { minutes: schedule.minutes });
    case "every_hours":
      return t("automation.schedule.everyHours", { count: schedule.hours, time: schedule.time });
    case "cron":
      return t("automation.schedule.cron", { expr: schedule.expr });
    case "once":
      return t("automation.schedule.once");
  }
}

/** A once schedule's own time, `12-02(수) 09:00`: the wall date and time it was set for, which the machine reads in its own zone. */
export function onceText(schedule: Extract<AutomationSchedule, { kind: "once" }>, language: InterfaceLanguage, t: Translate): string {
  const [year, month, day] = schedule.date.split("-").map(Number);
  const weekday = new Intl.DateTimeFormat(language, { weekday: "short", timeZone: "UTC" }).format(Date.UTC(year ?? 0, (month ?? 1) - 1, day ?? 1, 12));
  return t("automation.when.once", { date: schedule.date.slice(5), weekday, time: schedule.time });
}

/**
 * `7일 뒤`: how long until a once schedule's time, by the calendar when it is
 * another day (so 1주일 뒤 reads as 7일 뒤 at any hour), else `3시간 뒤`, or
 * `곧` in its last minute; `at` is the core's time for it.
 */
export function onceUntilText(schedule: Extract<AutomationSchedule, { kind: "once" }>, at: number, now: number, zone: string | undefined, language: InterfaceLanguage, t: Translate): string {
  const [year, month, day] = schedule.date.split("-").map(Number);
  const [nowYear, nowMonth, nowDay] = wallDay(now, zone).split("-").map(Number);
  const days = Math.round((Date.UTC(year ?? 0, (month ?? 1) - 1, day ?? 1) - Date.UTC(nowYear ?? 0, (nowMonth ?? 1) - 1, nowDay ?? 1)) / 86_400_000);
  if (days >= 1) return t("automationScreen.inTime", { time: formatElapsed(language, days * 86_400_000) });
  return at - now < 60_000 ? t("automationScreen.soon") : t("automationScreen.inTime", { time: formatElapsed(language, at - now) });
}

/** `12-02(수) 09:00 · 한 번` for a once schedule, which says its date where a repeating one says how often; any other reads as `scheduleText`. */
export function scheduleWithDate(schedule: AutomationSchedule, language: InterfaceLanguage, t: Translate): string {
  if (schedule.kind !== "once") return scheduleText(schedule, language, t);
  return t("automation.schedule.onceAt", { when: onceText(schedule, language, t) });
}

/** Whether it runs one time, at a date, rather than again and again (mission-104). */
export function isOnce(automation: { schedule: AutomationSchedule }): boolean {
  return automation.schedule.kind === "once";
}

/** `2026-11-25`: the day `unixMs` falls on in `zone`, the machine's own today when it is now. */
export function wallDay(unixMs: number, zone: string | undefined): string {
  return wall(unixMs, zone).day;
}

/** The wall date and time of `unixMs` in `zone`. */
function wall(unixMs: number, zone: string | undefined): { day: string; time: string } {
  const parts = new Intl.DateTimeFormat("en-CA", { timeZone: zone, year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).formatToParts(unixMs);
  const part = (type: Intl.DateTimeFormatPartTypes) => parts.find((one) => one.type === type)?.value ?? "";
  return { day: `${part("year")}-${part("month")}-${part("day")}`, time: `${part("hour")}:${part("minute")}` };
}

/** A time in the schedules' zone: `오늘 09:00`, `내일 09:00`, `어제 09:00`, else `목 11-20 09:00` (B3, B6, B20). */
export function whenText(unixMs: number, now: number, zone: string | undefined, language: InterfaceLanguage, t: Translate): string {
  const at = wall(unixMs, zone);
  if (at.day === wall(now, zone).day) return t("automation.when.today", { time: at.time });
  if (at.day === wall(now + 86_400_000, zone).day) return t("automation.when.tomorrow", { time: at.time });
  if (at.day === wall(now - 86_400_000, zone).day) return t("automation.when.yesterday", { time: at.time });
  const weekday = new Intl.DateTimeFormat(language, { timeZone: zone, weekday: "short" }).format(unixMs);
  return t("automation.when.date", { weekday, date: at.day.slice(5), time: at.time });
}

/** What 다음 실행 says (B3, B25): a run going, the automation paused, its next time, a once automation done (mission-104), or none (a schedule that never runs again). */
export type NextRun = { kind: "running" } | { kind: "paused" } | { kind: "at"; unixMs: number } | { kind: "done" } | { kind: "none" };

/** The fields of an automation its next run is read from; the phone's frame carries only these, `done` absent from a Mac older than once automations. */
export type NextRunFacts = Pick<Automation, "running" | "paused" | "next_run_at_unix_ms"> & { done?: boolean };

export function nextRun(automation: NextRunFacts): NextRun {
  if (automation.running) return { kind: "running" };
  if (automation.paused) return { kind: "paused" };
  if (automation.done) return { kind: "done" };
  return automation.next_run_at_unix_ms == null ? { kind: "none" } : { kind: "at", unixMs: automation.next_run_at_unix_ms };
}

/** `도는 중`, `멈춤`, `내일 09:00`, `끝남`, `—`: the words after the mark the screen draws (●, ⏸). */
export function nextRunText(automation: NextRunFacts, now: number, zone: string | undefined, language: InterfaceLanguage, t: Translate): string {
  const next = nextRun(automation);
  switch (next.kind) {
    case "running":
      return t("automation.next.running");
    case "paused":
      return t("automation.next.paused");
    case "at":
      return whenText(next.unixMs, now, zone, language, t);
    case "done":
      return t("automation.next.done");
    case "none":
      return t("automation.next.none");
  }
}

/** Why an automation stopped (B23, B25): the operator's own pause, or what a person fixes before [다시 켜기]. */
export function pauseReasonText(reason: AutomationPauseReason, t: Translate): string {
  switch (reason) {
    case "operator":
      return t("automation.pause.operator");
    case "login_expired":
      return t("automation.pause.loginExpired");
    case "folder_missing":
      return t("automation.pause.folderMissing");
    case "agent_missing":
      return t("automation.pause.agentMissing");
    case "folder_detached":
      return t("automation.pause.folderDetached");
    case "machine_changed":
      return t("automation.pause.machineChanged");
    case "folder_in_repository":
      return t("automation.pause.folderInRepository");
  }
}

const PAUSE_CODES: Record<string, AutomationPauseReason> = {
  automation_paused: "operator",
  automation_login_expired: "login_expired",
  automation_folder_missing: "folder_missing",
  automation_agent_missing: "agent_missing",
  automation_folder_detached: "folder_detached",
  automation_machine_changed: "machine_changed",
  automation_folder_in_repository: "folder_in_repository",
};

/** How a record's mark reads (B3, B16, B20, B22): ✓, ✗, –, ● or ■. */
export type RecordTone = "ok" | "failed" | "skipped" | "running" | "stopped";

/** A record's mark and its words after the time: `✓ 12분`, `✗ 실패 · 3번 시도`, `✗ 로그인 만료`, `■ 멈춤 · 사람이 끝냄`, `– 건너뜀 · Claude 사용 한도`, `– 건너뜀 · 이전 회차 진행 중 · 12번 · 오늘 14:00–18:55`, `– 어제 10:00 ~ 오늘 08:00 · 놓침 24회 · Hide가 꺼져 있었음`. */
export type RecordText = { tone: RecordTone; text: string };

/** The fields of a run its words are read from; the phone's frame carries only these, never a path, a branch, a pane or a conversation. */
export type RunFacts = Pick<AutomationRun, "state" | "attempt" | "reason" | "started_at_unix_ms" | "ended_at_unix_ms">;
/** A record as its words read it: a run's facts, a skipped time or the times missed. */
export type AutomationRecordLike = ({ kind: "run" } & RunFacts) | Exclude<AutomationRecord, { kind: "run" }>;

export function recordText(record: AutomationRecordLike, now: number, zone: string | undefined, language: InterfaceLanguage, t: Translate): RecordText {
  switch (record.kind) {
    case "run":
      return runText(record, language, t);
    case "skipped": {
      const what = record.reason.kind === "previous_running"
        ? t("automation.record.skippedPrevious")
        : t("automation.record.skippedLimit", { agent: agentProductName(record.reason.agent) });
      if (!skippedFolded(record)) return { tone: "skipped", text: what };
      const span = spanText(record.at_unix_ms, record.last_unix_ms, now, zone, language, t);
      return { tone: "skipped", text: t("automation.record.skippedFolded", { what, times: record.count, span }) };
    }
    case "missed": {
      const why = record.reason === "core_off" ? t("automation.record.missedCoreOff") : t("automation.record.missedAsleep");
      // One time missed is that one time, said in the time's own place (`recordTime`), never `A ~ A`.
      if (!missedFolded(record)) return { tone: "skipped", text: t("automation.record.missedOnce", { why }) };
      return {
        tone: "skipped",
        text: t("automation.record.missed", {
          first: whenText(record.first_unix_ms, now, zone, language, t),
          last: whenText(record.last_unix_ms, now, zone, language, t),
          count: record.count,
          why,
        }),
      };
    }
  }
}

/**
 * What a run spent, `토큰 12.3K` (D-26, B39), in the usage screen's token
 * form; the price stays off the screen (operator decision 2026-11-21).
 */
export function spendText(spend: AutomationSpend, language: InterfaceLanguage, t: Translate): string {
  return t("automation.spendTokens", { tokens: compactTokens(spend.tokens, language) });
}

/** `최근 5회 · 토큰 61.5K`: what the runs the ledger read among those kept spent together (B39). */
export function recentSpendText(recent: AutomationSpend & { runs: number }, language: InterfaceLanguage, t: Translate): string {
  return t("automation.recentSpend", { count: recent.runs, spend: spendText(recent, language, t) });
}

/** Whether a skipped line holds several times, which then say their own span as a missed line does, with no time of their own. */
export function skippedFolded(record: Extract<AutomationRecord, { kind: "skipped" }>): boolean {
  return record.count > 1;
}

/** Whether a missed line holds several times, which then say their own span; one time missed has its time as a run does. */
export function missedFolded(record: Extract<AutomationRecord, { kind: "missed" }>): boolean {
  return record.count > 1;
}

/**
 * When a skipped or missed line happened, the time a list puts in front of its words: its one time, or `null` for a line of
 * several times, whose words say their span (`recordText`). A run's time is the caller's, which reads a due or a start.
 */
export function lineTime(record: Exclude<AutomationRecord, { kind: "run" }>): number | null {
  if (record.kind === "skipped") return skippedFolded(record) ? null : record.at_unix_ms;
  return missedFolded(record) ? null : record.first_unix_ms;
}

/** `오늘 14:00–18:55` within one day, else `어제 23:30 ~ 오늘 01:10`. */
function spanText(first: number, last: number, now: number, zone: string | undefined, language: InterfaceLanguage, t: Translate): string {
  const from = whenText(first, now, zone, language, t);
  const end = wall(last, zone);
  return wall(first, zone).day === end.day ? `${from}–${end.time}` : `${from} ~ ${whenText(last, now, zone, language, t)}`;
}

/** A run's mark and words (B16, B22, B24): its length when it finished the task, else why it did not. */
export function runText(run: RunFacts, language: InterfaceLanguage, t: Translate): RecordText {
  switch (run.state) {
    case "starting":
    case "running":
      return { tone: "running", text: run.attempt > 1 ? t("automation.record.attempt", { count: run.attempt }) : t("automation.record.running") };
    case "succeeded":
      return { tone: "ok", text: formatElapsed(language, (run.ended_at_unix_ms ?? run.started_at_unix_ms) - run.started_at_unix_ms) };
    case "failed": {
      // A failure a person fixes stopped the automation and names why; the rest ran out of tries (D-17).
      const pause = run.reason ? PAUSE_CODES[run.reason] : undefined;
      return { tone: "failed", text: pause ? pauseReasonText(pause, t) : t("automation.record.failed", { count: run.attempt }) };
    }
    case "stopped":
      // A person ended it, or the core of another machine read it and stopped it.
      return { tone: "stopped", text: run.reason === "automation_machine_changed" ? t("automation.record.stoppedMachine") : t("automation.record.stopped") };
  }
}

/** The table's 마지막 결과 (B3): the newest finished run or skipped time; a run going and a missed line are not a result. */
export function lastResult(automation: Automation): Extract<AutomationRecord, { kind: "run" } | { kind: "skipped" }> | null {
  for (const record of automation.records) {
    if (record.kind === "skipped") return record;
    if (record.kind === "run" && record.state !== "starting" && record.state !== "running") return record;
  }
  return null;
}

/**
 * How a once automation's run ended, as its Needs You line says it after `자동화 · 한 번 ·` (mission-104 S-4): `✓ 끝남 · 4분`,
 * `✗ 실패 · 로그인 만료`, `✗ 실패 · 3번 시도`, `■ 멈춤 · 사람이 끝냄`.
 */
export function lookText(run: RunFacts, language: InterfaceLanguage, t: Translate): RecordText {
  const shown = runText(run, language, t);
  if (run.state === "succeeded") return { tone: "ok", text: t("automationScreen.look.done", { time: shown.text }) };
  if (run.state === "failed" && run.reason && PAUSE_CODES[run.reason]) return { tone: "failed", text: t("automationScreen.look.failed", { reason: shown.text }) };
  return shown;
}
