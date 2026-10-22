import type { UsageAmount, UsageDay, UsageRange, NodeUsage } from "./generated/hided-ws";
import type { InterfaceLanguage } from "./i18n/locale";

/** Native provider ids are provenance, never inferred from a model name. */
export function usageProvider(provider: string): { account: string } | { provider: string } | null {
  if (provider === "openai-codex") return { account: "Codex" };
  if (provider === "claude-code") return { account: "Claude Code" };
  const names: Record<string, string> = { anthropic: "Anthropic", openai: "OpenAI", google: "Google", "google-vertex": "Google Vertex AI", "google-gemini-cli": "Google Gemini CLI", xai: "xAI", groq: "Groq", mistral: "Mistral", openrouter: "OpenRouter", "amazon-bedrock": "Amazon Bedrock", azure: "Azure", deepseek: "DeepSeek", cerebras: "Cerebras", together: "Together AI" };
  return names[provider] ? { provider: names[provider] } : null;
}

/** Format a core civil date without moving it into the browser's timezone. */
export function usageDate(date: string, language: InterfaceLanguage, options: Intl.DateTimeFormatOptions = { month: "short", day: "numeric" }): string {
  return new Intl.DateTimeFormat(language, { ...options, timeZone: "UTC" }).format(new Date(`${date}T12:00:00Z`));
}
export function compactTokens(value: number, language: InterfaceLanguage): string {
  const unit = value >= 1e9 ? [1e9, "B"] as const : value >= 1e6 ? [1e6, "M"] as const : value >= 1e3 ? [1e3, "K"] as const : [1, ""] as const;
  return new Intl.NumberFormat(language, { maximumFractionDigits: 1 }).format(value / unit[0]) + unit[1];
}

/** A display period over core-prepared daily amounts; never reprice a call. */
export function usagePeriodDays(range: UsageRange, count: number): UsageDay[] {
  const end = Date.parse(`${range.calendar_end}T00:00:00Z`);
  const start = new Date(end - (count - 1) * 86_400_000).toISOString().slice(0, 10);
  const days = usageDays({ ...range, calendar_start: start });
  const offset = days[0]!.weekday;
  return days.map((day, index) => ({ ...day, week: Math.floor((index + offset) / 7) }));
}

export function sumUsageDays(days: readonly { amount: UsageAmount }[]): UsageAmount {
  const total: UsageAmount = {
    calls: 0, new_tokens: 0,
    tokens: { input: 0, output: 0, cache_read: 0, cache_write: 0, cache_write_1h: 0, reasoning: null },
    cost: { micro_usd: 0, partial: false, waiting: false, stale: false, incomplete: false, unpriced_models: [], other_unpriced_models: 0 },
  };
  const unpriced = new Set<string>();
  for (const { amount } of days) {
    total.calls += amount.calls;
    total.new_tokens += amount.new_tokens;
    for (const key of ["input", "output", "cache_read", "cache_write", "cache_write_1h"] as const) total.tokens[key] += amount.tokens[key];
    if (amount.tokens.reasoning !== null) total.tokens.reasoning = (total.tokens.reasoning ?? 0) + amount.tokens.reasoning;
    total.cost.micro_usd += amount.cost.micro_usd;
    for (const key of ["partial", "waiting", "stale", "incomplete"] as const) total.cost[key] ||= amount.cost[key];
    for (const model of amount.cost.unpriced_models) {
      if (unpriced.has(model)) continue;
      if (unpriced.size < 8) unpriced.add(model);
      else total.cost.other_unpriced_models = 1;
    }
    // Daily truncated names cannot establish a distinct-model count. Keep only
    // their presence, with at most eight visible names and nonnumeric copy.
    if (amount.cost.other_unpriced_models) total.cost.other_unpriced_models = 1;
  }
  total.cost.unpriced_models = [...unpriced];
  return total;
}

export function allTokens(amount: UsageAmount): number {
  return amount.new_tokens + amount.tokens.cache_read;
}
export function dollarAmount(value: UsageAmount, language: InterfaceLanguage): string {
  return new Intl.NumberFormat(language, { style: "currency", currency: "USD", currencyDisplay: "narrowSymbol", maximumFractionDigits: 2 }).format(value.cost.micro_usd / 1_000_000);
}
export function selectedUsage(node: NodeUsage, surface: "popover" | "year", agent: string | null): UsageRange | null {
  if (!agent) return node.history?.[surface].range ?? null;
  const cell = node.agents.find((row) => row.agent === agent)?.consumption;
  return cell?.state === "reading" ? cell.value[surface] : null;
}

/** Fill display cells only; recorded amounts, levels and range totals stay core-owned. */
export function usageDays(range: UsageRange): UsageDay[] {
  const start = Date.parse(`${range.calendar_start}T00:00:00Z`);
  const end = Date.parse(`${range.calendar_end}T00:00:00Z`);
  const count = (end - start) / 86_400_000 + 1;
  if (!Number.isInteger(count) || count < 1 || count > 371) throw new Error("Invalid usage calendar bounds");
  const recorded = new Map(range.days.map((day) => [day.date, day]));
  return Array.from({ length: count }, (_, index) => {
    const civil = new Date(start + index * 86_400_000);
    const date = civil.toISOString().slice(0, 10);
    return recorded.get(date) ?? {
      date, week: Math.floor(index / 7), weekday: civil.getUTCDay(),
      today: index === count - 1, recent_week: index >= count - 7,
      level: 0, pending: range.reading,
      amount: {
        calls: 0, new_tokens: 0,
        tokens: { input: 0, output: 0, cache_read: 0, cache_write: 0, cache_write_1h: 0, reasoning: null },
        cost: { micro_usd: 0, partial: false, waiting: false, stale: false, incomplete: false, unpriced_models: [], other_unpriced_models: 0 },
      },
      agents: [], top_agents: [], other_agents: 0, hide_ai: null,
    };
  });
}

/** Hide AI's days summed the way sessions are; unknown calls stay a count beside the amount. */
export function sumHideAiDays(days: readonly UsageDay[]): { amount: UsageAmount; unknown: number } {
  const counted = days.flatMap((day) => day.hide_ai ? [{ ...day, amount: day.hide_ai.amount }] : []);
  return { amount: sumUsageDays(counted), unknown: days.reduce((sum, day) => sum + (day.hide_ai?.unknown_calls ?? 0), 0) };
}

/** The share is over the day's known new tokens; a share too small to see reads as below a tenth of a percent. */
export function hideAiShare(basisPoints: number, language: InterfaceLanguage): string {
  const percent = new Intl.NumberFormat(language, { style: "percent", maximumFractionDigits: basisPoints < 1000 ? 1 : 0 });
  return basisPoints > 0 && basisPoints < 10 ? `<${percent.format(0.001)}` : percent.format(basisPoints / 10_000);
}
