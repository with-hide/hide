import type { UsageAmount, UsageModelDay } from "./generated/hided-ws";
import { sumUsageDays } from "./usageHistory";

export const HIDE_AI_USAGE = "hide_ai";
export const USAGE_MODEL_ROW_LIMIT = 4096;

export type UsageModelTotal = {
  key: string;
  agent: string;
  model: string | null;
  hideAi: boolean;
  amount: UsageAmount;
};

/** Select only witnessed model records. Aggregate headlines cannot supply a missing model. */
export function usageModelTotals(rows: readonly UsageModelDay[], dates: ReadonlySet<string>, agent: string): UsageModelTotal[] {
  if (rows.length > USAGE_MODEL_ROW_LIMIT) throw new Error("Usage model capacity exceeded");
  const groups = new Map<string, { row: UsageModelDay; amounts: { amount: UsageAmount }[] }>();
  for (const row of rows) {
    if (!dates.has(row.date)) continue;
    if (agent === HIDE_AI_USAGE ? !row.hide_ai : agent !== "all" && (row.hide_ai || row.agent !== agent)) continue;
    const key = JSON.stringify([row.hide_ai, row.agent, row.model]);
    const group = groups.get(key);
    if (group) group.amounts.push(row);
    else groups.set(key, { row, amounts: [row] });
  }
  return [...groups].map(([key, { row, amounts }]) => ({
    key, agent: row.agent, model: row.model, hideAi: row.hide_ai, amount: sumUsageDays(amounts),
  })).sort((a, b) => b.amount.new_tokens - a.amount.new_tokens || a.key.localeCompare(b.key));
}
