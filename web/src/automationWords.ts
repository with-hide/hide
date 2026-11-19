import type { TFunction } from "i18next";
import { agentProductName } from "./agentAdapters";
import { formatElapsed } from "./components/elapsed";
import type { InterfaceLanguage } from "./i18n/locale";
import type { Automation, AutomationPauseReason, AutomationRecord, AutomationRun, AutomationSchedule, AutomationWeekday } from "./snapshot";

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

/** `매일 09:00`, `평일 09:00`, `매주 월·수 09:00`, `3시간마다 · 09:00 기준`, or the cron line itself when it equals no choice (B3, B6). */
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
    case "every_hours":
      return t("automation.schedule.everyHours", { count: schedule.hours, time: schedule.time });
    case "cron":
      return t("automation.schedule.cron", { expr: schedule.expr });
  }
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

/** What 다음 실행 says (B3, B25): a run going, the automation paused, its next time, or none (a schedule that never runs again). */
export type NextRun = { kind: "running" } | { kind: "paused" } | { kind: "at"; unixMs: number } | { kind: "none" };

export function nextRun(automation: Automation): NextRun {
  if (automation.running) return { kind: "running" };
  if (automation.paused) return { kind: "paused" };
  return automation.next_run_at_unix_ms == null ? { kind: "none" } : { kind: "at", unixMs: automation.next_run_at_unix_ms };
}

/** `도는 중`, `멈춤`, `내일 09:00`, `—`: the words after the mark the screen draws (●, ⏸). */
export function nextRunText(automation: Automation, now: number, zone: string | undefined, language: InterfaceLanguage, t: Translate): string {
  const next = nextRun(automation);
  switch (next.kind) {
    case "running":
      return t("automation.next.running");
    case "paused":
      return t("automation.next.paused");
    case "at":
      return whenText(next.unixMs, now, zone, language, t);
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

/** A record's mark and its words after the time: `✓ 12분`, `✗ 실패 · 3번 시도`, `✗ 로그인 만료`, `■ 멈춤 · 사람이 끝냄`, `– 건너뜀 · Claude 사용 한도`, `– 어제 10:00 ~ 오늘 08:00 · 놓침 24회 · Hide가 꺼져 있었음`. */
export type RecordText = { tone: RecordTone; text: string };

export function recordText(record: AutomationRecord, now: number, zone: string | undefined, language: InterfaceLanguage, t: Translate): RecordText {
  switch (record.kind) {
    case "run":
      return runText(record, language, t);
    case "skipped":
      return {
        tone: "skipped",
        text: record.reason.kind === "previous_running"
          ? t("automation.record.skippedPrevious")
          : t("automation.record.skippedLimit", { agent: agentProductName(record.reason.agent) }),
      };
    case "missed":
      return {
        tone: "skipped",
        text: t("automation.record.missed", {
          first: whenText(record.first_unix_ms, now, zone, language, t),
          last: whenText(record.last_unix_ms, now, zone, language, t),
          count: record.count,
          why: record.reason === "core_off" ? t("automation.record.missedCoreOff") : t("automation.record.missedAsleep"),
        }),
      };
  }
}

/** A run's mark and words (B16, B22, B24): its length when it finished the task, else why it did not. */
export function runText(run: AutomationRun, language: InterfaceLanguage, t: Translate): RecordText {
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
      return { tone: "stopped", text: t("automation.record.stopped") };
    case "skipped":
      return { tone: "skipped", text: t("automation.record.skippedRunLimit") };
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
