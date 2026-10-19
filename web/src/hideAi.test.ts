import { describe, expect, it } from "vitest";
import {
  CLI_DEFAULT,
  addableProviders,
  firstEnabledAgent,
  firstRunAgent,
  hideAiCanAnswer,
  modelChoices,
  modelsFailed,
  nobodySignedIn,
  providerReason,
  resetTime,
  runsOnChoices,
  unusableProviders,
  usageLimitChoices,
  usageLimitOf,
  usageLimitToValue,
  valueToUsageLimit,
} from "./hideAi";
import type { AiProvider, BackgroundAi, KitAgent } from "./snapshot";

function provider(id: string, over: Partial<AiProvider> = {}): AiProvider {
  return {
    id,
    label: id,
    agent: id === "claude" ? "claude-code" : id,
    state: "ready",
    headline: "",
    message: null,
    installed: true,
    selectable: true,
    retry_at_ms: null,
    model: "",
    models: [],
    models_fixed: false,
    cli_default: false,
    models_unavailable_reason: null,
    ...over,
  };
}

function ai(providers: AiProvider[], over: Partial<BackgroundAi> = {}): BackgroundAi {
  return { enabled: true, provider: "claude", chosen: true, providers, fallback: [], refusal: null, unavailable_reason: null, ...over };
}

describe("the model menu (B36)", () => {
  it("keeps the stored model when the CLI's list lacks it", () => {
    const row = provider("claude", { model: "custom", models: ["sonnet", "opus"] });
    expect(modelChoices(row)).toEqual(["custom", "sonnet", "opus"]);
  });

  it("offers CLI default first only for an agent that can be asked without a model", () => {
    expect(modelChoices(provider("grok", { cli_default: true, models: ["m-1"], model: "m-1" }))).toEqual([CLI_DEFAULT, "m-1"]);
    expect(modelChoices(provider("claude", { models: ["sonnet"], model: "sonnet" }))).toEqual(["sonnet"]);
  });

  it("shows an empty stored model as CLI default so the select never has an empty value", () => {
    expect(modelChoices(provider("pi", { cli_default: true, models: ["a"], model: "" }))).toEqual([CLI_DEFAULT, "a"]);
  });

  it("reads a fallback entry's own model, not Runs on's", () => {
    const row = provider("codex", { model: "gpt-a", models: ["gpt-a", "gpt-b"] });
    expect(modelChoices(row, "gpt-b")).toEqual(["gpt-a", "gpt-b"]);
    expect(modelChoices(row, "legacy")).toEqual(["legacy", "gpt-a", "gpt-b"]);
  });

  it("calls a list failed only for a real failure, never for the core's waiting states or a fixed list", () => {
    expect(modelsFailed(provider("claude", { models_unavailable_reason: "claude_models_unreadable:exit=2" }))).toBe(true);
    expect(modelsFailed(provider("claude", { models_unavailable_reason: "not_asked" }))).toBe(false);
    expect(modelsFailed(provider("claude", { models_unavailable_reason: "not_observed" }))).toBe(false);
    expect(modelsFailed(provider("legacy-provider", { models_fixed: true, models_unavailable_reason: "fixed" }))).toBe(false);
    expect(modelsFailed(provider("claude"))).toBe(false);
  });
});

describe("who can be chosen (B34, B35, B38)", () => {
  const rows = [
    provider("claude"),
    provider("codex"),
    provider("antigravity", { state: "needs_login", selectable: false }),
    provider("grok", { installed: false, state: "not_installed", selectable: false }),
    provider("opencode", { state: "unsupported", message: "cannot_guarantee_read_only", selectable: false }),
    provider("pi", { state: "unread", selectable: false }),
  ];

  it("lists only agents the core would accept in Runs on", () => {
    expect(runsOnChoices(ai(rows)).map((row) => row.id)).toEqual(["claude", "codex"]);
  });

  it("keeps the chosen agent in the list with its problem instead of dropping it", () => {
    const broken = rows.map((row) => (row.id === "claude" ? provider("claude", { state: "usage_limited", selectable: false }) : row));
    expect(runsOnChoices(ai(broken)).map((row) => row.id)).toEqual(["claude", "codex"]);
  });

  it("adds only selectable agents that are neither Runs on nor already listed", () => {
    expect(addableProviders(ai(rows)).map((row) => row.id)).toEqual(["codex"]);
    expect(addableProviders(ai(rows, { fallback: [{ provider: "codex", model: "" }] }))).toEqual([]);
  });

  it("dims the installed agents it cannot use, and leaves out the uninstalled and the unread", () => {
    expect(unusableProviders(ai(rows)).map((row) => row.id)).toEqual(["antigravity", "opencode"]);
  });

  it("says nobody is signed in only after every row has been read", () => {
    const none = [provider("claude", { state: "needs_login", selectable: false }), provider("codex", { state: "not_installed", selectable: false })];
    expect(nobodySignedIn(ai(none, { provider: null, chosen: false }))).toBe(true);
    expect(nobodySignedIn(ai([provider("claude", { state: "unread", selectable: false })]))).toBe(false);
    expect(nobodySignedIn(ai(rows))).toBe(false);
  });
});

