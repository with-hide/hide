import { describe, expect, it } from "vitest";
import { addCandidate, candidateAgent, CANDIDATE_LIMIT, CAP_CHOICES, CHECK_COUNT_CHOICES, DEFAULT_CHECKERS, DEFAULT_CONDUCTOR, effortsOf, replaceCandidate, replaceCheck } from "./conductorSettings";

// Settings › Conductor's value (PRD software-factory-conductor D-31, B48, B49;
// PRD mission-map-git-criteria B36).

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

  it("starts each check as the Conductor's call, offers two to five checkers, and sends both defaults whole when one changes", () => {
    expect(DEFAULT_CONDUCTOR.checkers).toEqual({ review: DEFAULT_CHECKERS.review, verify: DEFAULT_CHECKERS.verify });
    expect(DEFAULT_CHECKERS.review).toEqual({ mode: "conductor", rule: "all", count: 2, agent: null, model: null, effort: null, other_vendor: true });
    expect(CHECK_COUNT_CHOICES).toEqual([2, 3, 4, 5]);
    const many = replaceCheck(DEFAULT_CHECKERS, "verify", { ...DEFAULT_CHECKERS.verify, mode: "many", count: 3, rule: "majority" });
    expect(many).toEqual({ review: DEFAULT_CHECKERS.review, verify: { ...DEFAULT_CHECKERS.verify, mode: "many", count: 3, rule: "majority" } });
  });

  it("drops a checker's model and effort when its agent changes, which the new agent may not take", () => {
    const codex = replaceCheck(DEFAULT_CHECKERS, "review", { ...DEFAULT_CHECKERS.review, agent: "codex" });
    const fixed = replaceCheck(codex, "review", { ...codex.review, model: "gpt-5.5", effort: "high" });
    expect(fixed.review).toMatchObject({ agent: "codex", model: "gpt-5.5", effort: "high" });
    expect(replaceCheck(fixed, "review", { ...fixed.review, agent: "claude" }).review).toMatchObject({ agent: "claude", model: null, effort: null });
    expect(replaceCheck(fixed, "review", { ...fixed.review, agent: null }).review).toMatchObject({ agent: null, model: null, effort: null });
  });
});
