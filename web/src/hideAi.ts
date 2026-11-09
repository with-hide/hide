// Hide AI's rules (PRD settings-cleanup D-14 to D-18, B33 to B47): which agents
// the Hide AI tab offers, what each state reads as, and which agent the
// first-run choice makes Hide AI use. The core decides who may be chosen
// (`AiProvider.selectable`); this module only reads its answer, so the screen
// never offers a change the core would refuse.
import { profileCatalog, type ModelCatalog } from "./agentPicker";
import { listedProfiles } from "./agentProfiles";
import { formatDateTime } from "./i18n/format";
import type { InterfaceLanguage } from "./i18n/locale";
import type { AiProvider, BackgroundAi, Device, KitAgent, MachineAgentProfiles } from "./snapshot";

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

/**
 * One item Hide AI runs on (PRD agent-profiles-phase2 D-07): an agent's row on
 * a machine, as one of that machine's profiles of it runs it. A profile's item
 * reads as a row of its own (its sign-in, models, week), so the screen words
 * and offers it as it does an agent; `profile` null is the built-in default,
 * which is the row itself.
 */
export type AiItem = AiProvider & {
  profile: string | null;
  /** The profile's name on its machine; null for the built-in default and for a profile not listed there. */
  profileName: string | null;
  /** The order names a profile its machine no longer lists (B21): shown as a missing profile, never offered. */
  missing: boolean;
};

/** An item as an event names it: the agent, its profile (null: the built-in default) and the machine (null: the core's own). */
export type AiItemRef = { provider: string; profile: string | null; device: string | null };

/** The profile list of the machine `device` names, null for the core's own machine; null while it is not read or the machine keeps none. */
export type ProfileLists = (device: string | null) => MachineAgentProfiles | null;

/** The profile lists of the navigator's machines; the core's own is its row that is not a remote device. */
export function profileListsOf(devices: readonly Device[] | undefined): ProfileLists {
  return (device) => listedProfiles(devices?.find((row) => (device === null ? row.kind !== "remote" : row.id === device)));
}

/** The agent's own list on a row's machine, which a profile's failed read falls back to (B9). */
function rowCatalog(row: AiProvider): ModelCatalog {
  if (row.models.length > 0) return { state: "ready", models: row.models, ...(row.model_labels ? { labels: row.model_labels } : {}) };
  return row.state === "unread" ? { state: "loading" } : { state: "unavailable", reason: row.models_unavailable_reason };
}

function builtIn(row: AiProvider): AiItem {
  return { ...row, profile: null, profileName: null, missing: false };
}

/**
 * The item of `row`'s machine profile `profile`: what its last read at the
 * profile's version now says. `missing` is the core's judgement for an item of
 * the order (`profile_missing`); a profile the list does not hold otherwise is
 * one not read yet.
 */
function profileItem(row: AiProvider, profile: string, lists: ProfileLists, missing = false): AiItem {
  const listed = missing ? undefined : lists(row.device_id ?? null)?.profiles.find((entry) => entry.id === profile && entry.kind === row.agent);
  const quiet = { retry_at_ms: null, weekly: null, past_limit: false, message: null, selectable: false, models: [], model_labels: undefined, models_fixed: false, models_unavailable_reason: null };
  if (!listed) return { ...row, ...quiet, profile, profileName: null, missing, state: missing ? "profile_missing" : "unread" };
  const read = row.profile_models?.find((entry) => entry.profile === profile && entry.version === listed.version);
  const item = { ...row, ...quiet, profile, profileName: listed.name, missing: false };
  if (!read?.state) return { ...item, state: "unread" };
  const catalog = profileCatalog(row, { id: profile, version: listed.version }, rowCatalog(row));
  return {
    ...item,
    state: read.state,
    message: read.message ?? null,
    selectable: read.selectable === true,
    models: catalog.state === "ready" ? [...catalog.models] : [],
    model_labels: catalog.state === "ready" ? catalog.labels : undefined,
    models_fixed: read.models_fixed,
    models_unavailable_reason: read.models_unavailable_reason,
    weekly: read.weekly ?? null,
    past_limit: read.past_limit === true,
  };
}

/** Every item on every machine's row: each agent's built-in default first, then its profiles in the machine's order. */
export function aiItems(ai: BackgroundAi, lists: ProfileLists): AiItem[] {
  return ai.providers.flatMap((row) => [builtIn(row), ...(row.profile_models ?? []).map((entry) => profileItem(row, entry.profile, lists))]);
}

/** The order: Runs on (null before one is chosen), then the fallback list, each with the machine it runs on. */
function orderRefs(ai: BackgroundAi): { runsOn: AiItemRef | null; fallback: AiItemRef[] } {
  return {
    runsOn: ai.provider ? { provider: ai.provider, profile: ai.profile ?? null, device: ai.device ?? null } : null,
    fallback: (ai.fallback ?? []).map((entry) => ({ provider: entry.provider, profile: entry.profile ?? null, device: entry.device ?? null })),
  };
}

