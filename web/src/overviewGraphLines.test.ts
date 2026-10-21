// PRD overview-main-map B3-B7: which lines the Work map draws, and the team outline around its members.
import { describe, expect, it } from "vitest";
import { between, communicationLines, hull, lineageLines, projectArea, teamOutline, turnLines } from "./overviewGraphLines";
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

  it("points only roots whose turn it is at the operator, muted outside the chosen team", () => {
    const rows = [{ id: "a", turn: true }, { id: "a1", turn: true }, { id: "b", turn: false }, { id: "c", turn: true }];
    const roots = new Set(["a", "b", "c"]);
    expect(turnLines(rows, roots, idle)).toEqual([{ id: "a", muted: false }, { id: "c", muted: false }]);
    expect(turnLines(rows, roots, { selection: "a1", lineageTeam: "a", teams: new Map([...teams, ["c", "c"]]) })).toEqual([{ id: "a", muted: false }, { id: "c", muted: true }]);
  });

  it("draws delegation lines only inside the chosen team", () => {
    const parents = new Map<string, string | null>([["a", null], ["a1", "a"], ["b", null], ["b1", "b"]]);
    const teamOf = (id: string) => teams.get(id)!;
    expect(lineageLines(["a", "a1", "b", "b1"], parents, teamOf, null)).toEqual([]);
    expect(lineageLines(["a", "a1", "b", "b1"], parents, teamOf, "b")).toEqual([{ parent: "b", child: "b1" }]);
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

describe("project area", () => {
  const name = { width: 100, height: 20 };
  it("covers its sessions in a band around the operator and puts the name outside the band's outer middle", () => {
    // Two sessions to the right of the operator, one above the axis and one below.
    const members = [{ left: 200, top: -100, right: 300, bottom: -20 }, { left: 200, top: 20, right: 300, bottom: 100 }];
    const area = projectArea(members, 10, 30, name);
    // The box holds every session.
    for (const member of members) {
      expect(member.left - area.left).toBeGreaterThanOrEqual(0);
      expect(member.top - area.top).toBeGreaterThanOrEqual(0);
      expect(area.left + area.width - member.right).toBeGreaterThanOrEqual(0);
      expect(area.top + area.height - member.bottom).toBeGreaterThanOrEqual(0);
    }
    // The name is on the axis through the band's middle, beyond the farthest corner.
    const label = { x: area.label.x + area.left, y: area.label.y + area.top };
    expect(label.y).toBeCloseTo(0, 6);
    expect(label.x - name.width / 2).toBeGreaterThan(Math.hypot(300, 100));
    expect(area.path.startsWith("M ")).toBe(true);
    expect(area.path).not.toContain("NaN");
  });

  it("draws a whole ring for a project that goes all the way around", () => {
    const members = Array.from({ length: 12 }, (_, index) => { const angle = (index * Math.PI) / 6; const x = Math.cos(angle) * 200; const y = Math.sin(angle) * 200; return { left: x - 60, top: y - 60, right: x + 60, bottom: y + 60 }; });
    const area = projectArea(members, 10, 30, name);
    // One closed outline, and the box holds every session.
    expect(area.path.match(/M /g)).toHaveLength(1);
    expect(area.path).not.toContain("NaN");
    for (const member of members) expect(member.left >= area.left && member.top >= area.top && member.right <= area.left + area.width && member.bottom <= area.top + area.height).toBe(true);
  });

  it("steps out only where a session reaches further, so the name sits by the nearer ones", () => {
    // Three sessions across the top: the middle one is near, one side reaches far out.
    const members = [{ left: -260, top: -240, right: -180, bottom: -160 }, { left: -40, top: -240, right: 40, bottom: -160 }, { left: 180, top: -500, right: 260, bottom: -420 }];
    const area = projectArea(members, 10, 30, name);
    const label = { x: area.label.x + area.left, y: area.label.y + area.top };
    expect(Math.hypot(label.x, label.y)).toBeLessThan(Math.hypot(220, 460));
    const box = { left: label.x - name.width / 2, right: label.x + name.width / 2, top: label.y - name.height / 2, bottom: label.y + name.height / 2 };
    for (const member of members) expect(box.right <= member.left || member.right <= box.left || box.bottom <= member.top || member.bottom <= box.top).toBe(true);
  });

  it("covers a session opposite the others' middle", () => {
    // Sessions at the top, the bottom and the left: the band must reach round through the left.
    const members = [{ left: -20, top: -220, right: 20, bottom: -180 }, { left: -20, top: 180, right: 20, bottom: 220 }, { left: -220, top: -20, right: -180, bottom: 20 }];
    const area = projectArea(members, 10, 30, name);
    const label = { x: area.label.x + area.left, y: area.label.y + area.top };
    expect(label.x).toBeLessThan(-200);
    expect(label.y).toBeCloseTo(0, 6);
  });
});
