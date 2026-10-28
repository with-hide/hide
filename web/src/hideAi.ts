// Hide AI's rules (PRD settings-cleanup D-14 to D-18, B33 to B47): which agents
// the Hide AI tab offers, what each state reads as, and which agent the
// first-run choice makes Hide AI use. The core decides who may be chosen
// (`AiProvider.selectable`); this module only reads its answer, so the screen
// never offers a change the core would refuse.
import { formatDateTime } from "./i18n/format";
import type { InterfaceLanguage } from "./i18n/locale";
import type { AiProvider, BackgroundAi, KitAgent } from "./snapshot";

/** A select cannot carry an empty value, so "CLI default" (the agent asked with no `--model`) is this. */
export const CLI_DEFAULT = "__cli_default__";

/** The select value for a stored model: an empty one is the CLI's own default. */
export function modelToValue(model: string): string {
  return model === "" ? CLI_DEFAULT : model;
}

/** The model a select value stands for. */
export function valueToModel(value: string): string {
  return value === CLI_DEFAULT ? "" : value;
}

/**
 * The model menu of one agent: what the CLI listed, "CLI default" first where
 * the agent can be asked without a model, and the stored model even when the
 * list does not carry it (a failed refresh must never drop the current choice,
 * B36).
 */
export function modelChoices(provider: AiProvider, current = provider.model): string[] {
  const models = [...provider.models];
  if (current !== "" && !models.includes(current)) models.unshift(current);
  const choices = models.map(modelToValue);
  if (provider.cli_default || current === "") choices.unshift(CLI_DEFAULT);
  return choices;
}

/** A model the screen cannot show is a failed refresh, not an unread list (`not_asked` and `not_observed` are the core's own waiting states). */
export function modelsFailed(provider: AiProvider): boolean {
  const reason = provider.models_unavailable_reason;
  return reason !== null && reason !== "not_asked" && reason !== "not_observed" && !provider.models_fixed;
}

/** The limit a settings file without the field uses (PRD hide-ai-usage-limit D-01). */
export const DEFAULT_USAGE_LIMIT = 90;
/** The limits the menu offers; a stored value outside them is still shown. */
const USAGE_LIMIT_CHOICES = [80, 90, 95];
/** The select value of No limit. */
export const NO_USAGE_LIMIT = "none";

/** The limit the core reports: a percent, `null` for No limit, an older daemon's absence for the default. */
export function usageLimitOf(ai: BackgroundAi | undefined): number | null {
  if (ai?.usage_limit === undefined) return DEFAULT_USAGE_LIMIT;
  return ai.usage_limit;
}

/** The select value for the stored limit. */
export function usageLimitToValue(limit: number | null): string {
  return limit === null ? NO_USAGE_LIMIT : String(limit);
}

/** The usage-limit menu: 80, 90, 95 and No limit, plus the stored percent when it is none of them. */
export function usageLimitChoices(limit: number | null): string[] {
  const percents = limit !== null && !USAGE_LIMIT_CHOICES.includes(limit) ? [...USAGE_LIMIT_CHOICES, limit].sort((a, b) => a - b) : USAGE_LIMIT_CHOICES;
  return [...percents.map(String), NO_USAGE_LIMIT];
}

/** The event value for a select value. */
export function valueToUsageLimit(value: string): number | "none" {
  return value === NO_USAGE_LIMIT ? NO_USAGE_LIMIT : Number(value);
}

/** A percent as the screen writes it: the figure the agent gave, rounded to whole percents. */
export function percentOf(used: number): number {
  return Math.round(used);
}

/** The machine an agent of the order runs on: Runs on's, a fallback entry's, and for any other agent the core's own (null). */
export function machineOf(ai: BackgroundAi | undefined, id: string | null | undefined): string | null {
  if (!ai || !id) return null;
  if (id === ai.provider) return ai.device ?? null;
  return ai.fallback?.find((entry) => entry.provider === id)?.device ?? null;
}

/** The row of agent `id` on the machine the order runs it on (PRD wsl-device B11): one agent runs on one machine. */
export function providerById(ai: BackgroundAi | undefined, id: string | null | undefined): AiProvider | null {
  const machine = machineOf(ai, id);
  return ai?.providers.find((provider) => provider.id === id && (provider.device_id ?? null) === machine) ?? null;
}

/** A row's select value: its agent, and the device it runs on after an `@`; an agent id never holds one. */
export function rowKey(provider: AiProvider): string {
  return provider.device_id ? `${provider.id}@${provider.device_id}` : provider.id;
}