/** The item of the order that is agent `provider` as profile `profile`, on the machine the order runs it on; null when the order has none or its machine's row is gone. */
export function orderItem(ai: BackgroundAi | undefined, lists: ProfileLists, provider: string | null | undefined, profile: string | null = null): AiItem | null {
  if (!ai || !provider) return null;
  const { runsOn, fallback } = orderRefs(ai);
  const index = fallback.findIndex((ref) => ref.provider === provider && ref.profile === profile);
  const ref = runsOn?.provider === provider && runsOn.profile === profile ? runsOn : fallback[index];
  const device = ref?.device ?? null;
  const row = ai.providers.find((candidate) => candidate.id === provider && (candidate.device_id ?? null) === device);
  if (!row) return null;
  if (profile === null) return builtIn(row);
  const missing = ref === runsOn ? ai.profile_missing === true : (ai.fallback?.[index]?.profile_missing ?? false);
  return profileItem(row, profile, lists, missing);
}

/** Runs on's item. */
export function runsOnItem(ai: BackgroundAi | undefined, lists: ProfileLists): AiItem | null {
  return orderItem(ai, lists, ai?.provider, ai?.profile ?? null);
}

/** The fallback list's items with each entry's own model, in order; an entry whose machine's row is gone is left out. */
export function fallbackItems(ai: BackgroundAi, lists: ProfileLists): { item: AiItem; model: string }[] {
  return (ai.fallback ?? []).flatMap((entry) => {
    const item = orderItem(ai, lists, entry.provider, entry.profile ?? null);
    return item ? [{ item, model: entry.model }] : [];
  });
}

/** What an event names of an item. */
export function itemRef(item: AiItem): AiItemRef {
  return { provider: item.id, profile: item.profile, device: item.device_id ?? null };
}

/** An item's select value: its agent, `#` and its profile, then `@` and the device it runs on; neither an agent id nor a profile id holds one. */
export function itemKey(item: AiItemRef | AiItem): string {
  const ref = "id" in item ? itemRef(item) : item;
  return `${ref.provider}${ref.profile ? `#${ref.profile}` : ""}${ref.device ? `@${ref.device}` : ""}`;
}

/** The item a select value names. */
export function parseItemKey(value: string): AiItemRef {
  const at = value.indexOf("@");
  const named = at < 0 ? value : value.slice(0, at);
  const hash = named.indexOf("#");
  return {
    provider: hash < 0 ? named : named.slice(0, hash),
    profile: hash < 0 ? null : named.slice(hash + 1),
    device: at < 0 ? null : value.slice(at + 1),
  };
}

/** Whether two items are one item: one agent as one profile, which runs on one machine (D-07). */
function sameItem(a: AiItemRef, b: AiItemRef): boolean {
  return a.provider === b.provider && a.profile === b.profile;
}

/**
 * Whether Hide AI would answer a request now (B7, B66): it is on, and the item
 * it runs on or one listed under it can be asked. A screen that would wait for
 * an answer asks this first, so it never waits on one that cannot come.
 */
export function hideAiCanAnswer(ai: BackgroundAi | undefined): boolean {
  if (!ai || ai.enabled === false) return false;
  const { runsOn, fallback } = orderRefs(ai);
  const answers = (ref: AiItemRef, missing: boolean) => {
    const row = ai.providers.find((candidate) => candidate.id === ref.provider && (candidate.device_id ?? null) === ref.device);
    if (!row || missing) return false;
    return ref.profile === null ? row.selectable : row.profile_models?.some((entry) => entry.profile === ref.profile && entry.selectable === true) === true;
  };
  return (runsOn !== null && answers(runsOn, ai.profile_missing === true)) || fallback.some((ref, index) => answers(ref, ai.fallback?.[index]?.profile_missing === true));
}

/** The feature switch, independent of a provider's temporary availability. */
export function summariesEnabled(ai: BackgroundAi | undefined): boolean {
  return ai?.enabled !== false && ai?.chosen !== false && ai?.agent_summary === true;
}

/** Runs on: every item the core would accept, on each machine, and the chosen one even when it has a problem or its profile is gone, last when no row lists it (B34, B21). */
export function runsOnChoices(ai: BackgroundAi, lists: ProfileLists): AiItem[] {
  const chosen = runsOnItem(ai, lists);
  const isChosen = (item: AiItem) => chosen !== null && itemKey(item) === itemKey(chosen);
  const items = aiItems(ai, lists)
    .map((item) => (isChosen(item) ? (chosen as AiItem) : item))
    .filter((item) => item.selectable || item === chosen);
  return chosen && !items.includes(chosen) ? [...items, chosen] : items;
}

function inOrder(ai: BackgroundAi, item: AiItem): boolean {
  const { runsOn, fallback } = orderRefs(ai);
  const ref = itemRef(item);
  return (runsOn !== null && sameItem(runsOn, ref)) || fallback.some((listed) => sameItem(listed, ref));
}

/** The Add agent menu: items Hide AI can use that are not Runs on and not already listed, on any machine (B38, B16). */
export function addableItems(ai: BackgroundAi, lists: ProfileLists): AiItem[] {
  return aiItems(ai, lists).filter((item) => item.selectable && !inOrder(ai, item));
}

/** The dimmed part of that menu: installed items Hide AI cannot use yet, each with its reason. */
export function unusableItems(ai: BackgroundAi, lists: ProfileLists): AiItem[] {
  return aiItems(ai, lists).filter(
    (item) => !item.selectable && item.installed && item.state !== "unread" && item.state !== "not_installed" && !inOrder(ai, item),
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