describe("what a state reads as", () => {
  it("separates the read-only refusal from other unsupported agents", () => {
    expect(providerReason(provider("opencode", { state: "unsupported", message: "cannot_guarantee_read_only" }))).toBe("read_only");
    expect(providerReason(provider("x", { state: "unsupported", message: null }))).toBe("unsupported");
    expect(providerReason(provider("x", { state: "something_new" }))).toBe("unavailable");
  });
});

describe("the first-run rule (D-18, B45, B46)", () => {
  const rows = [
    provider("claude", { state: "needs_login", selectable: false }),
    provider("codex"),
    provider("legacy-provider", { login_checked: false }),
  ];

  it("takes the first agent in the fixed order that is switched on and signed in", () => {
    expect(firstRunAgent(ai(rows), new Set(["claude-code", "codex", "legacy-provider"]))?.id).toBe("codex");
    // A compatible older peer may report a provider whose sign-in cannot be checked.
    expect(firstRunAgent(ai(rows), new Set(["legacy-provider"]))).toBeNull();
  });

  it("names nobody when no chosen agent is signed in, and while the probe has not been read", () => {
    expect(firstRunAgent(ai(rows), new Set(["claude-code"]))).toBeNull();
    expect(firstRunAgent(undefined, new Set(["codex"]))).toBeNull();
    expect(firstRunAgent(ai([provider("codex", { state: "unread", selectable: false })]), new Set(["codex"]))).toBeNull();
  });
});

describe("the sign-in line's agent (B47)", () => {
  const agent = (id: string, label: string, over: Partial<KitAgent>): KitAgent =>
    ({ id, label, availability: "available", enabled: false, skill: { state: "off", reason: null, location: null }, hook: null, doc_url: "", ...over }) as KitAgent;

  it("is the first agent that is switched on, in the adapters' order", () => {
    expect(firstEnabledAgent([agent("claude-code", "Claude Code", {}), agent("codex", "Codex", { enabled: true }), agent("pi", "Pi", { enabled: true })])).toBe("Codex");
  });

  it("is nothing when none is switched on or installed", () => {
    expect(firstEnabledAgent([agent("pi", "Pi", { enabled: true, availability: "not_installed" })])).toBeNull();
    expect(firstEnabledAgent(undefined)).toBeNull();
  });
});

describe("when a usage limit ends", () => {
  it("shows the time of day today and adds the weekday on another day", () => {
    const now = new Date(2026, 9, 6, 9, 0, 0);
    const today = new Date(2026, 9, 6, 15, 10, 0).getTime();
    const later = new Date(2026, 9, 8, 15, 10, 0).getTime();
    expect(resetTime("en", today, now)).toBe(new Intl.DateTimeFormat("en", { hour: "numeric", minute: "2-digit" }).format(today));
    expect(resetTime("en", later, now)).toBe(new Intl.DateTimeFormat("en", { weekday: "short", hour: "numeric", minute: "2-digit" }).format(later));
  });
});

describe("whether Hide AI would answer now (B7, B66)", () => {
  it("answers only when it is on and the agent it runs on, or one listed under it, can be asked", () => {
    const ready = provider("claude");
    const needsLogin = provider("codex", { state: "needs_login", selectable: false });
    expect(hideAiCanAnswer(undefined)).toBe(false);
    expect(hideAiCanAnswer(ai([ready]))).toBe(true);
    // Off: nothing is asked, whatever is signed in.
    expect(hideAiCanAnswer(ai([ready], { enabled: false }))).toBe(false);
    // Nobody chosen, or the chosen one cannot answer and nothing is listed under it.
    expect(hideAiCanAnswer(ai([ready], { provider: null, chosen: false }))).toBe(false);
    expect(hideAiCanAnswer(ai([needsLogin], { provider: "codex" }))).toBe(false);
    // A listed agent that can answer keeps it answering.
    expect(hideAiCanAnswer(ai([needsLogin, ready], { provider: "codex", fallback: [{ provider: "claude", model: "" }] }))).toBe(true);
    expect(hideAiCanAnswer(ai([needsLogin, provider("pi", { state: "unavailable", selectable: false })], { provider: "codex", fallback: [{ provider: "pi", model: "" }] }))).toBe(false);
    // An older daemon that sends no `enabled` is on.
    const older = ai([ready]);
    delete older.enabled;
    expect(hideAiCanAnswer(older)).toBe(true);
  });
});

describe("the usage limit menu (PRD hide-ai-usage-limit D-01)", () => {
  it("offers 80, 90, 95 and No limit, plus a stored percent that is none of them", () => {
    expect(usageLimitChoices(90)).toEqual(["80", "90", "95", "none"]);
    expect(usageLimitChoices(null)).toEqual(["80", "90", "95", "none"]);
    expect(usageLimitChoices(85)).toEqual(["80", "85", "90", "95", "none"]);
  });

  it("reads an older daemon's missing limit as 90 and null as No limit, and round-trips the select value", () => {
    expect(usageLimitOf(undefined)).toBe(90);
    expect(usageLimitOf(ai([]))).toBe(90);
    expect(usageLimitOf(ai([], { usage_limit: 95 }))).toBe(95);
    expect(usageLimitOf(ai([], { usage_limit: null }))).toBeNull();
    expect(usageLimitToValue(null)).toBe("none");
    expect(valueToUsageLimit("none")).toBe("none");
    expect(valueToUsageLimit("80")).toBe(80);
  });
});
