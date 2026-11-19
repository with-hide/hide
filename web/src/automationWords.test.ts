import { describe, expect, it } from "vitest";
import { createInterfaceI18n } from "./i18n/instance";
import { lastResult, nextRunText, recentSpendText, recordText, scheduleText, spendText, whenText } from "./automationWords";
import type { Automation, AutomationRecord } from "./snapshot";

// PRD automations B3, B6, B20, B22: what the table, the detail and the phone
// say for a schedule, a next time and a record, read in the zone the core
// reads its schedules in, not the page's.

const ko = (await createInterfaceI18n("ko")).t;
const en = (await createInterfaceI18n("en")).t;
const SEOUL = "Asia/Seoul";
/** Thursday 2026-11-19 23:30 in Seoul, 14:30 UTC. */
const NOW = Date.UTC(2026, 10, 19, 14, 30);

const run = (fields: Partial<Extract<AutomationRecord, { kind: "run" }>>): AutomationRecord => ({
  kind: "run", number: 1, trigger: "scheduled", started_at_unix_ms: NOW - 12 * 60_000, state: "succeeded", attempt: 1, opened: false, resumable: true, conversation_missing: false, seen: false, ...fields,
});

function automation(fields: Partial<Automation>): Automation {
  return {
    id: "a1", name: "아침 이슈 정리", task: "새 이슈 정리", schedule: { kind: "daily", time: "09:00" }, reads_as: { kind: "daily", time: "09:00" },
    next_run_at_unix_ms: null, agent: { kind: "claude" }, folder: { path: "/home/u/hide/automations/a", default: true, worktree: false }, running: false, unseen: 0, records: [], ...fields,
  };
}

