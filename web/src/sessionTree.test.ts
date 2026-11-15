import { describe, expect, it } from "vitest";
import { SESSION_INDENT_LEVELS, sessionTreeLines } from "./sessionTree";
import type { AgentRow } from "./snapshot";

// docs/UI_BEHAVIOR.md, Sessions: an opened row shows its children at any
// depth, five siblings at a time; the indent stops at three levels and the
// deeper rows hang from their parent's rail.
const agent = (pane_id: string, children: string[] = [], branch_badge: string | null = null) =>
  ({ pane_id, lineage_child_pane_ids: children, state: { tree_rank: 0, branch_badge } }) as unknown as AgentRow;

function tree(rows: AgentRow[]) {
  return new Map(rows.map((row) => [row.pane_id, row]));
}

const drawn = (lines: ReturnType<typeof sessionTreeLines>) =>
  lines.map((line) =>
    line.kind === "agent" ? `${line.row.agent.pane_id}@${line.row.depth}`
    : line.kind === "subagent" ? `~${line.subagent.id}@${line.depth}`
    : line.kind === "subagents-more" ? `~+${line.count}@${line.depth}`
    : `+${line.count}@${line.depth}`);

// The core names the number on the snapshot (`navigator.siblings_shown`); the lines take it as given.
const SIBLINGS = 5;

describe("sessionTreeLines", () => {
  // root > a > b > c > d > e, with a sibling s of c under b.
  const rows = [agent("root", ["a"]), agent("a", ["b"]), agent("b", ["c", "s"]), agent("c", ["d"]), agent("d", ["e"]), agent("e"), agent("s")];
  const byPane = tree(rows);
  const everyOpen = new Set(["root", "a", "b", "c", "d"]);

  it("opens every level the operator opened and stops at a folded row", () => {
    expect(drawn(sessionTreeLines(rows[0]!, byPane, everyOpen, new Set(), SIBLINGS))).toEqual(["root@0", "a@1", "b@2", "c@3", "d@4", "e@5", "s@3"]);
    expect(drawn(sessionTreeLines(rows[0]!, byPane, new Set(["root", "a", "b"]), new Set(), SIBLINGS))).toEqual(["root@0", "a@1", "b@2", "c@3", "s@3"]);
    expect(drawn(sessionTreeLines(rows[0]!, byPane, new Set(["a", "b"]), new Set(), SIBLINGS))).toEqual(["root@0"]);
  });

  it("keeps rows below the indent limit in the limit's column, hanging from the rail above", () => {
    expect(SESSION_INDENT_LEVELS).toBe(3);
    const lines = sessionTreeLines(rows[0]!, byPane, everyOpen, new Set(), SIBLINGS);
    const at = (pane: string) => lines.find((line) => line.kind === "agent" && line.row.agent.pane_id === pane)!.place;
    expect(at("c")).toEqual({ depth: 3, last: false, rails: [false, false] });
    // c has a later sibling (s), so its column's rail runs past d and e.
    expect(at("d")).toEqual({ depth: 3, last: true, rails: [false, false], chain: { pass: true, on: true } });
    expect(at("e")).toEqual({ depth: 3, last: true, rails: [false, false], chain: { pass: true, on: false } });
    expect(at("s").chain).toBeUndefined();
  });

  it("shows five children of a parent and folds the rest behind one line until asked", () => {
    const kids = ["k1", "k2", "k3", "k4", "k5", "k6", "k7"];
    const wide = [agent("root", kids), ...kids.map((id) => agent(id))];
    const map = tree(wide);
    expect(drawn(sessionTreeLines(wide[0]!, map, new Set(["root"]), new Set(), SIBLINGS))).toEqual(["root@0", "k1@1", "k2@1", "k3@1", "k4@1", "k5@1", "+2@1"]);
    expect(drawn(sessionTreeLines(wide[0]!, map, new Set(["root"]), new Set(["root"]), SIBLINGS))).toHaveLength(8);
  });

  // docs/UI_BEHAVIOR.md, Subagents: an opened row lists its own subagents after its children, as many as a parent
  // shows children, and says how many more run; they are no children, so they never count against them.
  it("lists an opened row's subagents after its children, five of them and the rest as one line", () => {
    const running = (count: number, listed: number) => ({
      subagents: count,
      subagent_rows: Array.from({ length: listed }, (_, at) => ({ id: `s${at + 1}`, started_unix_ms: at })),
    });
    const root = { ...agent("root", ["a"]), ...running(7, 6) } as AgentRow;
    const child = { ...agent("a"), ...running(1, 1) } as AgentRow;
    const map = tree([root, child]);
    expect(drawn(sessionTreeLines(root, map, new Set(["root", "a"]), new Set(), SIBLINGS))).toEqual(["root@0", "a@1", "~s1@2", "~s1@1", "~s2@1", "~s3@1", "~s4@1", "~s5@1", "~+2@1"]);
    // A folded row draws none of them, and a row its agent counts more subagents for than it names says so in one line.
    expect(drawn(sessionTreeLines(root, map, new Set(), new Set(), SIBLINGS))).toEqual(["root@0"]);
    const unnamed = { ...agent("root"), ...running(3, 0) } as AgentRow;
    expect(drawn(sessionTreeLines(unnamed, tree([unnamed]), new Set(["root"]), new Set(), SIBLINGS))).toEqual(["root@0", "~+3@1"]);
  });
});
