import { describe, expect, it } from "vitest";
import type { UsageAmount, UsageRange } from "./generated/hided-ws";
import { allTokens, hideAiShare, sumHideAiDays, sumUsageDays, usageDays, usagePeriodDays } from "./usageHistory";

const empty: UsageAmount = {
  calls: 0, new_tokens: 0,
  tokens: { input: 0, output: 0, cache_read: 0, cache_write: 0, cache_write_1h: 0, reasoning: null },
  cost: { micro_usd: 0, partial: false, waiting: false, stale: false, incomplete: false, unpriced_models: [], other_unpriced_models: 0 },
};
const range = (reading: boolean): UsageRange => ({
  calendar_start: "2026-03-08", calendar_end: "2026-03-10", days: [], amount: empty,
  since: null, active_days: 0, observed_days: 0, peak: null, current_streak: 0, longest_streak: 0,
  reading, limited: false, files_done: 0, files_total: 1, hide_ai_since: null,
});

describe("sparse usage calendar", () => {
  it("fills civil dates across a daylight-saving boundary without inventing records", () => {
    const days = usageDays(range(false));
    expect(days.map(({ date, weekday, week, today }) => ({ date, weekday, week, today }))).toEqual([
      { date: "2026-03-08", weekday: 0, week: 0, today: false },
      { date: "2026-03-09", weekday: 1, week: 0, today: false },
      { date: "2026-03-10", weekday: 2, week: 0, today: true },
    ]);
    expect(days.map((day) => [day.pending, day.amount.calls, day.level])).toEqual([[false, 0, 0], [false, 0, 0], [false, 0, 0]]);
  });

  it("keeps a zero-token recorded call distinct from pending empty cells", () => {
    const sparse = range(true);
    const recorded = {
      date: "2026-03-09", weekday: 1, week: 0, today: false, recent_week: true,
      level: 0, pending: false, agents: [], top_agents: [], other_agents: 0, hide_ai: null,
      amount: { ...empty, calls: 1, tokens: { ...empty.tokens, cache_read: 900 }, cost: { ...empty.cost, micro_usd: 125_000 } },
    };
    sparse.days = [recorded];
    const days = usageDays(sparse);
    expect(days.map((day) => day.pending)).toEqual([true, false, true]);
    expect(days[1]).toBe(recorded);
    expect(days[1]?.amount.cost.micro_usd).toBe(125_000);
    expect(sparse.days).toEqual([recorded]);
  });

  it("places the final empty week and recent-week outline using the core bounds", () => {
    const days = usageDays({ ...range(false), calendar_end: "2026-03-16" });
    expect(days).toHaveLength(9);
    expect(days[7]).toMatchObject({ date: "2026-03-15", week: 1, weekday: 0 });
    expect(days.filter((day) => day.recent_week).map((day) => day.date)).toEqual([
      "2026-03-10", "2026-03-11", "2026-03-12", "2026-03-13", "2026-03-14", "2026-03-15", "2026-03-16",
    ]);
  });
});

describe("usage display periods", () => {
  it("shows exactly thirty civil days and places a partial first week without shifting recorded cells", () => {
    const source = { ...range(false), calendar_start: "2026-02-01", calendar_end: "2026-03-10" };
    const record = { ...usageDays(source)[28]!, level: 4, week: 4, amount: { ...empty, calls: 1, new_tokens: 500 } };
    source.days = [record];
    const days = usagePeriodDays(source, 30);
    expect(days).toHaveLength(30);
    expect(days[0]).toMatchObject({ date: "2026-02-09", weekday: 1, week: 0 });
    expect(days.at(-1)).toMatchObject({ date: "2026-03-10", weekday: 2, week: 4, today: true });
    expect(days.find((day) => day.date === "2026-03-01")).toMatchObject({ weekday: 0, week: 3, level: 4, amount: { new_tokens: 500 } });
    expect(source.days[0]!.week).toBe(4);
  });

  it("sums the selected dates without counting cache reads twice or dropping a pricing limitation", () => {
    const days = usageDays(range(false));
    days[0]!.amount = { ...empty, calls: 1, new_tokens: 60, tokens: { ...empty.tokens, input: 10, output: 20, cache_write: 30, cache_write_1h: 5, cache_read: 900 }, cost: { ...empty.cost, micro_usd: 120_000 } };
    days[1]!.amount = { ...empty, calls: 1, new_tokens: 90, tokens: { ...empty.tokens, input: 20, output: 30, cache_write: 40, cache_read: 1800 }, cost: { ...empty.cost, partial: true, unpriced_models: ["unpriced-fixture"] } };
    const total = sumUsageDays(days);
    expect(total.new_tokens).toBe(150);
    expect(allTokens(total)).toBe(2850);
    expect(total.tokens).toMatchObject({ input: 30, output: 50, cache_write: 70, cache_write_1h: 5, cache_read: 2700 });
    expect(total.cost).toMatchObject({ micro_usd: 120_000, partial: true, unpriced_models: ["unpriced-fixture"] });
    expect(sumUsageDays(days.slice(1)).cost.micro_usd).toBe(0);
  });
  it("bounds cross-day unpriced names without inventing an additional-model count", () => {
    const days = usageDays(range(false));
    days[0]!.amount = { ...empty, calls: 1, cost: { ...empty.cost, partial: true, unpriced_models: ["a", "b", "c", "d", "e", "f", "g", "h"], other_unpriced_models: 2 } };
    days[1]!.amount = { ...empty, calls: 1, cost: { ...empty.cost, partial: true, unpriced_models: ["a", "i", "j", "k"] } };
    const total = sumUsageDays(days);
    expect(total.cost.partial).toBe(true);
    expect(total.cost.unpriced_models).toEqual(["a", "b", "c", "d", "e", "f", "g", "h"]);
    expect(total.cost.other_unpriced_models).toBeGreaterThan(0);
    expect(days[1]!.amount.cost.unpriced_models).toEqual(["a", "i", "j", "k"]);
  });

});

describe("Hide AI reading", () => {
  const hideDay = (unknown: number, basisPoints: number) => ({
    ...usageDays(range(true))[1]!,
    hide_ai: { amount: { ...empty, calls: 3, new_tokens: 4_500 }, share_basis_points: basisPoints, features: [], unknown_calls: unknown },
  });

  it("sums the counted days and keeps unknown calls a count", () => {
    const sum = sumHideAiDays([hideDay(2, 100), hideDay(1, 100), { ...hideDay(0, 0), hide_ai: null }]);
    expect(sum.amount.new_tokens).toBe(9_000);
    expect(sum.amount.calls).toBe(6);
    expect(sum.unknown).toBe(3);
  });

  it("writes a share as a rounded percent and a vanishing one as below a tenth of a percent", () => {
    expect(hideAiShare(1234, "en")).toBe("12%");
    expect(hideAiShare(250, "en")).toBe("2.5%");
    expect(hideAiShare(5, "en")).toBe("<0.1%");
    expect(hideAiShare(0, "en")).toBe("0%");
  });
});
