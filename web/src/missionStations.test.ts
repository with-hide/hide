import { describe, expect, it } from "vitest";
import { initializeInterfaceI18n } from "./i18n/instance";
import { mergeOverridden, nodeStations, stationWords, type StationNode } from "./missionStations";

// The station line of PRD mission-destination D-13-D-17, D-21, D-37, B24-B28: each case is a node the core sends and the line the PRD draws for it.

function node(extra: Partial<StationNode> = {}): StationNode {
  return { kind: "change", status: "working", delivery: "pr", pr: null, pr_state: null, base_pr: null, landing: null, ...extra };
}

const line = (one: StationNode, destination: string | null = "main", open = true) =>
  nodeStations(one, destination, open)?.map((station) => `${station.kind}:${station.state}${station.kind === "land" ? `@${station.place.kind === "stack" ? `#${station.place.pr}` : station.place.branch}${station.person ? "+person" : ""}` : ""}`).join(" ") ?? null;

describe("nodeStations", () => {
  it("draws no line for a node that changes no code (D-17)", () => {
    expect([line(node({ kind: "research", delivery: null })), line(node({ kind: "verify", delivery: null, status: "done" }))]).toEqual([null, null]);
  });

  it("walks a pull request from work to the destination the Conductor merges into", () => {
    expect(line(node())).toBe("work:current pr:todo approve:todo land:todo@main");
    // Once the pull request exists the work is passed and the pull request is where it is now.
    expect(line(node({ pr: 31, pr_state: "pending" }))).toBe("work:done pr:current approve:todo land:todo@main");
    // Ready and done: everything before the branch passed, so the last station is where the result is now.
    expect(line(node({ status: "done", pr: 31, pr_state: "mergeable" }))).toBe("work:done pr:done approve:done land:current@main");
    expect(line(node({ status: "done", pr: 31, pr_state: "merged" }))).toBe("work:done pr:done approve:done land:landed@main");
  });

  it("sends the work back while the pull request's checks failed, it is a draft, or it was closed unmerged", () => {
    expect(line(node({ pr: 31, pr_state: "failed" }))).toBe("work:current pr:failed approve:todo land:todo@main");
    expect(line(node({ pr: 31, pr_state: "draft" }))).toBe("work:current pr:todo approve:todo land:todo@main");
    expect(line(node({ pr: 31, pr_state: "closed" }))).toBe("work:current pr:closed approve:todo land:todo@main");
  });

  it("marks the last station the operator's when they merge: faint orange ahead, yellow once due, landed once merged (D-21, D-35, D-45)", () => {
    const mine = (state: "ahead" | "due" | "merged", extra: Partial<StationNode> = {}) => line(node({ status: "done", pr: 31, pr_state: state === "merged" ? "merged" : "mergeable", merge: { by: "operator", state, prs: [] }, ...extra }));
    expect([mine("ahead"), mine("due"), mine("merged")]).toEqual([
      "work:done pr:done approve:done land:ahead@main+person",
      "work:done pr:done approve:done land:due@main+person",
      "work:done pr:done approve:done land:landed@main+person",
    ]);
    // Never `due` before the core says so, however ready the pull request looks.
    expect(line(node({ pr: 31, pr_state: "pending", merge: { by: "operator", state: "ahead", prs: [31] } }))).toBe("work:done pr:current approve:todo land:ahead@main+person");
    // The Conductor's merge stays a plain station.
    expect(line(node({ status: "done", pr: 31, pr_state: "mergeable", merge: { by: "conductor", state: "ahead", prs: [31] } }))).toBe("work:done pr:done approve:done land:current@main");
  });

  it("ends a stacked pull request on the one it sits on, and one based elsewhere on that branch (D-14)", () => {
    expect(line(node({ pr: 32, pr_state: "pending", base_pr: 31 }))).toBe("work:done pr:current approve:todo land:todo@#31");
    expect(line(node({ pr: 32, pr_state: "pending", pr_base: "release/1.2" }))).toBe("work:done pr:current approve:todo land:todo@release/1.2");
    // A base GitHub answered as the destination is the destination.
    expect(nodeStations(node({ pr: 32, pr_state: "pending", pr_base: "main" }), "main", true)?.at(-1)).toMatchObject({ kind: "land", place: { kind: "destination", branch: "main" } });
  });

  it("goes straight from work to the destination for a direct change", () => {
    expect(line(node({ delivery: "direct" }))).toBe("work:current land:todo@main");
    expect(line(node({ delivery: "direct", status: "done", landing: { state: "left", action: "join" } }))).toBe("work:done land:current@main");
    expect(line(node({ delivery: "direct", status: "done", landing: { state: "landed", proof: { by: "in_main", commit: "abc" } } }))).toBe("work:done land:landed@main");
  });

  it("puts the operator's look before the branch and holds the branch until it passed", () => {
    const gate = (state: "ahead" | "open" | "passed") => line(node({ status: "done", pr: 31, pr_state: "mergeable", gate: { look: "the screens", state, via: null, said: null, ask: null, since_unix_ms: null, long_wait_at_unix_ms: null } }));
    expect([gate("ahead"), gate("open"), gate("passed")]).toEqual([
      "work:done pr:done approve:done gate:ahead land:todo@main",
      "work:done pr:done approve:done gate:due land:todo@main",
      "work:done pr:done approve:done gate:done land:current@main",
    ]);
  });

  it("leaves out a last station whose branch it cannot name: an unknown destination, an outside node's base (B17)", () => {
    expect(line(node({ pr: 31, pr_state: "pending" }), null)).toBe("work:done pr:current approve:todo");
    expect(line(node({ kind: "external", delivery: null, status: "planned", external: { pr: 1232 } }))).toBe("work:done pr:current approve:todo");
    // An outside pull request's state is not read: its node done is its pull request merged.
    expect(line(node({ kind: "external", delivery: null, status: "done", external: { pr: 1232 } }))).toBe("work:done pr:done approve:done");
  });

  it("lights nothing as current once the mission closed", () => {
    expect(line(node({ pr: 31, pr_state: "pending" }), "main", false)).toBe("work:done pr:todo approve:todo land:todo@main");
  });
});

describe("mergeOverridden", () => {
  it("is the strip's condition: a merge the plan gives the operator against a scope that has the Conductor merge (D-45), never in a scope pr mission (D-35)", () => {
    const mine = { merge: { by: "operator" as const, state: "ahead" as const, prs: [] } };
    expect([mergeOverridden("merge", mine), mergeOverridden("pr", mine), mergeOverridden("merge", { merge: { by: "conductor", state: "ahead", prs: [] } }), mergeOverridden("merge", {})]).toEqual([true, false, false, false]);
  });
});

describe("stationWords", () => {
  it("says every station's word and, past ahead, its state, the newest pull request with how many more, and the operator's merge on the last (B28, D-37)", () => {
    const en = initializeInterfaceI18n("en");
    const stations = nodeStations(node({ status: "done", pr: 32, pr_more: 1, pr_state: "mergeable", merge: { by: "operator", state: "due", prs: [32] } }), "main", true)!;
    expect(stationWords(stations, en.t)).toBe("Work (done) → #32 +1 (done) → Approval (done) → main · You merge it (yours now)");
    const ko = initializeInterfaceI18n("ko");
    expect(stationWords(nodeStations(node({ pr: 1503, pr_state: "pending", base_pr: 1502 }), "main", true)!, ko.t)).toBe("작업 (끝) → #1503 (지금) → 승인 → #1502 위에");
  });
});
