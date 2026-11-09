// The kind and model every start surface chooses (PRD home-device-rail D-18,
// D-20): the remembered choice the core keeps (`ui_state.agent_start`), the
// model list the provider catalog answers, and the one observation the
// catalog needs while any picker shows. Pure, so the rules read and test on
// their own; `components/agent-picker.tsx` draws them.

import type { AgentStartChoice, AiProvider, BackgroundAi } from "./snapshot";
import { AGENT_ADAPTERS, agentAdapter } from "./agentAdapters";

/** The kinds hide starts with a model choice; `terminal` is a tab alone and has no model. */
export type ProviderKind = NonNullable<AgentStartChoice["kind"]>;
export const PROVIDER_KINDS: readonly ProviderKind[] = AGENT_ADAPTERS
  .filter((row) => row.can_start)
  .map((row) => row.herdr_kind as ProviderKind);

export function providerLabel(kind: ProviderKind): string {
  const row = AGENT_ADAPTERS.find((row) => row.can_start && row.herdr_kind === kind);
  if (!row) throw new Error(`Missing start adapter: ${kind}`);
  return row.picker_label;
}

export type AgentKind = ProviderKind | "terminal";

/** One picker's value: the kind and the catalog id of its model, or null for the CLI's own default. */
export type AgentSelection = { kind: AgentKind; model: string | null };

const NOTHING_REMEMBERED: AgentStartChoice = { kind: null, models: {} };

/** The kind a picker opens on: the last one a start named, Claude before any. */
export function rememberedSelection(start: AgentStartChoice | undefined): AgentSelection {
  const remembered = start ?? NOTHING_REMEMBERED;
  const kind = remembered.kind && PROVIDER_KINDS.includes(remembered.kind) ? remembered.kind : PROVIDER_KINDS[0];
  if (!kind) throw new Error("No start adapter declared");
  return { kind, model: remembered.models[kind] ?? null };
}

/** The selection after the kind menu changes: that kind's remembered model, never the other kind's. */
export function selectKind(kind: AgentKind, start: AgentStartChoice | undefined): AgentSelection {
  if (kind === "terminal") return { kind, model: null };
  return { kind, model: (start ?? NOTHING_REMEMBERED).models[kind] ?? null };
}

/**
 * The model list of one agent item: the catalog's `ready` list, the last one
 * seen while the catalog is read again, or the reason there is none (null
 * when the provider answered with an empty list). `loading` is every state
 * where the catalog has not answered yet. A ready list carries the names the
 * agent shows for its models where they differ (`labels`: a start passes the
 * model, a menu shows its label), and `fromDefault` when a profile's own list
 * could not be read and the agent's is offered in its place.
 */
export type ModelCatalog =
  | { state: "ready"; models: readonly string[]; labels?: Readonly<Record<string, string>>; fromDefault?: true }
  | { state: "loading" }
  | { state: "unavailable"; reason: string | null };

type Listed = { models: readonly string[]; labels?: Readonly<Record<string, string>> };

/** The lists seen so far by item, kept for the page's life so a second open shows one at once. */
const lastLists = new Map<string, Listed>();

const ready = (listed: Listed): ModelCatalog => ({
  state: "ready",
  models: listed.models,
  ...(listed.labels && Object.keys(listed.labels).length > 0 ? { labels: listed.labels } : {}),
});

/**
 * The model list of `kind`'s item `profile` (a profile id; omitted or null is
 * the agent's built-in default), as the core read it in that profile's own
 * environment (PRD agent-profiles-phase2 D-09, B9).
 */
export function catalogFor(ai: BackgroundAi | undefined, kind: ProviderKind, profile?: string | null): ModelCatalog {
  // A start names the agent by its engine kind (`agy`); its Hide AI row is joined by the kit's adapter id (`antigravity`).
  const agent = AGENT_ADAPTERS.find((row) => row.can_start && row.herdr_kind === kind)?.id;
  const provider: AiProvider | undefined = ai?.providers.find((row) => row.agent === agent);
  if (profile) {
    const key = `${kind}\u0000${profile}`;
    const own = provider?.profile_models?.find((row) => row.profile === profile);
    if (own && own.models.length > 0) {
      const listed = { models: own.models, labels: own.model_labels };
      lastLists.set(key, listed);
      return ready(listed);
    }
    const kept = lastLists.get(key);
    if (kept) return ready(kept);
    // The profile's list could not be read: the agent's own is offered, the reason is in the log.
    if (own) {
      const agentList = catalogFor(ai, kind);
      return agentList.state === "ready" ? { ...agentList, fromDefault: true } : agentList;
    }
    // Not read yet while its agent's list is: the core reads every startable profile while a screen observes
    // the lists. Otherwise the profile's list waits on, or fails with, its agent's.
    if (provider && provider.models.length > 0) return { state: "loading" };
    return catalogFor(ai, kind);
  }
  if (provider && provider.models.length > 0) {
    const listed = { models: provider.models, labels: provider.model_labels };
    lastLists.set(kind, listed);
    return ready(listed);
  }
  const kept = lastLists.get(kind);
  if (kept) return ready(kept);
  if (!ai || provider?.state === "unread") return { state: "loading" };
  if (!provider) return { state: "unavailable", reason: null };
  return { state: "unavailable", reason: provider.models_unavailable_reason ?? provider.message ?? null };
}

/** The name a menu shows for `model`: the agent's own where it gave one, else the model itself. */
export function modelLabel(catalog: ModelCatalog | null, model: string): string {
  return (catalog?.state === "ready" && catalog.labels?.[model]) || model;
}

/** Forgets the kept lists; tests start from a page that has seen none. */
export function forgetCatalogs() {
  lastLists.clear();
}

/** Whether a start of `kind` takes a model: its adapter declares one (`can_pick_model`); a terminal never does. */
export function takesModel(kind: AgentKind): boolean {
  return agentAdapter(kind)?.can_pick_model === true;
}

/**
 * The model a start sends: the selection's own, listed or not. The catalog is
 * this Mac's reading and the target may be another device, so hide never
 * swaps a model it does not list for another; the CLI decides, and its
 * refusal is what the pane then shows (PRD home-device-rail B32). A kind
 * whose start takes no model sends none, even one remembered for it before
 * the core refused it.
 */
export function modelToSend(selection: AgentSelection): string | null {
  if (!takesModel(selection.kind)) return null;
  return selection.model;
}

/**
 * One observation of the catalog for every picker showing: the first opens
 * it, the last closes it, so two pickers do not switch each other off.
 */
export function createCatalogObserver(send: (observing: boolean) => void) {
  let holders = 0;
  return {
    acquire(): () => void {
      holders += 1;
      if (holders === 1) send(true);
      let released = false;
      return () => {
        if (released) return;
        released = true;
        holders -= 1;
        if (holders === 0) send(false);
      };
    },
    holders: () => holders,
  };
}
