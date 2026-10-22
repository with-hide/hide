import { describe, expect, it } from "vitest";
import { addCandidate, candidateAgent, CANDIDATE_LIMIT, CAP_CHOICES, DEFAULT_CONDUCTOR, effortsOf, replaceCandidate } from "./conductorSettings";

// Settings › Conductor's value (PRD software-factory-conductor D-31, B48, B49).

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

  it("reads a candidate's agent as the start panel names it, and sends no effort to an agent that takes none", () => {
    expect(candidateAgent("claude-code")).toBe("claude");
    expect(candidateAgent("antigravity")).toBe("agy");
    expect(candidateAgent("nobody")).toBeNull();
    expect(effortsOf("codex")).toContain("minimal");
    expect(effortsOf("grok")).toEqual([]);
  });
});
