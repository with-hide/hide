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
  const kind = remembered.kind ?? PROVIDER_KINDS[0];
  if (!kind) throw new Error("No start adapter declared");
  return { kind, model: remembered.models[kind] ?? null };
}

/** The selection after the kind menu changes: that kind's remembered model, never the other kind's. */
export function selectKind(kind: AgentKind, start: AgentStartChoice | undefined): AgentSelection {
  if (kind === "terminal") return { kind, model: null };
  return { kind, model: (start ?? NOTHING_REMEMBERED).models[kind] ?? null };
}

/**
 * The model list of one kind: the catalog's `ready` list, the last one seen
 * while the catalog is read again, or the reason there is none (null when the
 * provider answered with an empty list). `loading` is every state where the
 * catalog has not answered yet.
 */
export type ModelCatalog =
  | { state: "ready"; models: readonly string[] }
  | { state: "loading" }
  | { state: "unavailable"; reason: string | null };

/** The lists seen so far, kept for the page's life so a second open shows one at once. */
const lastLists = new Map<ProviderKind, readonly string[]>();

export function catalogFor(ai: BackgroundAi | undefined, kind: ProviderKind): ModelCatalog {
  // A start names the agent as Herdr does (`agy`); its Hide AI row is joined by the kit's adapter id (`antigravity`).
  const agent = AGENT_ADAPTERS.find((row) => row.can_start && row.herdr_kind === kind)?.id;
  const provider: AiProvider | undefined = ai?.providers.find((row) => row.agent === agent);
  if (provider && provider.models.length > 0) {
    lastLists.set(kind, provider.models);
    return { state: "ready", models: provider.models };
  }
  const kept = lastLists.get(kind);
  if (kept) return { state: "ready", models: kept };
  if (!ai || provider?.state === "unread") return { state: "loading" };
  if (!provider) return { state: "unavailable", reason: null };
  return { state: "unavailable", reason: provider.models_unavailable_reason ?? provider.message ?? null };
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
