import fs from "node:fs";
import path from "node:path";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { agentLogo, monogram } from "./agentLogos";
import { appliedAgents, hideAiFirstRunKey, selection, tileSwitchable } from "./agentOnboardingRules";
import { AgentLogo } from "./components/agent-logo";
import { catalogs } from "./i18n/catalogs";
import type { KitAgent } from "./snapshot";

const agent = (id: string, availability: KitAgent["availability"]): KitAgent => ({
  id,
  label: id,
  availability,
  enabled: false,
  chosen: false,
  skill: { state: "off", reason: null, location: null },
  hook: null,
  doc_url: "https://example.test",
});

describe("the first-run agent choice", () => {
  const agents = [agent("claude-code", "available"), agent("antigravity", "not_installed"), agent("codex", "available"), agent("opencode", "unsupported_system")];

  it("starts with the agents that are installed on, and never switches on one that is not", () => {
    expect([...selection(agents, new Set())]).toEqual(["claude-code", "codex"]);
    // The operator's flip turns one off; a flip of a tile with no switch changes nothing.
    expect([...selection(agents, new Set(["codex", "antigravity"]))]).toEqual(["claude-code"]);
    expect(agents.map(tileSwitchable)).toEqual([true, false, true, false]);
  });

  it("sends the agents still on in the adapters' order, and nothing for a tile with no switch", () => {
    expect(appliedAgents(agents, new Set())).toEqual(["claude-code", "codex"]);
    expect(appliedAgents(agents, new Set(["claude-code", "codex", "antigravity"]))).toEqual([]);
  });

  it("draws every mark the logo manifest lists, in whatever format it is bundled, and a monogram for the rest", () => {
    type Entry = { id: string };
    const manifest = JSON.parse(fs.readFileSync(path.resolve(__dirname, "assets/agents/manifest.json"), "utf8")) as Record<"logos" | "existing" | "monogram", Entry[]>;
    const drawn = [...manifest.logos, ...manifest.existing].map(({ id }) => id);
    expect(drawn.filter((id) => agentLogo(id) === null)).toEqual([]);
    expect(manifest.monogram.filter(({ id }) => agentLogo(id) !== null)).toEqual([]);
  });

  it("draws the initials of an agent the kit lists but the shell bundles no mark for, never an invented logo", () => {
    const markup = renderToStaticMarkup(createElement(AgentLogo, { agent: "no-such-agent", label: "No Such Agent" }));
    expect(markup).toContain('data-agent-monogram="no-such-agent"');
    expect(markup).toContain(">NS<");
    expect(markup).not.toContain("<img");
  });

  it("makes a monogram from a name without drawing anything", () => {
    expect(monogram("Gemini CLI")).toBe("GC");
    expect(monogram("Kiro")).toBe("KI");
    expect(monogram("Factory Droid")).toBe("FD");
    expect(monogram("")).toBe("?");
  });
});

describe("the first-run Hide AI line in Korean (B46)", () => {
  it("takes 를 after a name that ends in a vowel sound and 을 after Grok, whose reading ends in a consonant", () => {
    const line = (id: string) => catalogs.ko[hideAiFirstRunKey(id)].replace("{{agent}}", id);
    for (const id of ["claude-code", "codex", "opencode", "pi", "antigravity"]) expect(line(id)).toBe(`Hide AI는 ${id}를 씁니다`);
    expect(line("grok")).toBe("Hide AI는 grok을 씁니다");
    // English reads the same for every agent: no particle.
    expect(catalogs.en[hideAiFirstRunKey("grok")]).toBe(catalogs.en[hideAiFirstRunKey("codex")]);
  });
});
