import { describe, expect, it } from "vitest";
import type { BoardRow } from "./projectBoard";
import type { AgentRow } from "./snapshot";
import { sidebarTreeLines, siblingsShown } from "./sidebarTree";

// The core names the number on the snapshot (`navigator.siblings_shown`); the lines take it as given.
const SIBLINGS = 5;
const row = (pane_id: string, depth: number): BoardRow => ({ agent: { pane_id } as AgentRow, depth });
const drawn = (lines: ReturnType<typeof sidebarTreeLines>) =>
  lines.map((line) => (line.kind === "agent" ? `${line.row.agent.pane_id}@${line.row.depth}` : `+${line.count}@${line.depth}`));

describe("sidebarTreeLines", () => {
  it("keeps five children of a parent and folds the rest with their subtrees behind one line", () => {
    const rows = [row("root", 0), ...["a", "b", "c", "d", "e", "f", "g"].flatMap((id) => (id === "f" ? [row(id, 1), row("f1", 2)] : [row(id, 1)])), row("next", 0)];
    expect(drawn(sidebarTreeLines(rows, new Set(), SIBLINGS))).toEqual(["root@0", "a@1", "b@1", "c@1", "d@1", "e@1", "+2@1", "next@0"]);
    expect(drawn(sidebarTreeLines(rows, new Set(["root"]), SIBLINGS))).toContain("f1@2");
  });

  it("draws the last sibling's elbow short and keeps an ancestor's rail past a later sibling", () => {
    const lines = sidebarTreeLines([row("root", 0), row("a", 1), row("a1", 2), row("b", 1)], new Set(), SIBLINGS);
    const a1 = lines[2]!;
    expect(a1.place).toEqual({ depth: 2, last: true, rails: [true] });
    expect(lines[3]!.place.last).toBe(true);
    expect(lines[1]!.place.last).toBe(false);
  });
});

describe("siblingsShown", () => {
  it("reads the core's number from the snapshot's navigator", () => {
    expect(siblingsShown({ navigator: { siblings_shown: 5 } })).toBe(5);
  });

  it("raises when the snapshot names none, rather than guessing a cap", () => {
    expect(() => siblingsShown({ navigator: {} })).toThrow("no sibling cap");
    expect(() => siblingsShown(null)).toThrow("no sibling cap");
  });
});
