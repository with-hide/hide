import { describe, expect, it } from "vitest";
import { agent, deskSnapshot, type DeskModel, type PracticePane } from "./kit";

const PLACE = { project: "shop-app", label: "shop-app", branch: "main" };
const NOW = Date.UTC(2026, 9, 23, 12);

const lead = agent({ pane_id: "lead", identity_label: "Lead", group: "working", mark: "going", status_code: "working", elapsed: "1m" });
const child = (id: string) => agent({ pane_id: id, identity_label: id, group: "working", mark: "going", status_code: "working", elapsed: "1m", delegated: true, lineage_parent_pane_id: "lead" });
const pane = (id: string, row: PracticePane["agent"]): PracticePane => ({ id, agent: row, title: id });
const tab = (id: string, ...panes: string[]) => ({
  id, name: null, focused: panes[0]!, zoomed: false,
  root: panes.length === 1 ? { type: "pane" as const, pane_id: panes[0]! } : { type: "split" as const, direction: "right" as const, ratio: 0.5, first: { type: "pane" as const, pane_id: panes[0]! }, second: { type: "pane" as const, pane_id: panes[1]! } },
});

describe("a practice desk's tab strip (UI_BEHAVIOR, delegated child tabs)", () => {
  it("leaves off a tab holding only delegated children, and keeps one where the operator's own agent or a shell sits beside a child", () => {
    const model: DeskModel = {
      panes: [pane("lead", lead), pane("cause", child("cause")), pane("repro", child("repro")), pane("sh", null), pane("sh2", null)],
      tabs: [tab("t1", "lead"), tab("t2", "cause"), tab("t3", "repro", "sh"), tab("t4", "sh2")],
      groups: [{ id: null, name: null, tabs: ["t1", "t2", "t3", "t4"] }],
      selectedGroup: null, activeTab: "t2", nextId: 5, operations: [],
    };
    const checkout = deskSnapshot([{ place: PLACE, model }], PLACE.project, NOW).rest.navigator!.workspaces![0]!.checkouts[0]!;
    // A tab of a child and a plain shell holds only delegated agents too, so it stays off; a shell-only tab holds no agent and stays on.
    expect(checkout.tabs.map((row) => [row.id, row.delegated])).toEqual([["t1", false], ["t2", true], ["t3", true], ["t4", false]]);
    expect(checkout.strip.map((row) => row.source_id)).toEqual(["t1", "t4"]);
    // The canvas still shows the child's tab the core made visible.
    expect(checkout.active_tab_id).toBe("t2");
  });
});
