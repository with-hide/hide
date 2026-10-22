// PRD overview-main-map B3-B7: which lines the Work map draws, and the team outline around its members.
import { describe, expect, it } from "vitest";
import { between, cloud, communicationLines, exchangeLines, hull, lineageLines, projectAccent, teamOutline, turnLines } from "./overviewGraphLines";
import type { Communication } from "./overviewGraphModel";

const pair = (source: string, target: string, letters = 1): Communication => ({ id: `pair:${source}:${target}`, source, target, letters, direct: letters ? 0 : 1, directions: [{ from: source, to: target, letters, direct: letters ? 0 : 1 }] });
// Two teams: a with child a1, b with child b1.
const teams = new Map([["a", "a"], ["a1", "a"], ["b", "b"], ["b1", "b"]]);
const drawn = () => true;
const idle = { selection: null, lineageTeam: null, pointed: null, teams };

describe("work map lines", () => {
  it("draws a pair across teams always and a pair inside a team only while that team is chosen", () => {
    const pairs = [pair("a1", "b1"), pair("a", "a1")];
    expect(communicationLines(pairs, drawn, idle).map((line) => line.pair.id)).toEqual(["pair:a1:b1"]);
    const chosen = communicationLines(pairs, drawn, { ...idle, selection: "a1", lineageTeam: "a" });
    expect(chosen.map((line) => [line.pair.id, line.active, line.muted])).toEqual([["pair:a1:b1", true, false], ["pair:a:a1", true, false]]);
    // A pair whose end is not drawn has no line.
    expect(communicationLines(pairs, (id) => id !== "b1", idle)).toEqual([]);
  });

  it("mutes a line that neither touches the chosen session nor its team", () => {
    const pairs = [pair("a1", "b1"), pair("b", "c"), pair("c", "d")];
    const lines = communicationLines(pairs, drawn, { ...idle, teams: new Map([...teams, ["c", "c"], ["d", "d"]]), selection: "a", lineageTeam: "a" });
    expect(lines.map((line) => [line.pair.id, line.muted])).toEqual([["pair:a1:b1", false], ["pair:b:c", true], ["pair:c:d", true]]);
    expect(communicationLines(pairs, drawn, { ...idle, teams: new Map([...teams, ["c", "c"], ["d", "d"]]), pointed: "c" }).map((line) => [line.pair.id, line.active])).toEqual([["pair:a1:b1", false], ["pair:b:c", true], ["pair:c:d", true]]);
  });

  it("folds pairs onto their drawn ends with summed counts per direction, badged only where chosen or pointed", () => {
    const pairs: Communication[] = [
      { id: "pair:a1:b", source: "a1", target: "b", letters: 2, direct: 1, directions: [{ from: "a1", to: "b", letters: 2, direct: 0 }, { from: "b", to: "a1", letters: 0, direct: 1 }] },
      { id: "pair:a:b", source: "a", target: "b", letters: 1, direct: 0, directions: [{ from: "a", to: "b", letters: 1, direct: 0 }] },
      { id: "pair:a:a1", source: "a", target: "a1", letters: 4, direct: 0, directions: [{ from: "a", to: "a1", letters: 4, direct: 0 }] },
    ];
    // a1 is folded into its root a, so all three pairs land on a and b, and a with itself draws nothing.
    const drawnAs = (id: string) => (id === "a1" ? "a" : id);
    const state = { ...idle, teams: new Map([["a", "a"], ["a1", "a"], ["b", "b"]]) };
    const [line, ...rest] = exchangeLines(communicationLines(pairs, drawn, state), drawnAs, state);
    expect(rest).toEqual([]);
    expect(line).toMatchObject({ from: "a", to: "b", letters: 3, direct: 1, badged: false, directions: [{ from: "a", to: "b", letters: 3, direct: 0 }, { from: "b", to: "a", letters: 0, direct: 1 }] });
    // Choosing the folded child or pointing at either end badges the line; another session does not.
    expect(exchangeLines(communicationLines(pairs, drawn, state), drawnAs, { selection: "a1", pointed: null })[0]!.badged).toBe(true);
    expect(exchangeLines(communicationLines(pairs, drawn, state), drawnAs, { selection: null, pointed: "b" })[0]!.badged).toBe(true);
    expect(exchangeLines(communicationLines(pairs, drawn, state), drawnAs, { selection: "c", pointed: null })[0]!.badged).toBe(false);
  });

  it("points only roots whose turn it is at the operator, muted outside the chosen team", () => {
    const rows = [{ id: "a", turn: true }, { id: "a1", turn: true }, { id: "b", turn: false }, { id: "c", turn: true }];
    const roots = new Set(["a", "b", "c"]);
    expect(turnLines(rows, roots, idle)).toEqual([{ id: "a", muted: false }, { id: "c", muted: false }]);
    expect(turnLines(rows, roots, { selection: "a1", lineageTeam: "a", teams: new Map([...teams, ["c", "c"]]) })).toEqual([{ id: "a", muted: false }, { id: "c", muted: true }]);
  });

  it("draws every delegation line, brightening the chosen team's", () => {
    const parents = new Map<string, string | null>([["a", null], ["a1", "a"], ["b", null], ["b1", "b"]]);
    const teamOf = (id: string) => teams.get(id)!;
    expect(lineageLines(["a", "a1", "b", "b1"], parents, teamOf, null)).toEqual([{ parent: "a", child: "a1", emphasized: false }, { parent: "b", child: "b1", emphasized: false }]);
    expect(lineageLines(["a", "a1", "b", "b1"], parents, teamOf, "b").map((line) => line.emphasized)).toEqual([false, true]);
  });
});