describe("an automation's words", () => {
  it("reads each schedule choice, the days in the week's order and the minutes and hours by count", () => {
    expect(scheduleText({ kind: "weekdays", time: "09:00" }, "ko", ko)).toBe("평일 09:00");
    expect(scheduleText({ kind: "weekly", days: ["wed", "mon"], time: "10:00" }, "ko", ko)).toBe("매주 월·수 10:00");
    expect(scheduleText({ kind: "every_minutes", minutes: 15 }, "ko", ko)).toBe("15분마다");
    expect(scheduleText({ kind: "every_minutes", minutes: 5 }, "en", en)).toBe("Every 5 minutes");
    expect(scheduleText({ kind: "every_hours", hours: 1, time: "00:30" }, "en", en)).toBe("Every hour · from 00:30");
    expect(scheduleText({ kind: "every_hours", hours: 3, time: "09:00" }, "en", en)).toBe("Every 3 hours · from 09:00");
    expect(scheduleText({ kind: "cron", expr: "0 9 1 * *" }, "ko", ko)).toBe("cron 0 9 1 * *");
  });

  it("names the day in the schedules' zone, where 00:30 the next morning is already tomorrow", () => {
    // 15:30 UTC is 00:30 on Friday in Seoul, the same UTC day as now.
    expect(whenText(Date.UTC(2026, 10, 19, 15, 30), NOW, SEOUL, "ko", ko)).toBe("내일 00:30");
    expect(whenText(Date.UTC(2026, 10, 19, 14, 0), NOW, SEOUL, "ko", ko)).toBe("오늘 23:00");
    expect(whenText(Date.UTC(2026, 10, 21, 0, 0), NOW, SEOUL, "ko", ko)).toBe("토 11-21 09:00");
  });

  it("says a run going and a pause before any next time", () => {
    const next = Date.UTC(2026, 10, 20, 0, 0);
    expect(nextRunText(automation({ next_run_at_unix_ms: next }), NOW, SEOUL, "ko", ko)).toBe("내일 09:00");
    expect(nextRunText(automation({ next_run_at_unix_ms: next, running: true }), NOW, SEOUL, "ko", ko)).toBe("도는 중");
    expect(nextRunText(automation({ paused: { reason: "operator", at_unix_ms: NOW } }), NOW, SEOUL, "ko", ko)).toBe("멈춤");
  });

  it("words a run by how it ended: its length, its tries, or the reason a person fixes", () => {
    expect(recordText(run({ ended_at_unix_ms: NOW }), NOW, SEOUL, "ko", ko)).toEqual({ tone: "ok", text: "12분" });
    expect(recordText(run({ state: "failed", attempt: 3, reason: "agent_exited", ended_at_unix_ms: NOW }), NOW, SEOUL, "ko", ko)).toEqual({ tone: "failed", text: "실패 · 3번 시도" });
    expect(recordText(run({ state: "failed", reason: "automation_login_expired", ended_at_unix_ms: NOW }), NOW, SEOUL, "ko", ko)).toEqual({ tone: "failed", text: "로그인 만료" });
    expect(recordText(run({ state: "running", attempt: 2 }), NOW, SEOUL, "ko", ko)).toEqual({ tone: "running", text: "2번째 시도" });
    expect(recordText(run({ state: "stopped", reason: "stopped_by_person", ended_at_unix_ms: NOW }), NOW, SEOUL, "ko", ko)).toEqual({ tone: "stopped", text: "멈춤 · 사람이 끝냄" });
    expect(recordText(run({ state: "stopped", reason: "automation_machine_changed", ended_at_unix_ms: NOW }), NOW, SEOUL, "ko", ko)).toEqual({ tone: "stopped", text: "멈춤 · 기계 바뀜" });
    expect(recordText({ kind: "skipped", at_unix_ms: NOW, last_unix_ms: NOW, count: 1, reason: { kind: "previous_running" } }, NOW, SEOUL, "ko", ko)).toEqual({ tone: "skipped", text: "건너뜀 · 이전 회차 진행 중" });
  });

  it("words what a run spent in the usage screen's tokens, and its price as a lower bound when some calls had none", () => {
    expect(spendText({ tokens: 12_340, micro_usd: 420_000, unpriced: false }, "ko", ko)).toBe("토큰 12.3K · $0.42");
    expect(spendText({ tokens: 900, micro_usd: 90_000, unpriced: true }, "en", en)).toBe("900 tokens · ≥ $0.09");
    // A run of models with no known price says its tokens alone, never `≥ $0.00`.
    expect(spendText({ tokens: 12_340, micro_usd: 0, unpriced: true }, "ko", ko)).toBe("토큰 12.3K");
    expect(recentSpendText({ tokens: 1_500_000, micro_usd: 2_100_000, unpriced: false, runs: 5 }, "ko", ko)).toBe("최근 5회 · 토큰 1.5M · $2.10");
  });

  it("folds the times skipped one after another into one line that says how many and when", () => {
    // 05:00-09:55 UTC is 14:00-18:55 on the same day in Seoul; 14:30 UTC is 23:30 the day before 01:10.
    const today: AutomationRecord = { kind: "skipped", at_unix_ms: Date.UTC(2026, 10, 19, 5, 0), last_unix_ms: Date.UTC(2026, 10, 19, 9, 55), count: 12, reason: { kind: "previous_running" } };
    expect(recordText(today, NOW, SEOUL, "ko", ko)).toEqual({ tone: "skipped", text: "건너뜀 · 이전 회차 진행 중 · 12번 · 오늘 14:00–18:55" });
    const overnight: AutomationRecord = { ...today, at_unix_ms: Date.UTC(2026, 10, 18, 14, 30), last_unix_ms: Date.UTC(2026, 10, 18, 16, 10), count: 2, reason: { kind: "usage_limit", agent: "claude" } };
    expect(recordText(overnight, NOW, SEOUL, "en", en).text).toBe("Skipped · Claude Code usage limit · 2 times · Yesterday 23:30 ~ Today 01:10");
  });

  it("folds the times missed while Hide was off into one line", () => {
    const missed: AutomationRecord = { kind: "missed", first_unix_ms: Date.UTC(2026, 10, 18, 1, 0), last_unix_ms: Date.UTC(2026, 10, 18, 23, 0), count: 24, reason: "core_off" };
    expect(recordText(missed, NOW, SEOUL, "ko", ko).text).toBe("어제 10:00 ~ 오늘 08:00 · 놓침 24회 · Hide가 꺼져 있었음");
  });

  it("takes the newest finished run or skipped time as the last result, never the run going", () => {
    const going = run({ number: 3, state: "running" });
    const done = run({ number: 2, ended_at_unix_ms: NOW });
    expect(lastResult(automation({ records: [going, done] }))).toBe(done);
    expect(lastResult(automation({ records: [going] }))).toBeNull();
  });
});
