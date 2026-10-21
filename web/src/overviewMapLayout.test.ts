// PRD overview-main-map B1, B2, B17, B18: the operator at the center, roots on
// the first ring by project, descendants outside their parent, no overlap, and
// the same sessions always in the same places.
import { describe, expect, it } from "vitest";
import { placeOverviewMap, type MapInput, type MapSession } from "./overviewMapLayout";

const root = { width: 140, height: 112, orb: 44 };
const child = { width: 120, height: 92, orb: 28 };
const label = { width: 140, height: 24 };
const session = (id: string, projectId: string, parent: string | null = null): MapSession => ({ id, projectId, parent, box: parent ? child : root });
const input = (sessions: MapSession[], projects = [...new Set(sessions.map((row) => row.projectId))]): MapInput => ({ projects: projects.map((id) => ({ id, label })), sessions, operator: 44, gap: 12, projectGap: 40 });

function rectangles(map: ReturnType<typeof placeOverviewMap>, sessions: readonly MapSession[]) {
  const boxes = sessions.map((row) => { const point = map.nodes.get(row.id)!; return { id: row.id, left: point.x - row.box.width / 2, right: point.x + row.box.width / 2, top: point.y - row.box.orb / 2, bottom: point.y - row.box.orb / 2 + row.box.height }; });
  return [{ id: "operator", left: -22, right: 22, top: -22, bottom: 22 }, ...boxes];
}
function expectApart(map: ReturnType<typeof placeOverviewMap>, sessions: readonly MapSession[]) {
  const boxes = rectangles(map, sessions);
  for (const [i, a] of boxes.entries()) for (const b of boxes.slice(i + 1)) {
    expect(a.right <= b.left || b.right <= a.left || a.bottom <= b.top || b.bottom <= a.top, `${a.id} overlaps ${b.id}`).toBe(true);
  }
}
const distance = (point: { x: number; y: number }) => Math.hypot(point.x, point.y);

describe("operator-centered map", () => {
  it("rings the roots around the operator by project and keeps each team outside its root", () => {
    const sessions = [session("a", "hide"), session("a1", "hide", "a"), session("a2", "hide", "a"), session("g", "hide", "a1"), session("b", "hide"), session("c", "billing"), session("d", "site")];
    const map = placeOverviewMap(input(sessions, ["hide", "billing", "site"]));
    expectApart(map, sessions);
    const ring = ["a", "b", "c", "d"].map((id) => distance(map.nodes.get(id)!));
    for (const radius of ring) expect(radius).toBeCloseTo(ring[0]!, 6);
    expect(map.nodes.get("a1")!.level).toBe(2);
    expect(map.nodes.get("g")!.level).toBe(3);
    expect(distance(map.nodes.get("a1")!)).toBeGreaterThan(ring[0]!);
    expect(distance(map.nodes.get("g")!)).toBeGreaterThan(distance(map.nodes.get("a1")!));
    expect(map.nodes.get("g")!.team).toBe("a");
    // A child sits within its root's angle, nearer to it than any other root.
    const a = map.nodes.get("a")!;
    for (const id of ["b", "c", "d"]) expect(Math.abs(map.nodes.get("a1")!.angle - a.angle)).toBeLessThan(Math.abs(map.nodes.get(id)!.angle - a.angle));
    // Each project's name is outside its own sessions and clear of every session.
    for (const id of ["hide", "billing", "site"]) {
      const own = Math.max(...sessions.filter((row) => row.projectId === id).map((row) => distance(map.nodes.get(row.id)!)));
      expect(distance(map.labels.get(id)!)).toBeGreaterThan(own);
      const point = map.labels.get(id)!;
      const name = { left: point.x - label.width / 2, right: point.x + label.width / 2, top: point.y - label.height / 2, bottom: point.y + label.height / 2 };
      for (const box of rectangles(map, sessions)) expect(name.right <= box.left || box.right <= name.left || name.bottom <= box.top || box.bottom <= name.top, `${id} touches ${box.id}`).toBe(true);
    }
  });

  it("places the same sessions in the same places, and a project's sessions together", () => {
    const sessions = [session("a", "p"), session("b", "q"), session("c", "p"), session("c1", "p", "c")];
    const first = placeOverviewMap(input(sessions, ["p", "q"]));
    expect(placeOverviewMap(input(sessions, ["p", "q"]))).toEqual(first);
    const order = ["a", "c", "b"].map((id) => first.nodes.get(id)!.angle);
    expect(order[0]!).toBeLessThan(order[1]!);
    expect(order[1]!).toBeLessThan(order[2]!);
  });

  it.each([1, 3, 12, 40, 150])("keeps %i roots and their teams apart", (roots) => {
    const sessions = Array.from({ length: roots }, (_, index) => session(`r${index}`, `p${index % 5}`)).sort((a, b) => a.projectId.localeCompare(b.projectId));
    for (let index = 0; index < roots; index += 3) sessions.push(session(`r${index}-c`, `p${index % 5}`, `r${index}`), session(`r${index}-g`, `p${index % 5}`, `r${index}-c`));
    const map = placeOverviewMap(input(sessions));
    expect(map.nodes.size).toBe(sessions.length);
    expectApart(map, sessions);
  });

  it("gives one wide team room without wrapping its children around the operator", () => {
    const sessions = [session("root", "p"), ...Array.from({ length: 15 }, (_, index) => session(`c${index}`, "p", "root"))];
    const map = placeOverviewMap(input(sessions));
    expectApart(map, sessions);
    const angles = sessions.slice(1).map((row) => map.nodes.get(row.id)!.angle);
    expect(Math.max(...angles) - Math.min(...angles)).toBeLessThan(Math.PI * 2);
  });

  it("places a lineage cycle once and refuses more than the node limit", () => {
    const cycle = [{ ...session("a", "p"), parent: "b" }, { ...session("b", "p"), parent: "a" }];
    const map = placeOverviewMap(input(cycle));
    expect([...map.nodes.keys()].sort()).toEqual(["a", "b"]);
    expectApart(map, cycle);
    expect(() => placeOverviewMap(input(Array.from({ length: 400 }, (_, index) => session(`s${index}`, "p"))))).toThrow(/over the layout limit/);
    expect(() => placeOverviewMap(input([session("x", "p", "missing")]))).toThrow(/unplaced parent/);
  });

  it("draws only the operator when nothing is placed", () => {
    const map = placeOverviewMap(input([]));
    expect(map.nodes.size).toBe(0);
    expect(map.labels.size).toBe(0);
  });
});
