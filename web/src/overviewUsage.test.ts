import { describe, expect, it } from "vitest";
import type { UsageAmount, UsageModelDay } from "./generated/hided-ws";
import { HIDE_AI_USAGE, usageModelTotals } from "./overviewUsage";

function row(date: string, agent: string, model: string | null, input: number, hide_ai = false): UsageModelDay {
  const amount: UsageAmount = {
    calls: 1, new_tokens: input + 50,
    tokens: { input, output: 20, cache_write: 30, cache_read: 900, cache_write_1h: 0, reasoning: null },
    cost: { micro_usd: 100, partial: false, waiting: false, stale: false, incomplete: false, unpriced_models: [], other_unpriced_models: 0 },
  };
  return { date, agent, model, hide_ai, amount };
}

// B23-B24: day and agent selection never estimates a missing model from a headline.
describe("recorded usage model comparison", () => {
  const dates = new Set(["2026-10-13", "2026-10-14"]);
  const rows = [
    row("2026-10-13", "pi", "same-model", 100),
    row("2026-10-14", "pi", "same-model", 200),
    row("2026-10-14", "codex", "same-model", 300),
    row("2026-10-14", "pi", "same-model", 400, true),
    row("2026-10-12", "pi", "outside-period", 1_000_000),
  ];

  it("adds only the selected real dates and keeps agent and Hide AI identities distinct", () => {
    const totals = usageModelTotals(rows, dates, "all");
    expect(totals.map((value) => [value.agent, value.hideAi, value.amount.new_tokens])).toEqual([
      ["pi", true, 450], ["pi", false, 400], ["codex", false, 350],
    ]);
    expect(totals[1]!.amount.tokens).toMatchObject({ input: 300, output: 40, cache_write: 60, cache_read: 1800 });
    expect(totals[1]!.amount.cost.micro_usd).toBe(200);
    expect(usageModelTotals(rows, dates, "pi").map((value) => value.amount.new_tokens)).toEqual([400]);
    expect(usageModelTotals(rows, dates, HIDE_AI_USAGE).map((value) => value.amount.new_tokens)).toEqual([450]);
    expect(usageModelTotals(rows, new Set(["2026-10-14"]), "pi")[0]!.amount.new_tokens).toBe(250);
  });

  it("preserves unpriced, waiting, stale and lower-bound facts without creating a model", () => {
    const unpriced = row("2026-10-14", "grok", null, 100);
    unpriced.amount.cost = { ...unpriced.amount.cost, micro_usd: 0, partial: true, waiting: true, stale: true, incomplete: true };
    const totals = usageModelTotals([unpriced], dates, "all");
    expect(totals).toHaveLength(1);
    expect(totals[0]!.model).toBeNull();
    expect(totals[0]!.amount.cost).toMatchObject({ micro_usd: 0, partial: true, waiting: true, stale: true, incomplete: true });
    expect(usageModelTotals([], dates, "all")).toEqual([]);
  });

  it("admits 4096 model records and refuses the first row beyond the wire cap", () => {
    const records = Array.from({ length: 4096 }, () => rows[0]!);
    expect(usageModelTotals(records, dates, "all")[0]!.amount.new_tokens).toBe(614_400);
    expect(() => usageModelTotals([...records, rows[0]!], dates, "all")).toThrow("Usage model capacity exceeded");
  });
});
