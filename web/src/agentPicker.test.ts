import { beforeEach, describe, expect, it } from "vitest";
import { catalogFor, createCatalogObserver, forgetCatalogs, modelToSend, rememberedSelection, selectKind, takesModel } from "./agentPicker";
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