describe("work map geometry", () => {
  it("trims a line to the orbs' edges and draws none between touching orbs", () => {
    expect(between({ x: 0, y: 0, r: 10 }, { x: 100, y: 0, r: 20 }, 2)).toEqual({ x1: 12, y1: 0, x2: 78, y2: 0 });
    expect(between({ x: 0, y: 0, r: 10 }, { x: 25, y: 0, r: 10 }, 4)).toBeNull();
  });

  it("keeps hull points that turn one way and drops the inside ones", () => {
    const shape = hull([{ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 10, y: 10 }, { x: 0, y: 10 }, { x: 5, y: 5 }]);
    expect(shape).toHaveLength(4);
    expect(shape).not.toContainEqual({ x: 5, y: 5 });
  });

  it("outlines every member's rectangle with the pad outside it", () => {
    const shape = teamOutline([{ left: 0, top: 0, right: 100, bottom: 50 }, { left: 200, top: 100, right: 260, bottom: 160 }], 6);
    expect([shape.left, shape.top, shape.width, shape.height]).toEqual([-6, -6, 272, 172]);
    // Every vertex the path reaches lies within the outline's box, and the path closes.
    const points = [...shape.path.matchAll(/(-?[\d.]+),(-?[\d.]+)/g)].map((match) => ({ x: Number(match[1]), y: Number(match[2]) }));
    for (const point of points) {
      expect(point.x).toBeGreaterThanOrEqual(-1e-9);
      expect(point.y).toBeGreaterThanOrEqual(-1e-9);
      expect(point.x).toBeLessThanOrEqual(shape.width + 1e-9);
      expect(point.y).toBeLessThanOrEqual(shape.height + 1e-9);
    }
    expect(shape.path.endsWith("Z")).toBe(true);
  });
});

describe("project marks", () => {
  it("gives each project one accent choice that never changes", () => {
    expect(projectAccent("hide")).toBe(projectAccent("hide"));
    expect(["sky", "violet", "lime", "amber"]).toContain(projectAccent("hide"));
    const colors = new Set(Array.from({ length: 40 }, (_, index) => projectAccent(`project-${index}`)));
    expect(colors.size).toBe(4);
  });

  it("centers a glow on its sessions and reaches past the farthest", () => {
    expect(cloud([{ x: 0, y: 0 }, { x: 100, y: 0 }], 60)).toEqual({ x: 50, y: 0, r: 110 });
    expect(cloud([{ x: 10, y: 20 }], 60)).toEqual({ x: 10, y: 20, r: 60 });
  });
});
