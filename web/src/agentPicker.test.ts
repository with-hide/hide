import { beforeEach, describe, expect, it } from "vitest";
import { catalogFor, createCatalogObserver, forgetCatalogs, modelLabel, modelToSend, rememberedSelection, selectKind, takesModel } from "./agentPicker";
import type { AgentStartChoice, BackgroundAi } from "./snapshot";

const START: AgentStartChoice = { kind: "codex", models: { claude: "opus", codex: "gpt-6-astra" } };

/** A Hide AI row; `agent` is the kit's adapter id, which differs from the id for Claude Code and agy. */
const AGENT_OF: Record<string, string> = { claude: "claude-code", antigravity: "antigravity" };
const provider = (id: string, over: Record<string, unknown> = {}) => ({ id, label: id, agent: AGENT_OF[id] ?? id, state: "ready", headline: "", message: null, model: "", models: [], models_unavailable_reason: null, ...over });
const ai = (...providers: ReturnType<typeof provider>[]) => ({ provider: "claude", chosen: false, providers, unavailable_reason: null }) as unknown as BackgroundAi;

beforeEach(forgetCatalogs);

describe("the remembered selection (B29)", () => {
  it("opens on the last kind and that kind's model", () => {
    expect(rememberedSelection(START)).toEqual({ kind: "codex", model: "gpt-6-astra" });
  });

  it("opens on Claude and the CLI default before anything was chosen", () => {
    expect(rememberedSelection(undefined)).toEqual({ kind: "claude", model: null });
    expect(rememberedSelection({ kind: null, models: {} })).toEqual({ kind: "claude", model: null });
  });

  it("opens a supported kind after Cursor was retired without discarding other remembered models", () => {
    expect(rememberedSelection({ kind: "cursor", models: { claude: "opus", cursor: "old" } })).toEqual({ kind: "claude", model: "opus" });
  });
});

describe("switching the kind (B27)", () => {
  it("takes that kind's remembered model, never the other kind's", () => {
    expect(selectKind("claude", START)).toEqual({ kind: "claude", model: "opus" });
    expect(selectKind("codex", { kind: "claude", models: { claude: "opus" } })).toEqual({ kind: "codex", model: null });
  });

  it("has no model for a terminal", () => {
    expect(selectKind("terminal", START)).toEqual({ kind: "terminal", model: null });
  });
});

describe("an agent item's model list (agent-profiles-phase2 D-09, B8, B9)", () => {
  const read = (profile: string, version: number, over: Record<string, unknown> = {}) => ({ profile, version, models: [] as string[], models_fixed: false, models_unavailable_reason: null as string | null, ...over });
  const gateway = read("p-gw", 1, { models: ["opus", "sonnet"], model_labels: { opus: "glm-4.6" } });
  const failed = (profile: string, version: number) => read(profile, version, { models_unavailable_reason: "models_not_read" });

  it("lists a profile's own models by the names its agent shows, and a start passes the model", () => {
    const state = ai(provider("claude", { models: ["haiku", "sonnet"], profile_models: [gateway, failed("p-side", 1)] }));
    const catalog = catalogFor(state, "claude", { id: "p-gw", version: 1 });
    expect(catalog).toEqual({ state: "ready", models: ["opus", "sonnet"], labels: { opus: "glm-4.6" } });
    expect(modelLabel(catalog, "opus")).toBe("glm-4.6");
    expect(modelLabel(catalog, "sonnet")).toBe("sonnet");
    expect(catalogFor(state, "claude")).toEqual({ state: "ready", models: ["haiku", "sonnet"] });
    expect(catalogFor(state, "claude", null)).toEqual({ state: "ready", models: ["haiku", "sonnet"] });
  });

  it("offers the agent's own list, marked, when a profile's could not be read", () => {
    const state = ai(provider("claude", { models: ["haiku", "sonnet"], model_labels: { haiku: "Haiku" }, profile_models: [failed("p-side", 1)] }));
    expect(catalogFor(state, "claude", { id: "p-side", version: 1 })).toEqual({ state: "ready", models: ["haiku", "sonnet"], labels: { haiku: "Haiku" }, fromDefault: true });
  });

  it("drops a profile's list once a read of it fails, also after it was read before", () => {
    const listed = ai(provider("claude", { models: ["haiku"], profile_models: [gateway] }));
    expect(catalogFor(listed, "claude", { id: "p-gw", version: 1 })).toMatchObject({ models: ["opus", "sonnet"] });
    const expired = ai(provider("claude", { models: ["haiku"], profile_models: [failed("p-gw", 1)] }));
    expect(catalogFor(expired, "claude", { id: "p-gw", version: 1 })).toEqual({ state: "ready", models: ["haiku"], fromDefault: true });
  });

  it("never takes a list read before the profile was edited for its list", () => {
    const before = ai(provider("claude", { models: ["haiku"], profile_models: [gateway] }));
    catalogFor(before, "claude", { id: "p-gw", version: 1 });
    expect(catalogFor(before, "claude", { id: "p-gw", version: 2 })).toEqual({ state: "loading" });
    const after = ai(provider("claude", { models: ["haiku"], profile_models: [failed("p-gw", 2)] }));
    expect(catalogFor(after, "claude", { id: "p-gw", version: 2 })).toEqual({ state: "ready", models: ["haiku"], fromDefault: true });
  });

  it("keeps no list of a removed profile, so one made again with its id starts unread", () => {
    catalogFor(ai(provider("claude", { models: ["haiku"], profile_models: [gateway] })), "claude", { id: "p-gw", version: 1 });
    const removed = ai(provider("claude", { models: ["haiku"] }));
    expect(catalogFor(removed, "claude", { id: "p-gw", version: 1 })).toEqual({ state: "loading" });
    const rereading = ai(provider("claude", { state: "unread" }));
    expect(catalogFor(rereading, "claude")).toEqual({ state: "ready", models: ["haiku"] });
    expect(catalogFor(rereading, "claude", { id: "p-gw", version: 1 })).toEqual({ state: "loading" });
  });

  it("is loading while the profile is unread, and fails with its agent when the agent cannot answer", () => {
    expect(catalogFor(undefined, "claude", { id: "p-new", version: 1 })).toEqual({ state: "loading" });
    expect(catalogFor(ai(provider("claude", { state: "unread" })), "claude", { id: "p-new", version: 1 })).toEqual({ state: "loading" });
    const signedOut = ai(provider("claude", { state: "needs_login", models_unavailable_reason: "Claude is not signed in" }));
    expect(catalogFor(signedOut, "claude", { id: "p-new", version: 1 })).toEqual({ state: "unavailable", reason: "Claude is not signed in" });
  });
});