/** The agent and the device (null: the core's own machine) a select value names. */
export function parseRowKey(value: string): { provider: string; device: string | null } {
  const at = value.indexOf("@");
  return at < 0 ? { provider: value, device: null } : { provider: value.slice(0, at), device: value.slice(at + 1) };
}

/**
 * Whether Hide AI would answer a request now (B7, B66): it is on, and the agent
 * it runs on or one listed under it can be asked. A screen that would wait for
 * an answer asks this first, so it never waits on one that cannot come.
 */
export function hideAiCanAnswer(ai: BackgroundAi | undefined): boolean {
  if (!ai || ai.enabled === false) return false;
  if (providerById(ai, ai.provider)?.selectable) return true;
  return (ai.fallback ?? []).some((entry) => providerById(ai, entry.provider)?.selectable === true);
}

/** The feature switch, independent of a provider's temporary availability. */
export function summariesEnabled(ai: BackgroundAi | undefined): boolean {
  return ai?.enabled !== false && ai?.chosen !== false && ai?.agent_summary === true;
}

/** Runs on: every agent the core would accept, on each machine, and the chosen one even when it has a problem (B34). */
export function runsOnChoices(ai: BackgroundAi): AiProvider[] {
  const chosen = providerById(ai, ai.provider);
  return ai.providers.filter((provider) => provider.selectable || provider === chosen);
}

function inFallback(ai: BackgroundAi, id: string): boolean {
  return (ai.fallback ?? []).some((entry) => entry.provider === id);
}

/** The Add agent menu: agents Hide AI can use that are not Runs on and not already listed (B38). */
export function addableProviders(ai: BackgroundAi): AiProvider[] {
  return ai.providers.filter((provider) => provider.selectable && provider.id !== ai.provider && !inFallback(ai, provider.id));
}

/** The dimmed part of that menu: installed agents Hide AI cannot use yet, each with its reason. */
export function unusableProviders(ai: BackgroundAi): AiProvider[] {
  return ai.providers.filter(
    (provider) =>
      !provider.selectable &&
      provider.installed &&
      provider.state !== "unread" &&
      provider.state !== "not_installed" &&
      provider.id !== ai.provider &&
      !inFallback(ai, provider.id),
  );
}

/** One of the agent's states as the screen words it. */
export type ProviderReason = "ready" | "needs_login" | "usage_limited" | "not_installed" | "unavailable" | "read_only" | "unsupported" | "unread";

export function providerReason(provider: AiProvider): ProviderReason {
  switch (provider.state) {
    case "ready":
    case "needs_login":
    case "usage_limited":
    case "not_installed":
    case "unavailable":
    case "unread":
      return provider.state;
    case "unsupported":
      return provider.message === "cannot_guarantee_read_only" ? "read_only" : "unsupported";
    default:
      return "unavailable";
  }
}

/** The name of the first agent that is switched on, for the "sign in to use Hide AI" line (B47); null when none is. */
export function firstEnabledAgent(agents: readonly KitAgent[] | undefined): string | null {
  return agents?.find((agent) => agent.enabled && agent.availability === "available")?.label ?? null;
}

/**
 * No agent can answer: every probe has been read and none is signed in. While
 * any row is still unread nothing is said, so the line does not flash before
 * the first probe returns (B47).
 */
export function nobodySignedIn(ai: BackgroundAi): boolean {
  return ai.providers.every((provider) => provider.state !== "unread") && !ai.providers.some((provider) => provider.selectable);
}

/**
 * The first-run rule (D-18, B45, B46): the first agent in the fixed order that
 * is switched on in the choice and verifiably signed in: an agent whose sign-in
 * the core cannot probe (`login_checked` false) is never picked on its own. The registry already lists the
 * agents in that order.
 */
export function firstRunAgent(ai: BackgroundAi | undefined, on: ReadonlySet<string>): AiProvider | null {
  // `on` is this machine's choice, so only this machine's rows answer it.
  return ai?.providers.find((provider) => !provider.device_id && on.has(provider.agent) && provider.state === "ready" && provider.login_checked !== false) ?? null;
}

/** When a usage limit ends: the time of day today, with the weekday on another day. */
export function resetTime(language: InterfaceLanguage, unixMs: number, now = new Date()): string {
  const sameDay = new Date(unixMs).toDateString() === now.toDateString();
  const options: Intl.DateTimeFormatOptions = sameDay ? { hour: "numeric", minute: "2-digit" } : { weekday: "short", hour: "numeric", minute: "2-digit" };
  return formatDateTime(language, unixMs, options);
}
