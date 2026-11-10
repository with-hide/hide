import { describe, expect, it } from "vitest";
import { addCandidate, candidateAgent, candidateGone, candidateItems, candidateProfile, CANDIDATE_LIMIT, CAP_CHOICES, DEFAULT_CONDUCTOR, effortsOf, pickItem, replaceCandidate } from "./conductorSettings";
import type { AgentProfileWire, Device } from "./snapshot";

// Settings › Mission's value (PRD software-factory-conductor D-31, B48, B49).

describe("Conductor settings", () => {
  it("offers the cap the core accepts, 1 to 16, and no more candidates than it keeps", () => {
    expect(CAP_CHOICES[0]).toBe(1);
    expect(CAP_CHOICES.at(-1)).toBe(16);
    let settings = DEFAULT_CONDUCTOR;
    for (let index = 0; index < CANDIDATE_LIMIT + 2; index += 1) settings = addCandidate(settings);
    expect(settings.candidates).toHaveLength(CANDIDATE_LIMIT);
  });

  it("adds an agent not listed yet first, and takes one out or replaces it by its place", () => {
    const one = addCandidate(DEFAULT_CONDUCTOR);
    const two = addCandidate(one);
    expect(two.candidates[1]!.agent).not.toBe(two.candidates[0]!.agent);
    expect(replaceCandidate(two, 0, null).candidates).toEqual([two.candidates[1]]);
    expect(replaceCandidate(two, 1, { agent: "codex", effort: "high", purpose: "tests" }).candidates[1]).toEqual({ agent: "codex", effort: "high", purpose: "tests" });
  });

  // PRD agent-profiles-phase2 B7, B14, B15.
  it("lists each agent's built-in item and then its profiles on this machine, and names the agents as before with none", () => {
    const profile = (id: string, kind: string, name: string) => ({ id, kind, name, account: `/accounts/${name}`, program: null, env_keys: [], version: 1, install: "own", startable: true }) as unknown as AgentProfileWire;
    const device = (profiles: AgentProfileWire[]) => ({ id: "mac", kind: "local", kit: { profiles: { state: "listed", version: 1, profiles, defaults: {} } } }) as unknown as Device;
    const none = candidateItems(device([]));
    expect(none.named).toBe(false);
    expect(none.items.every((item) => item.profile === "default" && item.name === null)).toBe(true);
    const listed = candidateItems(device([profile("p-gate", "claude-code", "opengateway"), profile("p-work", "claude-code", "work")]));
    expect(listed.named).toBe(true);
    expect(listed.items.filter((item) => item.kind === "claude")).toEqual([
      { kind: "claude", profile: "default", name: null },
      { kind: "claude", profile: "p-gate", name: "opengateway" },
      { kind: "claude", profile: "p-work", name: "work" },
    ]);
    // A row kept before profiles is the built-in item; a row of a profile the machine no longer lists is gone.
    expect(candidateProfile({ agent: "claude", purpose: "" })).toBe("default");
    expect(candidateGone({ agent: "claude", purpose: "" }, listed.items)).toBe(false);
    expect(candidateGone({ agent: "claude", profile: "p-work", purpose: "" }, listed.items)).toBe(false);
    expect(candidateGone({ agent: "claude", profile: "p-old", purpose: "" }, listed.items)).toBe(true);
    expect(candidateGone({ agent: "codex", profile: "p-work", purpose: "" }, listed.items)).toBe(true);
  });

  it("picks an item as its agent and profile, clears the other item's model, and sends no profile for the built-in one", () => {
    const row = { agent: "claude", model: "opus", effort: "high", purpose: "changes" };
    expect(pickItem(row, { kind: "claude", profile: "p-gate" })).toEqual({ agent: "claude", profile: "p-gate", purpose: "changes" });
    expect(pickItem({ ...row, profile: "p-gate" }, { kind: "claude", profile: "default" })).toEqual({ agent: "claude", purpose: "changes" });
  });

  it("reads a candidate's agent as the start panel names it, and sends no effort to an agent that takes none", () => {
    expect(candidateAgent("claude-code")).toBe("claude");
    expect(candidateAgent("antigravity")).toBe("agy");
    expect(candidateAgent("nobody")).toBeNull();
    expect(effortsOf("codex")).toContain("minimal");
    expect(effortsOf("grok")).toEqual([]);
  });
});
