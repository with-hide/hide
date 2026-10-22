// PRD overview-main-map B1, B2, B17, B18, B22, B23: a settled force map around
// the operator, teams tied to their parents, the same shape for the same
// sessions, new sessions placed without moving the others, and drags.
import { describe, expect, it } from "vitest";
import { ForceMap, type MapInput, type MapSession } from "./overviewMapLayout";

const session = (id: string, projectId: string, parent: string | null = null): MapSession => ({ id, projectId, parent, radius: parent ? 50 : 80 });
const input = (sessions: MapSession[], projects = [...new Set(sessions.map((row) => row.projectId))]): MapInput => ({ projects, sessions, operator: 40 });
const near = (a: { x: number; y: number }, b: { x: number; y: number }) => Math.hypot(a.x - b.x, a.y - b.y);
const mean = (points: { x: number; y: number }[]) => ({ x: points.reduce((t, p) => t + p.x, 0) / points.length, y: points.reduce((t, p) => t + p.y, 0) / points.length });
const scene = [session("a", "hide"), session("a1", "hide", "a"), session("a2", "hide", "a"), session("g", "hide", "a1"), session("b", "hide"), session("c", "billing"), session("c1", "billing", "c"), session("d", "site")];

describe("force map", () => {
  it("opens the same sessions in the same shape", () => {
    const first = new ForceMap(input(scene)).positions();
    expect(new ForceMap(input([...scene].reverse(), ["hide", "billing", "site"])).positions()).toEqual(first);
    for (const point of first.values()) expect(Number.isFinite(point.x) && Number.isFinite(point.y)).toBe(true);
  });

  it("ties each team to its parent and gathers each project apart from the others", () => {
    const map = new ForceMap(input(scene)).positions();
    const at = (id: string) => map.get(id)!;
    // A child is nearer its parent than any other root; a grandchild is in its root's team.
    for (const [child, parent] of [["a1", "a"], ["a2", "a"], ["g", "a1"], ["c1", "c"]] as const) {
      for (const other of ["a", "b", "c", "d"].filter((id) => id !== parent)) expect(near(at(child), at(parent))).toBeLessThan(near(at(child), at(other)));
    }
    expect(at("g").team).toBe("a");
    expect(at("a").team).toBe("a");
    // Sessions keep their room: no two circles closer than most of their combined room.
    const rows = [...scene];
    for (const [i, a] of rows.entries()) for (const b of rows.slice(i + 1)) expect(near(at(a.id), at(b.id))).toBeGreaterThan((a.radius + b.radius) * 0.8);
    // A project's own sessions sit nearer its middle than another project's middle.
    const middle = (project: string) => mean(scene.filter((row) => row.projectId === project).map((row) => at(row.id)));
    for (const row of scene) for (const other of ["hide", "billing", "site"].filter((project) => project !== row.projectId)) expect(near(at(row.id), middle(row.projectId))).toBeLessThan(near(at(row.id), middle(other)));
  });

  it("places a new session beside its parent without moving any other", () => {
    const before = new ForceMap(input(scene));
    const after = new ForceMap(input([...scene, session("a3", "hide", "a"), session("e", "site")]), before);
    const old = before.positions();
    const now = after.positions();
    for (const [id, point] of old) expect(now.get(id)).toEqual(point);
    expect(near(now.get("a3")!, now.get("a")!)).toBeLessThan(near(now.get("a3")!, now.get("c")!));
    // A session that leaves takes only itself away.
    const fewer = new ForceMap(input(scene.filter((row) => row.id !== "b")), after).positions();
    for (const [id, point] of fewer) expect(point).toEqual(now.get(id));
  });

  it("moves a project's sessions together and nothing else", () => {
    const map = new ForceMap(input(scene));
    const before = map.positions();
    map.moveProject("billing", { x: 120, y: -40 });
    const after = map.positions();
    for (const [id, point] of before) {
      const moved = after.get(id)!;
      if (id.startsWith("c")) expect(moved).toEqual({ ...point, x: point.x + 120, y: point.y - 40 });
      else expect(moved).toEqual(point);
    }
    // Dropped, the project stays put; a later settling elsewhere does not pull it back where it was.
    map.dropProject("billing");
    expect(map.positions()).toEqual(after);
    map.reduceMotion(true);
    map.grab("d");
    map.drop("d");
    for (const id of ["c", "c1"]) expect(near(map.positions().get(id)!, after.get(id)!)).toBeLessThan(near(map.positions().get(id)!, before.get(id)!));
    // A new session arriving later keeps the moved project's ties too.
    const placedAgain = new ForceMap(input([...scene, session("e", "site")]), map);
    placedAgain.reduceMotion(true);
    const moved = placedAgain.positions();
    placedAgain.grab("d");
    placedAgain.drop("d");
    for (const id of ["c", "c1"]) expect(near(placedAgain.positions().get(id)!, moved.get(id)!)).toBeLessThan(near(placedAgain.positions().get(id)!, before.get(id)!));
    placedAgain.stop();
    map.stop();
  });

  it("keeps a dropped session where it was put and settles at once when motion is reduced", () => {
    const map = new ForceMap(input(scene));
    map.reduceMotion(true);
    let redraws = 0;
    map.listen(() => { redraws += 1; });
    const start = map.positions().get("a")!;
    map.grab("a");
    map.drag("a", { x: start.x + 300, y: start.y + 200 });
    expect(map.positions().get("a")).toMatchObject({ x: start.x + 300, y: start.y + 200 });
    map.drop("a");
    expect(redraws).toBe(1);
    const after = map.positions();
    expect(after.get("a")).toMatchObject({ x: start.x + 300, y: start.y + 200 });
    // The team followed its root on settling.
    expect(near(after.get("a1")!, after.get("a")!)).toBeLessThan(near(after.get("a1")!, start));
    map.stop();
  });

  it("places a lineage cycle once and refuses what it cannot place", () => {
    const cycle = [{ ...session("x", "p"), parent: "y" }, { ...session("y", "p"), parent: "x" }];
    const map = new ForceMap(input(cycle)).positions();
    expect([...map.keys()].sort()).toEqual(["x", "y"]);
    expect(map.get("x")!.team).toBe("x");
    expect(() => new ForceMap(input(Array.from({ length: 400 }, (_, index) => session(`s${index}`, "p"))))).toThrow(/over the layout limit/);
    expect(() => new ForceMap(input([session("x", "p", "missing")]))).toThrow(/unplaced parent/);
    expect(() => new ForceMap({ projects: [], sessions: [session("x", "p")], operator: 40 })).toThrow(/unlisted project/);
    expect(new ForceMap(input([])).positions().size).toBe(0);
  });

  // PRD overview-graph-remembered B3, B12: where the operator dropped things opens there again.
  it("opens a remembered project and pinned session where they were dropped, and the rest around them", () => {
    const remembered = { anchors: new Map([["billing", { x: 600, y: 420 }]]), pins: new Map([["a1", { x: -500, y: 300 }], ["gone", { x: 1, y: 1 }]]) };
    const map = new ForceMap({ ...input(scene), remembered });
    expect(map.anchorOf("billing")).toEqual({ x: 600, y: 420 });
    expect(map.positions().get("a1")).toMatchObject({ x: -500, y: 300 });
    expect(map.pinOf("a1")).toEqual({ x: -500, y: 300 });
    expect(map.pinOf("a")).toBeUndefined();
    expect(map.pinsIn("hide")).toEqual([{ id: "a1", at: { x: -500, y: 300 } }]);
    // A record of a session that is gone draws nothing.
    expect(map.positions().has("gone")).toBe(false);
    // Billing's sessions gather around its remembered anchor.
    const billing = mean(["c", "c1"].map((id) => map.positions().get(id)!));
    expect(near(billing, { x: 600, y: 420 })).toBeLessThan(near(billing, map.anchorOf("site")!));
  });

  it("never moves what an open map already placed to a remembered place", () => {
    const open = new ForceMap(input(scene));
    const stale = { anchors: new Map([["billing", { x: 900, y: 900 }]]), pins: new Map([["a1", { x: -900, y: 900 }]]) };
    const again = new ForceMap({ ...input([...scene, session("a3", "hide", "a")]), remembered: stale }, open);
    for (const [id, point] of open.positions()) expect(again.positions().get(id)).toEqual(point);
    expect(again.anchorOf("billing")).toEqual(open.anchorOf("billing"));
  });
});