describe("the model catalog (B28)", () => {
  it("lists the chosen kind's models", () => {
    const state = ai(provider("claude", { models: ["haiku", "sonnet"] }), provider("codex", { models: ["gpt-6-astra"] }));
    expect(catalogFor(state, "claude")).toEqual({ state: "ready", models: ["haiku", "sonnet"] });
    expect(catalogFor(state, "codex")).toEqual({ state: "ready", models: ["gpt-6-astra"] });
  });

  it("lists agy's models though Herdr names it agy and Hide AI antigravity", () => {
    const state = ai(provider("antigravity", { models: ["gemini-3.8-flash-high", "claude-sonnet-4-6"] }));
    expect(catalogFor(state, "agy")).toEqual({ state: "ready", models: ["gemini-3.8-flash-high", "claude-sonnet-4-6"] });
  });

  it("is loading before the catalog answered", () => {
    expect(catalogFor(undefined, "claude")).toEqual({ state: "loading" });
    expect(catalogFor(ai(provider("claude", { state: "unread" })), "claude")).toEqual({ state: "loading" });
  });

  it("gives the reason when the catalog cannot list", () => {
    const state = ai(provider("codex", { state: "needs_login", models_unavailable_reason: "Codex is not signed in" }));
    expect(catalogFor(state, "codex")).toEqual({ state: "unavailable", reason: "Codex is not signed in" });
    // A provider that answered with no models and no reason leaves the wording to the picker, in the interface language.
    expect(catalogFor(ai(provider("claude", { state: "ready" })), "claude")).toEqual({ state: "unavailable", reason: null });
  });

  it("finishes loading when the chosen start has no model catalog provider", () => {
    expect(catalogFor(undefined, "omp")).toEqual({ state: "loading" });
    expect(catalogFor(ai(provider("claude")), "omp")).toEqual({ state: "unavailable", reason: null });
  });

  it("keeps the last list while the catalog is read again", () => {
    catalogFor(ai(provider("claude", { models: ["haiku"] })), "claude");
    expect(catalogFor(ai(provider("claude", { state: "unread" })), "claude")).toEqual({ state: "ready", models: ["haiku"] });
    expect(catalogFor(undefined, "claude")).toEqual({ state: "ready", models: ["haiku"] });
    expect(catalogFor(undefined, "codex")).toEqual({ state: "loading" });
  });
});

describe("the model a start sends", () => {
  it("is the chosen one, listed or not, and never another in its place", () => {
    expect(modelToSend({ kind: "claude", model: "opus" })).toBe("opus");
    expect(modelToSend({ kind: "claude", model: "retired" })).toBe("retired");
    expect(modelToSend({ kind: "claude", model: null })).toBeNull();
  });

  it("is none for a terminal", () => {
    expect(modelToSend({ kind: "terminal", model: null })).toBeNull();
  });

  it("is none for an agent whose start takes no model, even one remembered for it", () => {
    expect(takesModel("opencode")).toBe(false);
    expect(modelToSend({ kind: "opencode", model: "remembered" })).toBeNull();
    for (const kind of ["claude", "codex", "grok", "pi", "omp", "agy"] as const) expect(takesModel(kind)).toBe(true);
  });
});

describe("the catalog observation", () => {
  it("opens with the first picker and closes with the last, so two pickers do not fight", () => {
    const sent: boolean[] = [];
    const observer = createCatalogObserver((observing) => sent.push(observing));
    const first = observer.acquire();
    const second = observer.acquire();
    expect(sent).toEqual([true]);
    first();
    expect(sent).toEqual([true]);
    second();
    expect(sent).toEqual([true, false]);
  });

  it("counts a release once, and opens again for a later picker", () => {
    const sent: boolean[] = [];
    const observer = createCatalogObserver((observing) => sent.push(observing));
    const release = observer.acquire();
    release();
    release();
    expect(observer.holders()).toBe(0);
    observer.acquire();
    expect(sent).toEqual([true, false, true]);
  });
});
