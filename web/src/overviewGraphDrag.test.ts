// PRD overview-main-map B22, B23: a ring session's drag carries its team, any
// other session moves alone, and a project or team outline lands where it is dropped.
import { describe, expect, it } from "vitest";
import { dropAt, sessionPlace, teamKey, type Placed } from "./overviewGraphDrag";

// A team: ring session r with child c; s is another ring session.
const placed: Record<string, Placed> = {
  r: { x: 100, y: 0, ring: true, team: "r" },
  c: { x: 200, y: 50, ring: false, team: "r" },
  s: { x: -100, y: 0, ring: true, team: "s" },
};
const frame = { x: -150, y: -40 };
const where = (offsets: ReadonlyMap<string, { x: number; y: number }>, id: string) => { const at = sessionPlace(id, placed[id]!, offsets); return { x: at.x, y: at.y }; };
// React Flow reports a session's position relative to its project box.
const drop = (offsets: ReadonlyMap<string, { x: number; y: number }>, id: string, to: { x: number; y: number }) => dropAt(offsets, { kind: "session", id, placed: placed[id]!, frame }, { x: to.x - frame.x, y: to.y - frame.y });

describe("work map drags", () => {
  it("moves a ring session with its whole team and leaves other teams in place", () => {
    const moved = drop(new Map(), "r", { x: 130, y: 20 });
    expect(where(moved, "r")).toEqual({ x: 130, y: 20 });
    expect(where(moved, "c")).toEqual({ x: 230, y: 70 });
    expect(where(moved, "s")).toEqual({ x: -100, y: 0 });
    expect(moved.has("r")).toBe(false);
  });

  it("moves a child alone, and keeps that move when its team is carried afterwards", () => {
    const child = drop(new Map(), "c", { x: 210, y: 90 });
    expect(where(child, "c")).toEqual({ x: 210, y: 90 });
    expect(where(child, "r")).toEqual({ x: 100, y: 0 });
    const team = drop(child, "r", { x: 110, y: 10 });
    expect(where(team, "c")).toEqual({ x: 220, y: 100 });
  });

  it("lands every event of one gesture where it points, not one step further each time", () => {
    const once = drop(new Map(), "r", { x: 140, y: 30 });
    expect(drop(once, "r", { x: 140, y: 30 })).toEqual(once);
    const outline = dropAt(new Map(), { kind: "team", id: teamKey("r"), base: { x: 80, y: -20 } }, { x: 95, y: -5 });
    expect(dropAt(outline, { kind: "team", id: teamKey("r"), base: { x: 80, y: -20 } }, { x: 95, y: -5 })).toEqual(outline);
    expect(where(outline, "c")).toEqual({ x: 215, y: 65 });
    expect(dropAt(new Map(), { kind: "project", id: "p", base: { x: -150, y: -40 } }, { x: -100, y: -40 }).get("p")).toEqual({ x: 50, y: 0 });
  });

  it("restores every place when the offsets are cleared", () => {
    for (const id of Object.keys(placed)) expect(where(new Map(), id)).toEqual({ x: placed[id]!.x, y: placed[id]!.y });
  });
});
